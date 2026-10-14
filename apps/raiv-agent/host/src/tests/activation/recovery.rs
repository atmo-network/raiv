use super::*;
use crate::{
    acceptance::accept_ram,
    admission::{AdmissionBook, AdmissionError, AdmissionStatus},
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::{tests::Directory, DiskBlockStore},
    recovery::{adopt_local, LocalRecoveryPin},
    registry::Registry,
    store::{MemoryBlockStore, StoreLimits},
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
fn save(
    disk: &mut DiskBlockStore,
    registered: &crate::registry::RegisteredAgent,
    book: &AdmissionBook,
) -> LocalRecoveryPin {
    disk.place(registered.snapshot()).unwrap();
    let data = CheckpointManifest::capture(registered, book, &caps()).unwrap();
    let bytes = data.encode(&caps()).unwrap();
    let previous = disk.manifest(caps().wire).read().unwrap();
    disk.manifest(caps().wire)
        .replace(previous.as_deref(), &bytes)
        .unwrap();
    LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: data.anchor(),
        program: data.program_binding(),
        resources: data.resources(),
    }
}
fn finish(
    wasm: &[u8],
    registry: &mut Registry,
    book: &mut AdmissionBook,
    event: u8,
    grant: u8,
    tool: &str,
) {
    let registered = registry.agent([0xb0; 32]).unwrap();
    let input = admission::input(event, grant);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    let mut run = book.start_run(wasm, registered, &token).unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, tool, map(vec![])))
        .unwrap();
    let mut run = run.into_sealed().unwrap();
    let mut ram = MemoryBlockStore::new(StoreLimits {
        max_blocks: 100,
        max_bytes: 1_000_000,
        wire: DecodeLimits::default(),
    });
    ram.place(run.snapshot().unwrap()).unwrap();
    accept_ram(registry, book, &token, &mut run, &ram).unwrap();
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn recovered_consumed_ids_cannot_replay_fresh_run_works_and_unresolved_launch_remains_blocked() {
    let wasm = wasm();
    let mut original = admission::registry(&wasm);
    let mut original_book = AdmissionBook::new(4).unwrap();
    finish(&wasm, &mut original, &mut original_book, 0x60, 0x70, "");
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let pin = save(
        &mut disk,
        original.agent([0xb0; 32]).unwrap(),
        &original_book,
    );
    drop(disk);
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let mut registry = Registry::new(2).unwrap();
    let mut book = AdmissionBook::new(4).unwrap();
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
    assert_eq!(registered.anchor().revision, 1);
    assert_eq!(
        registered.snapshot(),
        original.agent([0xb0; 32]).unwrap().snapshot()
    );
    for (event, grant) in [(0x60, 0x71), (0x61, 0x70)] {
        let input = admission::input(event, grant);
        let grant = input.budget_admission();
        assert!(matches!(
            book.admit_actor_wake(registered, input, grant),
            Err(AdmissionError::Replay)
        ));
    }
    let input = admission::input(0x61, 0x71);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    let mut run = book.start_run(&wasm, registered, &token).unwrap();
    let (ticket, _) = run.pending().unwrap();
    assert!(run
        .inference_error(ticket, "scripted provider loss")
        .is_err());
    drop(run);
    let unresolved = save(&mut disk, registry.agent([0xb0; 32]).unwrap(), &book);
    let mut restarted = Registry::new(2).unwrap();
    let mut restarted_book = AdmissionBook::new(4).unwrap();
    adopt_local(
        &wasm,
        &mut disk,
        &mut restarted,
        &mut restarted_book,
        unresolved,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let registered = restarted.agent([0xb0; 32]).unwrap();
    let record = restarted_book.record([0xb0; 32], [0x61; 32]).unwrap();
    assert_eq!(record.status, AdmissionStatus::Launched);
    assert_eq!(record.launch_attempts, 1);
    assert_eq!(registered.anchor(), pin.anchor);
    assert!(matches!(
        restarted_book.start_run(&wasm, registered, &token),
        Err(AdmissionError::ForeignToken)
    ));
    let input = admission::input(0x62, 0x72);
    let grant = input.budget_admission();
    assert!(matches!(
        restarted_book.admit_actor_wake(registered, input, grant),
        Err(AdmissionError::Busy)
    ));
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn recovered_pending_command_is_exact_and_wake_cannot_bypass_it_or_dispatch_an_effect() {
    let wasm = wasm();
    let mut original = admission::registry(&wasm);
    let mut original_book = AdmissionBook::new(4).unwrap();
    finish(
        &wasm,
        &mut original,
        &mut original_book,
        0x60,
        0x70,
        "trade",
    );
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let pin = save(
        &mut disk,
        original.agent([0xb0; 32]).unwrap(),
        &original_book,
    );
    let expected = original.agent([0xb0; 32]).unwrap().snapshot();
    let mut registry = Registry::new(2).unwrap();
    let mut book = AdmissionBook::new(4).unwrap();
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
    assert_eq!(registered.snapshot(), expected);
    assert!(matches!(
        registered.snapshot().state().continuation(),
        raiv::state::Continuation::AwaitAction { .. }
    ));
    let input = admission::input(0x61, 0x71);
    let grant = input.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registered, input, grant),
        Err(AdmissionError::WrongPhase)
    ));
    assert_eq!(registered.anchor(), pin.anchor);
    assert_eq!(
        CheckpointManifest::capture(registered, &book, &caps())
            .unwrap()
            .encode(&caps())
            .unwrap(),
        disk.manifest(caps().wire).read().unwrap().unwrap()
    );
}
