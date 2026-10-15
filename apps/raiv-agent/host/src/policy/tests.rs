use super::*;
use crate::{
    disk::tests::Directory,
    tests::activation::{admission, output},
};
use raiv::format::DecodeLimits;
use std::{fs, os::unix::fs::MetadataExt};

fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}

#[test]
fn empty_approved_baseline_exact_ack_and_current_pin_preserve_inode_and_ram() {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let tight = Limits {
        max_blocks: 0,
        ..Limits::default()
    };
    assert!(initialize_local(registered, &book, &mut disk, &caps(), &tight).is_err());
    assert!(disk.directory_is_empty().unwrap());
    let pin = initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    let path = directory.0.join("raiv-manifest.cbor");
    let inode = fs::metadata(&path).unwrap().ino();
    let bytes = fs::read(&path).unwrap();
    let again =
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    assert_eq!(again.manifest_digest, pin.manifest_digest);
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    let current = current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    assert_eq!(current.anchor, registered.anchor());
    assert_eq!(
        current.manifest_digest,
        DaslCid::for_block(DaslCodec::Raw, &bytes).digest()
    );
    assert_eq!(registry.len(), 2);
    assert!(book.is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let block_path = directory.0.join(registered.snapshot().root().to_string());
    fs::remove_file(&block_path).unwrap();
    assert!(current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).is_err());
    assert!(!block_path.exists());
    fs::write(&block_path, [0x80]).unwrap();
    assert!(current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).is_err());
    assert_eq!(fs::read(&block_path).unwrap(), [0x80]);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn missing_manifest_nonempty_directory_any_orphan_and_corruption_are_never_genesis_fallback() {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let book = AdmissionBook::new(3).unwrap();
    for name in ["unrelated", ".raiv-block-orphan"] {
        let directory = Directory::new();
        fs::write(directory.0.join(name), [0xff]).unwrap();
        let mut disk =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        assert!(matches!(
            initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()),
            Err(PolicyError::NonEmptyUninitialized)
        ));
        assert_eq!(fs::read(directory.0.join(name)).unwrap(), [0xff]);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    fs::remove_file(directory.0.join("raiv-manifest.cbor")).unwrap();
    assert!(matches!(
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::NonEmptyUninitialized)
    ));
    assert!(matches!(
        current_pin(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::MissingManifest)
    ));
    fs::write(directory.0.join("raiv-manifest.cbor"), [0xff]).unwrap();
    assert!(initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).is_err());
    assert!(current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).is_err());
    assert_eq!(
        fs::read(directory.0.join("raiv-manifest.cbor")).unwrap(),
        [0xff]
    );
}

#[test]
fn uncertainty_exact_retry_conflict_and_pending_admission_cannot_be_reinitialized_or_exported_as_old_pin(
) {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::Manifest(ManifestError::UnknownOutcome(_)))
    ));
    disk.fail_manifest_after_publish = false;
    let pin = initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    let source = disk.manifest(caps().wire).read().unwrap().unwrap();
    disk.manifest(caps().wire)
        .replace(Some(&source), &[0xa0])
        .unwrap();
    assert!(matches!(
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::Conflict)
    ));
    assert!(matches!(
        current_pin(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::Conflict)
    ));
    disk.manifest(caps().wire)
        .replace(Some(&[0xa0]), &source)
        .unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    book.admit_actor_wake(registered, input, grant).unwrap();
    assert!(matches!(
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::AdmissionsPresent)
    ));
    assert!(matches!(
        current_pin(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::Conflict)
    ));
    assert_eq!(
        DaslCid::for_block(
            DaslCodec::Raw,
            &disk.manifest(caps().wire).read().unwrap().unwrap()
        )
        .digest(),
        pin.manifest_digest
    );
}

#[test]
fn initialization_enumeration_and_publication_stay_in_held_fd_after_path_replacement() {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let held = directory.0.with_extension("held");
    fs::rename(&directory.0, &held).unwrap();
    fs::create_dir(&directory.0).unwrap();
    fs::write(directory.0.join("replacement"), [0xff]).unwrap();
    assert!(disk.directory_is_empty().unwrap());
    initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    assert!(held.join("raiv-manifest.cbor").exists());
    assert!(!directory.0.join("raiv-manifest.cbor").exists());
    assert_eq!(fs::read(directory.0.join("replacement")).unwrap(), [0xff]);
    drop(disk);
    fs::remove_dir_all(held).unwrap();
}
