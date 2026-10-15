use super::*;
use crate::disk::tests::Directory;
use std::{
    fs,
    os::unix::{
        fs::{symlink, MetadataExt},
        process::ExitStatusExt,
    },
    process::Command,
};

const OLD: &[u8] = &[0xa1, 0x61, b'x', 0x01];

const NEXT: &[u8] = &[0xa1, 0x61, b'x', 0x02];

fn metadata(disk: &mut DiskBlockStore, slot: u8) -> ManifestFile<'_> {
    match slot {
        0 => disk.manifest(DecodeLimits::default()),
        1 => ManifestFile::executor(disk, DecodeLimits::default()),
        2 => ManifestFile::world(disk, DecodeLimits::default()),
        _ => panic!("unknown test metadata slot"),
    }
}

#[test]
fn bounded_canonical_exact_byte_cas_reopen_idempotence_and_ignored_orphans() {
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let mut manifest = disk.manifest(DecodeLimits::default());
    assert_eq!(manifest.read().unwrap(), None);
    assert!(matches!(
        manifest.replace(Some(OLD), NEXT),
        Err(ManifestError::Conflict)
    ));
    assert!(matches!(
        manifest.replace(None, &[0x18, 0]),
        Err(ManifestError::BeforePublication(_))
    ));
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    assert_eq!(manifest.replace(None, OLD).unwrap(), Publication::Replaced);
    assert!(matches!(
        manifest.replace(None, NEXT),
        Err(ManifestError::Conflict)
    ));
    assert_eq!(
        manifest.replace(Some(OLD), NEXT).unwrap(),
        Publication::Replaced
    );
    let path = directory.0.join("raiv-manifest.cbor");
    let inode = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        manifest.replace(Some(OLD), NEXT).unwrap(),
        Publication::AlreadyPresent
    );
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    fs::write(directory.0.join(".raiv-manifest-interrupted"), [0xff]).unwrap();
    drop(disk);
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    assert_eq!(
        disk.manifest(DecodeLimits::default())
            .read()
            .unwrap()
            .as_deref(),
        Some(NEXT)
    );
    let wire = DecodeLimits {
        max_block_bytes: 3,
        ..DecodeLimits::default()
    };
    let mut manifest = disk.manifest(wire);
    assert!(manifest.read().is_err());
    assert!(matches!(
        manifest.replace(Some(NEXT), OLD),
        Err(ManifestError::BeforePublication(_))
    ));
    assert_eq!(fs::read(path).unwrap(), NEXT);
}

