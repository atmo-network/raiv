use super::*;
use crate::{
    acceptance::accept_local,
    admission::{AdmissionBook, AdmissionError, AdmissionStatus},
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::{tests::Directory, DiskBlockStore},
    launch::{launch_local, record_reserved},
    recovery::{adopt_local, resume_reserved, LocalRecoveryPin, RecoveryError},
    registry::Registry,
};
use raiv::{
    format::{DaslCid, DaslCodec, DecodeLimits},
    limits::Limits,
};

fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}

fn snapshot_pin(disk: &mut DiskBlockStore) -> LocalRecoveryPin {
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    let data = CheckpointManifest::decode(&bytes, &caps()).unwrap();
    LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: data.anchor(),
        program: data.program_binding(),
        resources: data.resources(),
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn reserved_restart_reissues_new_capability_for_same_exact_input_then_launches_and_commits_once() {
    let wasm = wasm();
    let original = admission::registry(&wasm);
    let registered = original.agent([0xb0; 32]).unwrap();
    let mut old_book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.place(registered.snapshot()).unwrap();
    let baseline = CheckpointManifest::capture(registered, &old_book, &caps())
        .unwrap()
        .encode(&caps())
        .unwrap();
    disk.manifest(caps().wire).replace(None, &baseline).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let old_token = old_book.admit_actor_wake(registered, input, grant).unwrap();
    record_reserved(
        registered,
        &old_book,
        &old_token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let pin = snapshot_pin(&mut disk);
    let mut registry = Registry::new(2).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    adopt_local(
        &wasm,
        &mut disk,
        &mut registry,
        &mut book,
        pin,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let registered = registry.agent([0xb0; 32]).unwrap();
    assert!(matches!(
        book.start_run(&wasm, registered, &old_token),
        Err(AdmissionError::ForeignToken)
    ));
    let token = resume_reserved(
        registered,
        &mut book,
        &mut disk,
        [0x60; 32],
        pin,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        token.packet().input().as_bytes(),
        old_token.packet().input().as_bytes()
    );
    assert_eq!(token.packet().binding(), old_token.packet().binding());
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        0
    );
    assert!(matches!(
        resume_reserved(
            registered,
            &mut book,
            &mut disk,
            [0x60; 32],
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::Admission(AdmissionError::TokenAlreadyIssued))
    ));
    let mut run = launch_local(
        &wasm,
        registered,
        &mut book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    let mut run = run.into_sealed().unwrap();
    let accepted = accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut run,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(accepted.revision, 1);
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Consumed
    );
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        1
    );
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn attempted_and_launched_restart_never_reissue_zero_attempt_capability_or_change_counts() {
    let wasm = wasm();
    for uncertain in [true, false] {
        let original = admission::registry(&wasm);
        let registered = original.agent([0xb0; 32]).unwrap();
        let mut old_book = AdmissionBook::new(3).unwrap();
        let directory = Directory::new();
        let mut disk =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        disk.place(registered.snapshot()).unwrap();
        let baseline = CheckpointManifest::capture(registered, &old_book, &caps())
            .unwrap()
            .encode(&caps())
            .unwrap();
        disk.manifest(caps().wire).replace(None, &baseline).unwrap();
        let input = admission::input(0x60, 0x70);
        let grant = input.budget_admission();
        let token = old_book.admit_actor_wake(registered, input, grant).unwrap();
        record_reserved(
            registered,
            &old_book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        disk.fail_manifest_after_publish = uncertain;
        let run = launch_local(
            &wasm,
            registered,
            &mut old_book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default(),
        );
        if uncertain {
            assert!(run.is_err());
        } else {
            assert!(run.is_ok());
        }
        drop(run);
        disk.fail_manifest_after_publish = false;
        let pin = snapshot_pin(&mut disk);
        let mut registry = Registry::new(2).unwrap();
        let mut book = AdmissionBook::new(3).unwrap();
        adopt_local(
            &wasm,
            &mut disk,
            &mut registry,
            &mut book,
            pin,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let before = book.record([0xb0; 32], [0x60; 32]);
        assert!(matches!(
            resume_reserved(
                registered,
                &mut book,
                &mut disk,
                [0x60; 32],
                pin,
                &caps(),
                &Limits::default()
            ),
            Err(RecoveryError::Admission(AdmissionError::AlreadyAttempted))
        ));
        assert_eq!(book.record([0xb0; 32], [0x60; 32]), before);
        assert_eq!(before.unwrap().launch_attempts, 1);
        assert_eq!(registered.anchor().revision, 0);
    }
}
