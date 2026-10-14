use super::*;
use crate::{
    acceptance::accept_local,
    admission::{AdmissionBook, AdmissionError, AdmissionStatus},
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::{manifest::ManifestError, tests::Directory, DiskBlockStore},
    launch::{launch_local, record_reserved, LaunchError},
};
use raiv::{format::DecodeLimits, limits::Limits};
fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}
fn baseline(
    disk: &mut DiskBlockStore,
    registered: &crate::registry::RegisteredAgent,
    book: &AdmissionBook,
) {
    disk.place(registered.snapshot()).unwrap();
    let bytes = CheckpointManifest::capture(registered, book, &caps())
        .unwrap()
        .encode(&caps())
        .unwrap();
    disk.manifest(caps().wire).replace(None, &bytes).unwrap();
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn durable_reserved_attempted_launched_exposure_and_two_boundary_commits_keep_exact_inputs_and_counts(
) {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(4).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    baseline(&mut disk, registry.agent([0xb0; 32]).unwrap(), &book);
    for (event, grant, tool) in [(0x60, 0x70, ""), (0x61, 0x71, "trade")] {
        let registered = registry.agent([0xb0; 32]).unwrap();
        let input = admission::input(event, grant);
        let grant = input.budget_admission();
        let token = book.admit_actor_wake(registered, input, grant).unwrap();
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
        assert!(book.matches_run(registered, &token, &run));
        let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
        let data = CheckpointManifest::decode(&bytes, &caps()).unwrap();
        let row = data
            .admissions()
            .iter()
            .find(|r| r.binding.admitted_event_id == [event; 32])
            .unwrap();
        assert_eq!(row.status, AdmissionStatus::Launched);
        assert_eq!(row.launch_attempts, 1);
        assert_eq!(
            row.binding.admitted_input_digest,
            token.packet().input().cid().digest()
        );
        let (ticket, _) = run.pending().unwrap();
        run.reply(ticket, &routing::normalized(1, tool, map(vec![])))
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
        assert_eq!(accepted.revision, if event == 0x60 { 1 } else { 2 });
        assert_eq!(
            book.record([0xb0; 32], [event; 32]).unwrap().status,
            AdmissionStatus::Consumed
        );
    }
    assert!(matches!(
        registry
            .agent([0xb0; 32])
            .unwrap()
            .snapshot()
            .state()
            .continuation(),
        raiv::state::Continuation::AwaitAction { .. }
    ));
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn launched_publication_failure_never_exposes_a_run_or_permits_repeat_launch_or_refund() {
    let wasm = wasm();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    baseline(&mut disk, registered, &book);
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    record_reserved(
        registered,
        &book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    disk.manifest_failure_countdown = Some(1);
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
    let record = book.record([0xb0; 32], [0x60; 32]).unwrap();
    assert_eq!(record.status, AdmissionStatus::Launched);
    assert_eq!(record.launch_attempts, 1);
    assert_eq!(registered.anchor().revision, 0);
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    let data = CheckpointManifest::decode(&bytes, &caps()).unwrap();
    assert_eq!(data.admissions()[0].status, AdmissionStatus::Launched);
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
    let input = admission::input(0x61, 0x71);
    let grant = input.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registered, input, grant),
        Err(AdmissionError::Busy)
    ));
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), Some(record));
}
