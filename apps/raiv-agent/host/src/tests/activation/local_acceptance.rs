use super::*;
use crate::{
    acceptance::{accept_local, LocalAcceptanceError},
    admission::{AdmissionBook, AdmissionStatus, AdmittedInput},
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::{manifest::ManifestError, tests::Directory, DiskBlockStore},
    recovery::{adopt_local, LocalRecoveryPin},
    registry::Registry,
    run::SealedRun,
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

fn prepared(
    wasm: &[u8],
    registry: &Registry,
    book: &mut AdmissionBook,
    disk: &mut DiskBlockStore,
    event: u8,
    grant: u8,
    tool: &str,
) -> (AdmittedInput, SealedRun) {
    let registered = registry.agent([0xb0; 32]).unwrap();
    let input = admission::input(event, grant);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    let mut run = book.start_run(wasm, registered, &token).unwrap();
    // The test persists Launched before delivering a scripted model reply. This is
    // not a complete durable admission/attempt-before-instantiation application.
    disk.place(registered.snapshot()).unwrap();
    let bytes = CheckpointManifest::capture(registered, book, &caps())
        .unwrap()
        .encode(&caps())
        .unwrap();
    let previous = disk.manifest(caps().wire).read().unwrap();
    disk.manifest(caps().wire)
        .replace(previous.as_deref(), &bytes)
        .unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, tool, map(vec![])))
        .unwrap();
    {
        let sealed = run.into_sealed().unwrap();
        (token, sealed)
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn finish_yield_atomic_metadata_recovery_exact_retry_and_old_decision_do_not_reinfer_or_rollback() {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(4).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let (token, mut sealed) = prepared(&wasm, &registry, &mut book, &mut disk, 0x60, 0x70, "");
    let first = accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut sealed,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert!(sealed.snapshot().is_none());
    assert_eq!(first.revision, 1);
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Consumed
    );
    let first_bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    assert_eq!(
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default()
        )
        .unwrap(),
        first
    );
    assert_eq!(
        disk.manifest(caps().wire).read().unwrap().unwrap(),
        first_bytes
    );
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        1
    );
    let (token2, mut sealed2) =
        prepared(&wasm, &registry, &mut book, &mut disk, 0x61, 0x71, "trade");
    let second = accept_local(
        &mut registry,
        &mut book,
        &token2,
        &mut sealed2,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(second.revision, 2);
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    let data = CheckpointManifest::decode(&bytes, &caps()).unwrap();
    assert_eq!(data.anchor(), second);
    assert_eq!(
        data.admissions()
            .iter()
            .filter(|r| r.status == AdmissionStatus::Consumed)
            .count(),
        2
    );
    assert_eq!(
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default()
        )
        .unwrap(),
        first
    );
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), second);
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), bytes);
    let pin = LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: second,
        program: data.program_binding(),
        resources: data.resources(),
    };
    drop(disk);
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let mut recovered = Registry::new(2).unwrap();
    let mut recovered_book = AdmissionBook::new(4).unwrap();
    adopt_local(
        &wasm,
        &mut disk,
        &mut recovered,
        &mut recovered_book,
        pin,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        recovered.agent([0xb0; 32]).unwrap().snapshot(),
        registry.agent([0xb0; 32]).unwrap().snapshot()
    );
    assert!(matches!(
        recovered
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
fn unknown_publication_keeps_ram_and_handoff_old_then_exact_locked_retry_or_pinned_adoption_resolves(
) {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let (token, mut sealed) = prepared(&wasm, &registry, &mut book, &mut disk, 0x60, 0x70, "");
    let old = registry.agent([0xb0; 32]).unwrap().anchor();
    let record = book.record([0xb0; 32], [0x60; 32]);
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LocalAcceptanceError::Manifest(
            ManifestError::UnknownOutcome(_)
        ))
    ));
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), old);
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), record);
    assert!(sealed.snapshot().is_some());
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    let new = CheckpointManifest::decode(&bytes, &caps()).unwrap();
    assert_eq!(new.anchor().root, sealed.candidate().new_root());
    assert_eq!(new.admissions()[0].status, AdmissionStatus::Consumed);
    // A restart can approve the independently expected candidate metadata under lock;
    // adoption now re-fsyncs the entire closure and identical manifest before publication.
    let pin = LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: new.anchor(),
        program: new.program_binding(),
        resources: new.resources(),
    };
    let mut restart = Registry::new(2).unwrap();
    let mut restart_book = AdmissionBook::new(3).unwrap();
    adopt_local(
        &wasm,
        &mut disk,
        &mut restart,
        &mut restart_book,
        pin,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    disk.fail_manifest_after_publish = false;
    let decision = accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut sealed,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(decision, restart.agent([0xb0; 32]).unwrap().anchor());
    assert!(sealed.snapshot().is_none());
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        1
    );
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn absence_conflict_raw_run_caps_and_corrupt_source_refuse_without_head_consumption_or_handoff_loss(
) {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let (token, mut sealed) = prepared(&wasm, &registry, &mut book, &mut disk, 0x60, 0x70, "");
    let old = registry.agent([0xb0; 32]).unwrap().anchor();
    let record = book.record([0xb0; 32], [0x60; 32]);
    let source = disk.manifest(caps().wire).read().unwrap().unwrap();
    std::fs::remove_file(directory.0.join("raiv-manifest.cbor")).unwrap();
    assert!(matches!(
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LocalAcceptanceError::MissingManifest)
    ));
    disk.manifest(caps().wire).replace(None, &[0xa0]).unwrap();
    assert!(matches!(
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LocalAcceptanceError::Conflict)
    ));
    disk.manifest(caps().wire)
        .replace(Some(&[0xa0]), &source)
        .unwrap();
    let tight = Limits {
        max_blocks: 0,
        ..Limits::default()
    };
    assert!(accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut sealed,
        &mut disk,
        &caps(),
        &tight
    )
    .is_err());
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut raw = registered.start_run(&wasm, token.packet()).unwrap();
    let (ticket, _) = raw.pending().unwrap();
    raw.reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    let mut raw = raw.into_sealed().unwrap();
    assert!(matches!(
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut raw,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(LocalAcceptanceError::Binding(_))
    ));
    let path = directory.0.join(old.root.to_string());
    std::fs::write(&path, [0x80]).unwrap();
    assert!(accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut sealed,
        &mut disk,
        &caps(),
        &Limits::default()
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), [0x80]);
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), source);
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), old);
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), record);
    assert!(sealed.snapshot().is_some());
}
