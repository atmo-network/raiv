use super::*;
use crate::{
    disk::tests::Directory,
    tests::activation::{admission, output},
};
use raiv::format::DecodeLimits;
fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}
fn baseline(disk: &mut DiskBlockStore, registered: &RegisteredAgent, book: &AdmissionBook) {
    disk.place(registered.snapshot()).unwrap();
    let bytes = CheckpointManifest::capture(registered, book, &caps())
        .unwrap()
        .encode(&caps())
        .unwrap();
    disk.manifest(caps().wire).replace(None, &bytes).unwrap();
}
#[test]
fn reserved_input_exact_bytes_unknown_metadata_retry_and_failed_boot_are_durable_and_never_refunded(
) {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    baseline(&mut disk, registered, &book);
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        record_reserved(
            registered,
            &book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Manifest(ManifestError::UnknownOutcome(_)))
    ));
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        0
    );
    disk.fail_manifest_after_publish = false;
    record_reserved(
        registered,
        &book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        disk.load(token.packet().input().cid()).unwrap(),
        token.packet().input().as_bytes()
    );
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        launch_local(
            &wasm,
            registered,
            &mut book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Manifest(ManifestError::UnknownOutcome(_)))
    ));
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Attempted
    );
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        1
    );
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    assert_eq!(
        CheckpointManifest::decode(&bytes, &caps())
            .unwrap()
            .admissions()[0]
            .status,
        AdmissionStatus::Attempted
    );
    disk.fail_manifest_after_publish = false;
    let fresh = admission::registry(&wasm);
    let fresh = fresh.agent([0xb0; 32]).unwrap();
    let mut fresh_book = AdmissionBook::new(3).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let fresh_token = fresh_book.admit_actor_wake(fresh, input, grant).unwrap();
    assert!(matches!(
        launch_local(
            &wasm,
            fresh,
            &mut fresh_book,
            &fresh_token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Conflict)
    ));
    assert_eq!(
        fresh_book
            .record([0xb0; 32], [0x60; 32])
            .unwrap()
            .launch_attempts,
        0
    );
    assert!(matches!(
        launch_local(
            &wasm,
            registered,
            &mut book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Admission(AdmissionError::AlreadyAttempted))
    ));
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), bytes);
}
#[test]
fn source_absence_conflict_wrong_module_caps_and_missing_input_do_not_attempt_vm_creation() {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let old = CheckpointManifest::capture(registered, &book, &caps())
        .unwrap()
        .encode(&caps())
        .unwrap();
    disk.place(registered.snapshot()).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    assert!(matches!(
        record_reserved(
            registered,
            &book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::MissingManifest)
    ));
    disk.manifest(caps().wire).replace(None, &[0xa0]).unwrap();
    assert!(matches!(
        record_reserved(
            registered,
            &book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Conflict)
    ));
    disk.manifest(caps().wire)
        .replace(Some(&[0xa0]), &old)
        .unwrap();
    record_reserved(
        registered,
        &book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let reserved = disk.manifest(caps().wire).read().unwrap().unwrap();
    assert!(matches!(
        launch_local(
            &[0],
            registered,
            &mut book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Run(_))
    ));
    let tight = Limits {
        max_observation_bytes: 0,
        ..Limits::default()
    };
    assert!(launch_local(
        &wasm,
        registered,
        &mut book,
        &token,
        &mut disk,
        &caps(),
        &tight
    )
    .is_err());
    std::fs::remove_file(directory.0.join(token.packet().input().cid().to_string())).unwrap();
    assert!(launch_local(
        &wasm,
        registered,
        &mut book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default()
    )
    .is_err());
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        0
    );
    assert_eq!(
        disk.manifest(caps().wire).read().unwrap().unwrap(),
        reserved
    );
    assert_eq!(registered.anchor().revision, 0);
    disk.put(
        token.packet().input().cid(),
        token.packet().input().as_bytes(),
    )
    .unwrap();
    assert!(matches!(
        launch_local(
            &wasm,
            registered,
            &mut book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LaunchError::Run(_))
    ));
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Attempted
    );
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        1
    );
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    assert_eq!(
        CheckpointManifest::decode(&bytes, &caps())
            .unwrap()
            .admissions()[0]
            .status,
        AdmissionStatus::Attempted
    );
}
