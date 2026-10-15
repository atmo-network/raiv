//! Atomic canonical-metadata FILE seam, not a typed host acceptance manifest.
//! Byte CAS proves file continuity only; payload bindings/admission policy belong above.

use super::{DiskBlockStore, DiskError, NEXT_TEMP};
use crate::store::StoreError;
use raiv::format::{validate_drisl, DecodeLimits};
use std::{
    ffi::CString,
    fmt,
    io::{self, Write},
    os::fd::AsRawFd,
};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum ManifestError {
    BeforePublication(DiskError),
    Conflict,
    UnknownOutcome(DiskError),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforePublication(e) => e.fmt(f),
            Self::Conflict => f.write_str("manifest exact-byte CAS conflict"),
            Self::UnknownOutcome(e) => write!(
                f,
                "manifest publication/durability outcome is unknown; reread under lock: {e}"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Publication {
    Replaced,
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    TemporaryCreated,
    TemporaryWritten,
    FileSynced,
    Renamed,
    DirectorySynced,
}

#[derive(Clone, Copy)]
enum FileKind {
    Checkpoint,
    Executor,
    World,
}

/// Mutable borrow prevents a second in-process writer or unlocked block interleaving.
/// The inherited flock is cooperative; the directory remains operator-owned.
/// The fixed filename is native metadata, not a new CID-addressed structural record.
pub struct ManifestFile<'a> {
    store: &'a mut DiskBlockStore,
    wire: DecodeLimits,
    kind: FileKind,
}

impl<'a> ManifestFile<'a> {
    pub(super) fn new(store: &'a mut DiskBlockStore, wire: DecodeLimits) -> Self {
        Self {
            store,
            wire,
            kind: FileKind::Checkpoint,
        }
    }

    pub(crate) fn executor(store: &'a mut DiskBlockStore, wire: DecodeLimits) -> Self {
        Self {
            store,
            wire,
            kind: FileKind::Executor,
        }
    }

    pub(crate) fn world(store: &'a mut DiskBlockStore, wire: DecodeLimits) -> Self {
        Self {
            store,
            wire,
            kind: FileKind::World,
        }
    }

    fn name(&self) -> CString {
        CString::new(match self.kind {
            FileKind::Checkpoint => "raiv-manifest.cbor",
            FileKind::Executor => "raiv-executor.cbor",
            FileKind::World => "raiv-world.cbor",
        })
        .expect("fixed filename")
    }

    pub fn read(&self) -> Result<Option<Vec<u8>>, DiskError> {
        let mut file = match self
            .store
            .open_file(&self.name(), libc::O_RDONLY | libc::O_NONBLOCK)
        {
            Ok(file) => file,
            Err(DiskError::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let bytes = DiskBlockStore::read_bytes(&mut file, self.wire.max_block_bytes)?;
        self.validate(&bytes)?;
        Ok(Some(bytes))
    }

    /// None means initialize only an absent file. Existing corruption is never absence.
    /// Exact next bytes already present are re-fsynced/acknowledged without replacement.
    pub fn replace(
        &mut self,
        expected: Option<&[u8]>,
        next: &[u8],
    ) -> Result<Publication, ManifestError> {
        #[cfg(test)]
        let fail = {
            let countdown = match self.store.manifest_failure_countdown {
                Some(0) => {
                    self.store.manifest_failure_countdown = None;
                    true
                }
                Some(n) => {
                    self.store.manifest_failure_countdown = Some(n - 1);
                    false
                }
                None => false,
            };
            self.store.fail_manifest_after_publish || countdown
        };
        self.replace_at(expected, next, |stage| {
            #[cfg(test)]
            if fail && stage == Stage::Renamed {
                return Err(io::Error::other("injected post-publication failure").into());
            }
            let _ = stage;
            Ok(())
        })
    }

    fn validate(&self, bytes: &[u8]) -> Result<(), DiskError> {
        validate_drisl(bytes, &self.wire).map_err(|e| DiskError::Data(StoreError::Codec(e)))
    }

    fn replace_at(
        &mut self,
        expected: Option<&[u8]>,
        next: &[u8],
        mut stage: impl FnMut(Stage) -> Result<(), DiskError>,
    ) -> Result<Publication, ManifestError> {
        let before = ManifestError::BeforePublication;
        self.validate(next).map_err(before)?;
        if let Some(expected) = expected {
            self.validate(expected).map_err(before)?;
        }
        let current = self.read().map_err(before)?;
        if current.as_deref() == Some(next) {
            let file = self
                .store
                .open_file(&self.name(), libc::O_RDONLY | libc::O_NONBLOCK)
                .map_err(before)?;
            file.sync_all()
                .map_err(DiskError::from)
                .map_err(ManifestError::UnknownOutcome)?;
            self.store
                .directory
                .sync_all()
                .map_err(DiskError::from)
                .map_err(ManifestError::UnknownOutcome)?;
            return Ok(Publication::AlreadyPresent);
        }
        if current.as_deref() != expected {
            return Err(ManifestError::Conflict);
        }
        let nonce =
            crate::atomic::next_u64(&NEXT_TEMP).map_err(|_| before(DiskError::OriginExhausted))?;
        let temp = CString::new(format!(".raiv-manifest-{}-{nonce}", std::process::id()))
            .expect("numeric internal filename");
        let mut file = self
            .store
            .open_file(&temp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)
            .map_err(before)?;
        let mut publication_attempted = false;
        let result = (|| {
            stage(Stage::TemporaryCreated)?;
            file.write_all(next)?;
            stage(Stage::TemporaryWritten)?;
            file.sync_all()?;
            stage(Stage::FileSynced)?;
            // SAFETY: held directory FD and both NUL-terminated generated names remain
            // live. The prior CAS and cooperative exclusive lock exclude another writer.
            publication_attempted = true;
            if unsafe {
                libc::renameat(
                    self.store.directory.as_raw_fd(),
                    temp.as_ptr(),
                    self.store.directory.as_raw_fd(),
                    self.name().as_ptr(),
                )
            } != 0
            {
                return Err(DiskError::Io(io::Error::last_os_error()));
            }
            stage(Stage::Renamed)?;
            self.store.directory.sync_all()?;
            stage(Stage::DirectorySynced)?;
            Ok(())
        })();
        if let Err(error) = result {
            if publication_attempted {
                return Err(ManifestError::UnknownOutcome(error));
            }
            // SAFETY: only the staging name successfully created by this call, relative
            // to the still-held directory. Cleanup failure leaves an ignored orphan.
            unsafe { libc::unlinkat(self.store.directory.as_raw_fd(), temp.as_ptr(), 0) };
            return Err(before(error));
        }
        Ok(Publication::Replaced)
    }
}
