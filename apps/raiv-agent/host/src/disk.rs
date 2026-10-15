//! Linux exact-byte block persistence only; no manifest/HEAD/effect acceptance.

use crate::store::{MemoryBlockStore, StoreError, StoreLimits};
use raiv::{
    format::{validate_drisl, DaslCid, DecodeLimits},
    graph::RestoredAgent,
    limits::Limits,
};
use std::{
    ffi::CString,
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
    path::Path,
    sync::atomic::AtomicU64,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

pub mod manifest;
#[cfg(test)]
pub(crate) mod tests;

#[derive(Debug)]
pub enum DiskError {
    Io(io::Error),
    Data(StoreError),
    NotRegular,
    Length,
    OriginExhausted,
}

impl fmt::Display for DiskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Data(e) => e.fmt(f),
            Self::NotRegular => f.write_str("block path is not a regular file"),
            Self::Length => f.write_str("block file length exceeds bounds or changed during read"),
            Self::OriginExhausted => f.write_str("disk staging identity exhausted"),
        }
    }
}

impl std::error::Error for DiskError {}

impl From<io::Error> for DiskError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Hold an exclusive cooperative Linux directory lock. All file operations are relative
/// to the held descriptor, so renaming/replacing the directory path cannot retarget them.
/// Only open an existing operator-owned directory; no implicit creation/genesis/recovery.
/// flock does not defend against another principal bypassing the cooperative lock.
pub struct DiskBlockStore {
    #[cfg(test)]
    pub(crate) fail_manifest_after_publish: bool,
    #[cfg(test)]
    pub(crate) manifest_failure_countdown: Option<usize>,
    directory: File,
    wire: DecodeLimits,
}