#[test]
fn corrupt_symlink_nonregular_metadata_never_becomes_absence_or_gets_replaced() {
    let directory = Directory::new();
    let elsewhere = Directory::new();
    let target = elsewhere.0.join("sentinel");
    fs::write(&target, OLD).unwrap();
    let path = directory.0.join("raiv-manifest.cbor");
    symlink(&target, &path).unwrap();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let mut manifest = disk.manifest(DecodeLimits::default());
    assert!(manifest.read().is_err());
    assert!(matches!(
        manifest.replace(None, NEXT),
        Err(ManifestError::BeforePublication(_))
    ));
    assert_eq!(fs::read(&target).unwrap(), OLD);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(manifest.read().is_err());
    assert!(manifest.replace(None, NEXT).is_err());
    fs::remove_dir(&path).unwrap();
    for bytes in [&[0xff][..], &[0x18, 0], &[0xa0, 0xa0]] {
        fs::write(&path, bytes).unwrap();
        assert!(manifest.read().is_err());
        assert!(manifest.replace(None, NEXT).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn injected_failures_preserve_old_before_rename_and_report_unknown_after_attempt() {
    for (executor, at) in [0, 1, 2].into_iter().flat_map(|executor| {
        [
            Stage::TemporaryCreated,
            Stage::TemporaryWritten,
            Stage::FileSynced,
            Stage::Renamed,
            Stage::DirectorySynced,
        ]
        .into_iter()
        .map(move |at| (executor, at))
    }) {
        let directory = Directory::new();
        let mut disk =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        let mut manifest = metadata(&mut disk, executor);
        manifest.replace(None, OLD).unwrap();
        let error = manifest
            .replace_at(Some(OLD), NEXT, |stage| {
                if stage == at {
                    Err(io::Error::other("injected filesystem failure").into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let published = matches!(at, Stage::Renamed | Stage::DirectorySynced);
        if published {
            assert!(matches!(error, ManifestError::UnknownOutcome(_)));
        } else {
            assert!(matches!(error, ManifestError::BeforePublication(_)));
        }
        assert_eq!(
            manifest.read().unwrap().as_deref(),
            Some(if published { NEXT } else { OLD })
        );
        assert_eq!(
            manifest.replace(Some(OLD), NEXT).unwrap(),
            if published {
                Publication::AlreadyPresent
            } else {
                Publication::Replaced
            }
        );
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}

#[test]
#[ignore = "owned-child process suite: run separately with test-host.sh, never alongside FD/lock tests"]
fn process_termination_leaves_complete_old_or_new_metadata() {
    // Re-enter this exact test in a child; SIGKILL bypasses cleanup/destructors.
    if let Some(path) = std::env::var_os("RAIV_MANIFEST_CRASH_DIR") {
        let at: usize = std::env::var("RAIV_MANIFEST_CRASH_STAGE")
            .unwrap()
            .parse()
            .unwrap();
        let mut disk =
            DiskBlockStore::open_existing(std::path::Path::new(&path), DecodeLimits::default())
                .unwrap();
        let mut manifest = metadata(
            &mut disk,
            std::env::var("RAIV_MANIFEST_CRASH_SLOT")
                .unwrap()
                .parse()
                .unwrap(),
        );
        let mut step = 0;
        manifest
            .replace_at(Some(OLD), NEXT, |_| {
                if step == at {
                    // SAFETY: kill targets only this isolated test child process.
                    unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
                    panic!("SIGKILL failed");
                }
                step += 1;
                Ok(())
            })
            .unwrap();
        panic!("missing interruption point");
    }
    for (executor, at) in [0, 1, 2]
        .into_iter()
        .flat_map(|executor| (0..5).map(move |at| (executor, at)))
    {
        let directory = Directory::new();
        let mut disk =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        metadata(&mut disk, executor).replace(None, OLD).unwrap();
        drop(disk);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "disk::manifest::tests::process_termination_leaves_complete_old_or_new_metadata",
                "--nocapture",
                "--ignored",
            ])
            .env("RAIV_MANIFEST_CRASH_DIR", &directory.0)
            .env("RAIV_MANIFEST_CRASH_STAGE", at.to_string())
            .env("RAIV_MANIFEST_CRASH_SLOT", executor.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.signal(),
            Some(libc::SIGKILL),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut reopened =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        let mut manifest = metadata(&mut reopened, executor);
        let expected = if at < 3 { OLD } else { NEXT };
        assert_eq!(manifest.read().unwrap().as_deref(), Some(expected));
        assert_eq!(
            manifest.replace(Some(OLD), NEXT).unwrap(),
            if at < 3 {
                Publication::Replaced
            } else {
                Publication::AlreadyPresent
            }
        );
        assert_eq!(manifest.read().unwrap().as_deref(), Some(NEXT));
    }
}

#[test]
#[ignore = "owned-child process suite: run separately with test-host.sh, never alongside FD/lock tests"]
fn process_fork_retains_another_directory_flock_until_child_closes_inherited_descriptor() {
    use std::{
        fs::File,
        os::fd::{AsRawFd, FromRawFd},
    };
    // Child performs only async-signal-safe libc operations between fork and _exit.
    // An EOF fence also releases it during parent unwinding; owned guard always reaps.
    struct Child {
        pid: libc::pid_t,
        writer: Option<File>,
    }
    impl Child {
        fn reap(&mut self) -> io::Result<libc::c_int> {
            self.writer.take();
            let mut status = 0;
            loop {
                let result = unsafe { libc::waitpid(self.pid, &mut status, 0) };
                if result == self.pid {
                    self.pid = 0;
                    return Ok(status);
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
    impl Drop for Child {
        fn drop(&mut self) {
            if self.pid != 0 {
                let _ = self.reap();
            }
        }
    }
    let directory = Directory::new();
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let mut pipe = [-1; 2];
    // SAFETY: writable two-FD buffer; on success these descriptors are newly owned.
    assert_eq!(
        unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    let reader = unsafe { File::from_raw_fd(pipe[0]) };
    let writer = unsafe { File::from_raw_fd(pipe[1]) };
    let read_fd = reader.as_raw_fd();
    let write_fd = writer.as_raw_fd();
    // SAFETY: the child uses no allocator/locks/Rust destructors and exits via _exit.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        unsafe {
            libc::close(write_fd);
            let mut byte = 0u8;
            let read = libc::read(read_fd, (&mut byte as *mut u8).cast(), 1);
            libc::_exit(if read == 1 || read == 0 { 0 } else { 2 });
        }
    }
    assert!(pid > 0, "{}", io::Error::last_os_error());
    drop(reader);
    let mut child = Child {
        pid,
        writer: Some(writer),
    };
    drop(disk);
    let reopened = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default());
    child.writer.as_mut().unwrap().write_all(&[1]).unwrap();
    let status = child.reap().unwrap();
    assert!(libc::WIFEXITED(status));
    assert_eq!(libc::WEXITSTATUS(status), 0);
    assert!(
        matches!(reopened,Err(DiskError::Io(error)) if error.kind()==io::ErrorKind::WouldBlock)
    );
    DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
}
