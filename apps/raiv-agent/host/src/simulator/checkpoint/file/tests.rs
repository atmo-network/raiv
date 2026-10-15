use super::*;
use crate::disk::tests::Directory;
use crate::simulator::{tests::entry, ExecutorBook, Status};
use raiv::format::DecodeLimits;
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt},
};

fn caps() -> ExecutorLimits {
    ExecutorLimits {
        wire: DecodeLimits::default(),
        max_actions: 10,
    }
}

fn data(intents: u64) -> ExecutorCheckpoint {
    let mut e = entry(1, 1);
    e.intents = intents;
    let frame = super::super::wire::encode(e.anchor.agent_id, e.program, &[&e], &caps()).unwrap();
    ExecutorCheckpoint::decode(&frame, &caps()).unwrap()
}

fn open(directory: &Directory) -> DiskBlockStore {
    DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap()
}

#[test]
fn typed_exact_cas_reopen_known_retry_keeps_inode_and_independent_main_file_and_does_not_create_capabilities(
) {
    let directory = Directory::new();
    let mut disk = open(&directory);
    disk.manifest(caps().wire).replace(None, &[0xa0]).unwrap();
    let old = data(0);
    let next = data(1);
    let mut file = ExecutorCheckpointFile::new(&mut disk, caps());
    assert!(file.read().unwrap().is_none());
    assert!(matches!(
        file.replace(Some(&old), &next),
        Err(ExecutorFileError::Manifest(ManifestError::Conflict))
    ));
    assert_eq!(file.replace(None, &old).unwrap(), Publication::Replaced);
    assert_eq!(
        file.replace(Some(&old), &next).unwrap(),
        Publication::Replaced
    );
    let path = directory.0.join("raiv-executor.cbor");
    let ino = fs::metadata(&path).unwrap().ino();
    assert_eq!(
        file.replace(Some(&old), &next).unwrap(),
        Publication::AlreadyPresent
    );
    assert_eq!(fs::metadata(&path).unwrap().ino(), ino);
    assert!(matches!(
        file.replace(None, &old),
        Err(ExecutorFileError::Manifest(ManifestError::Conflict))
    ));
    assert_eq!(
        fs::read(directory.0.join("raiv-manifest.cbor")).unwrap(),
        [0xa0]
    );
    drop(disk);
    let mut disk = open(&directory);
    let mut file = ExecutorCheckpointFile::new(&mut disk, caps());
    let loaded = file.read().unwrap().unwrap();
    assert_eq!(loaded.as_bytes(), next.as_bytes());
    assert_eq!(loaded.records().next().unwrap().status, Status::Submitted);
    assert_eq!(
        file.replace(Some(&old), &next).unwrap(),
        Publication::AlreadyPresent
    );
    assert!(ExecutorBook::new(10).unwrap().is_empty());
}

#[test]
fn typed_schema_corruption_tight_caps_symlinks_nonregular_refuse_without_repair_or_other_file_changes(
) {
    let directory = Directory::new();
    let elsewhere = Directory::new();
    let mut disk = open(&directory);
    let target = elsewhere.0.join("sentinel");
    fs::write(&target, [0xa0]).unwrap();
    let path = directory.0.join("raiv-executor.cbor");
    let next = data(1);
    for bad in [&[0xa0][..], &[0xff], &[0x18, 0]] {
        fs::write(&path, bad).unwrap();
        let mut file = ExecutorCheckpointFile::new(&mut disk, caps());
        assert!(file.read().is_err());
        assert!(file.replace(None, &next).is_err());
        assert_eq!(fs::read(&path).unwrap(), bad);
    }
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    let mut file = ExecutorCheckpointFile::new(&mut disk, caps());
    assert!(file.read().is_err());
    assert!(file.replace(None, &next).is_err());
    assert_eq!(fs::read(&target).unwrap(), [0xa0]);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(file.read().is_err());
    assert!(file.replace(None, &next).is_err());
    fs::remove_dir(&path).unwrap();
    ExecutorCheckpointFile::new(&mut disk, caps())
        .replace(None, &next)
        .unwrap();
    for cap in [
        ExecutorLimits {
            max_actions: 0,
            ..caps()
        },
        ExecutorLimits {
            wire: DecodeLimits {
                max_block_bytes: 1,
                ..caps().wire
            },
            ..caps()
        },
    ] {
        let mut file = ExecutorCheckpointFile::new(&mut disk, cap);
        assert!(file.read().is_err());
        assert!(file.replace(Some(&next), &next).is_err());
        assert_eq!(fs::read(&path).unwrap(), next.as_bytes());
    }
    assert!(!directory.0.join("raiv-manifest.cbor").exists());
}

#[test]
fn unknown_publication_is_typed_reread_and_identical_acknowledgement_not_a_new_intent_or_world_transaction(
) {
    let directory = Directory::new();
    let mut disk = open(&directory);
    let old = data(0);
    let next = data(1);
    ExecutorCheckpointFile::new(&mut disk, caps())
        .replace(None, &old)
        .unwrap();
    disk.fail_manifest_after_publish = true;
    let mut file = ExecutorCheckpointFile::new(&mut disk, caps());
    assert!(matches!(
        file.replace(Some(&old), &next),
        Err(ExecutorFileError::Manifest(ManifestError::UnknownOutcome(
            _
        )))
    ));
    let loaded = file.read().unwrap().unwrap();
    assert_eq!(loaded.as_bytes(), next.as_bytes());
    assert_eq!(
        file.replace(Some(&old), &next).unwrap(),
        Publication::AlreadyPresent
    );
    assert_eq!(
        file.read()
            .unwrap()
            .unwrap()
            .records()
            .next()
            .unwrap()
            .submission_intents,
        1
    );
}

#[test]
fn executor_file_keeps_held_fd_confinement_after_directory_path_replacement() {
    let directory = Directory::new();
    let mut disk = open(&directory);
    let moved = directory.0.with_extension("moved");
    fs::rename(&directory.0, &moved).unwrap();
    fs::create_dir(&directory.0).unwrap();
    let old = data(0);
    let mut file = ExecutorCheckpointFile::new(&mut disk, caps());
    file.replace(None, &old).unwrap();
    assert_eq!(file.read().unwrap().unwrap().as_bytes(), old.as_bytes());
    assert_eq!(
        fs::read(moved.join("raiv-executor.cbor")).unwrap(),
        old.as_bytes()
    );
    assert!(fs::read_dir(&directory.0).unwrap().next().is_none());
    fs::remove_dir(&directory.0).unwrap();
    fs::rename(moved, &directory.0).unwrap();
}