impl DiskBlockStore {
    pub fn open_existing(path: &Path, wire: DecodeLimits) -> Result<Self, DiskError> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        // SAFETY: the owned directory FD remains live throughout this syscall.
        if unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self {
            directory,
            wire,
            #[cfg(test)]
            fail_manifest_after_publish: false,
            #[cfg(test)]
            manifest_failure_countdown: None,
        })
    }

    /// Exclusive metadata-file writer over the same held directory/lock.
    pub fn manifest(&mut self, wire: DecodeLimits) -> manifest::ManifestFile<'_> {
        manifest::ManifestFile::new(self, wire)
    }

    /// Existing identical bytes are verified/fsynced, never blindly overwritten.
    pub fn put(&self, cid: DaslCid, body: &[u8]) -> Result<(), DiskError> {
        Self::validate(cid, body, &self.wire)?;
        let name = Self::name(cid);
        match self.open_file(&name, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(mut file) => {
                let bytes = self.read_file(cid, &mut file, &self.wire)?;
                if bytes != body {
                    return Err(DiskError::Data(StoreError::ConflictingBytes));
                }
                file.sync_all()?;
                self.directory.sync_all()?;
                return Ok(());
            }
            Err(DiskError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let nonce = crate::atomic::next_u64(&NEXT_TEMP).map_err(|_| DiskError::OriginExhausted)?;
        let temp = CString::new(format!(".raiv-block-{}-{nonce}", std::process::id()))
            .expect("numeric generated filename");
        // create_new equivalent: never truncate an orphan or follow a symlink.
        let mut file = self.open_file(&temp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        let placed = (|| {
            file.write_all(body)?;
            file.sync_all()?;
            // SAFETY: both NUL-terminated generated filenames and held directory FD
            // stay live; linkat publishes this inode atomically without replacement.
            if unsafe {
                libc::linkat(
                    self.directory.as_raw_fd(),
                    temp.as_ptr(),
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(DiskError::Io(io::Error::last_os_error()));
            }
            self.directory.sync_all()?;
            Ok(())
        })();
        // Remove only our successfully created staging name, not a colliding existing file.
        // SAFETY: valid held FD and generated C string; unlinkat flags are zero.
        let removed = unsafe { libc::unlinkat(self.directory.as_raw_fd(), temp.as_ptr(), 0) };
        placed?;
        if removed != 0 {
            return Err(io::Error::last_os_error().into());
        }
        self.directory.sync_all()?;
        Ok(())
    }

    /// Explicit I/O port, not a hydrating MaterializedBlocks implementation.
    pub fn load(&self, cid: DaslCid) -> Result<Vec<u8>, DiskError> {
        self.load_capped(cid, &self.wire)
    }

    /// Partial staging on failure is allowed; no manifest/HEAD is changed by placement.
    pub fn place(&self, snapshot: &RestoredAgent) -> Result<(), DiskError> {
        for (cid, body) in snapshot.blocks() {
            self.put(cid, body)?;
        }
        Ok(())
    }

    /// Explicitly materialize a known complete dependency set before invoking Core.
    /// Unknown closure discovery/restart from a root alone remains the manifest owner.
    pub fn readback(
        &self,
        expected: &RestoredAgent,
        caller: &Limits,
    ) -> Result<MemoryBlockStore, DiskError> {
        let mut ram = MemoryBlockStore::new(StoreLimits {
            max_blocks: caller.max_blocks,
            max_bytes: caller.max_materialized_bytes,
            wire: caller.decode_limits(),
        });
        let wire = caller.decode_limits();
        let mut count = 0u64;
        let mut total = 0u64;
        for (_, body) in expected.blocks() {
            count = count.checked_add(1).ok_or(DiskError::Length)?;
            total = total
                .checked_add(body.len() as u64)
                .ok_or(DiskError::Length)?;
        }
        if count > caller.max_blocks || total > caller.max_materialized_bytes {
            return Err(DiskError::Data(StoreError::Capacity));
        }
        for (cid, _) in expected.blocks() {
            let bytes = self.load_capped(cid, &wire)?;
            ram.put(cid, &bytes).map_err(DiskError::Data)?;
        }
        ram.readback(expected, caller).map_err(DiskError::Data)?;
        Ok(ram)
    }

    pub(crate) fn load_capped(
        &self,
        cid: DaslCid,
        caller: &DecodeLimits,
    ) -> Result<Vec<u8>, DiskError> {
        let wire = DecodeLimits {
            max_block_bytes: caller.max_block_bytes.min(self.wire.max_block_bytes),
            max_depth: caller.max_depth.min(self.wire.max_depth),
            max_collection_entries: caller
                .max_collection_entries
                .min(self.wire.max_collection_entries),
        };
        cid.require_state()
            .map_err(|_| DiskError::Data(StoreError::Integrity))?;
        let mut file = self.open_file(&Self::name(cid), libc::O_RDONLY | libc::O_NONBLOCK)?;
        self.read_file(cid, &mut file, &wire)
    }

    fn validate(cid: DaslCid, body: &[u8], wire: &DecodeLimits) -> Result<(), DiskError> {
        cid.require_state()
            .map_err(|_| DiskError::Data(StoreError::Integrity))?;
        validate_drisl(body, wire).map_err(|e| DiskError::Data(StoreError::Codec(e)))?;
        cid.verify(body)
            .map_err(|_| DiskError::Data(StoreError::Integrity))
    }

    fn read_file(
        &self,
        cid: DaslCid,
        file: &mut File,
        wire: &DecodeLimits,
    ) -> Result<Vec<u8>, DiskError> {
        let bytes = Self::read_bytes(file, wire.max_block_bytes)?;
        Self::validate(cid, &bytes, wire)?;
        Ok(bytes)
    }

    fn read_bytes(file: &mut File, max_bytes: u64) -> Result<Vec<u8>, DiskError> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(DiskError::NotRegular);
        }
        let len = metadata.len();
        if len > max_bytes {
            return Err(DiskError::Length);
        }
        let len = usize::try_from(len).map_err(|_| DiskError::Length)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| DiskError::Data(StoreError::Allocation))?;
        bytes.resize(len, 0);
        file.read_exact(&mut bytes)?;
        if file.read(&mut [0])? != 0 {
            return Err(DiskError::Length);
        }
        Ok(bytes)
    }

    /// Bounded held-FD namespace inspection DATA, not approval to seed, repair or dispatch.
    pub fn is_empty_namespace(&self) -> Result<bool, DiskError> {
        self.directory_is_empty()
    }

    pub(crate) fn directory_is_empty(&self) -> Result<bool, DiskError> {
        self.directory_entries_match(|_| false)
    }

    pub(crate) fn directory_is_source_baseline(
        &self,
        source: &RestoredAgent,
    ) -> Result<bool, DiskError> {
        self.directory_entries_match(|name| {
            name == b"raiv-manifest.cbor"
                || core::str::from_utf8(name)
                    .ok()
                    .and_then(|text| text.parse::<DaslCid>().ok())
                    .is_some_and(|cid| source.blocks().any(|(key, _)| key == cid))
        })
    }

    fn directory_entries_match(
        &self,
        mut allowed: impl FnMut(&[u8]) -> bool,
    ) -> Result<bool, DiskError> {
        use std::{ffi::CStr, os::fd::IntoRawFd};
        let fd = self
            .open_file(
                &CString::new(".").expect("fixed filename"),
                libc::O_RDONLY | libc::O_DIRECTORY,
            )?
            .into_raw_fd();
        // SAFETY: fd is a new owned directory descriptor. fdopendir owns it on success.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(error.into());
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                // SAFETY: this guard alone owns the valid fdopendir stream.
                unsafe { libc::closedir(self.0) };
            }
        }
        let stream = Stream(stream);
        loop {
            // SAFETY: valid live stream, thread-local errno. readdir's name remains
            // readable until the next call; inspect it before iterating again.
            let entry = unsafe {
                *libc::__errno_location() = 0;
                libc::readdir(stream.0)
            };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(0) {
                    Ok(true)
                } else {
                    Err(error.into())
                };
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." && !allowed(name) {
                return Ok(false);
            }
        }
    }

    fn name(cid: DaslCid) -> CString {
        CString::new(cid.to_string()).expect("fixed canonical CID alphabet")
    }

    fn open_file(&self, name: &CString, flags: libc::c_int) -> Result<File, DiskError> {
        // SAFETY: owned directory FD and NUL-terminated internal filename are live;
        // O_NOFOLLOW rejects final symlinks, mode applies only to O_CREAT. We take
        // ownership only of a newly returned descriptor, never the directory FD.
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::mode_t,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: openat returned a new valid owned descriptor on this branch.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
