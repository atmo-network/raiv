//! Volatile transaction evidence only; never persistence/finality/effect authorization.
use super::*;
use crate::{
    acceptance::{accept_ram, AcceptanceError},
    admission::{AdmissionBook, AdmissionStatus, AdmittedInput},
    registry::RegisteredAgent,
    run::{RunError, SealedRun},
    store::{MemoryBlockStore, StoreLimits},
};
use raiv::{format::DecodeLimits, graph::MaterializedBlocks};
fn store() -> MemoryBlockStore {
    MemoryBlockStore::new(StoreLimits {
        max_blocks: 1000,
        max_bytes: 1_000_000,
        wire: DecodeLimits::default(),
    })
}
fn sealed(
    wasm: &[u8],
    registered: &RegisteredAgent,
    book: &mut AdmissionBook,
    event: u8,
    grant: u8,
    tool: &str,
) -> (AdmittedInput, SealedRun) {
    let input = admission::input(event, grant);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    let mut run = book.start_run(wasm, registered, &token).unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, tool, map(vec![])))
        .unwrap();
    let sealed = run.into_sealed().unwrap();
    (token, sealed)
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn complete_readback_precedes_atomic_ram_anchor_consumption_and_exact_retry_without_inference() {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(4).unwrap();
    let old = registry.agent([0xb0; 32]).unwrap().anchor();
    let (token, mut output) = sealed(
        &wasm,
        registry.agent([0xb0; 32]).unwrap(),
        &mut book,
        0x60,
        0x70,
        "",
    );
    let next = output.candidate().new_root();
    let before = book.record([0xb0; 32], [0x60; 32]).unwrap();
    let mut store = store();
    assert!(matches!(
        accept_ram(&mut registry, &mut book, &token, &mut output, &store),
        Err(AcceptanceError::Readback(_))
    ));
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), old);
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), Some(before));
    assert!(output.snapshot().is_some());
    store.place(output.snapshot().unwrap()).unwrap();
    let block_count = store.block_count();
    let decision = accept_ram(&mut registry, &mut book, &token, &mut output, &store).unwrap();
    assert_eq!(decision.root, next);
    assert_eq!(decision.revision, 1);
    assert!(output.snapshot().is_none());
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Consumed
    );
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().accepted_root,
        Some(next)
    );
    assert_eq!(registry.agent([0xb0; 32]).unwrap().snapshot().root(), next);
    assert_eq!(
        accept_ram(&mut registry, &mut book, &token, &mut output, &store).unwrap(),
        decision
    );
    assert_eq!(store.block_count(), block_count);
    // One next ActorWake is possible only after the first RAM transaction consumed its reservation.
    let (token2, mut output2) = sealed(
        &wasm,
        registry.agent([0xb0; 32]).unwrap(),
        &mut book,
        0x61,
        0x71,
        "",
    );
    store.place(output2.snapshot().unwrap()).unwrap();
    let later = accept_ram(&mut registry, &mut book, &token2, &mut output2, &store).unwrap();
    assert_eq!(later.revision, 2);
    // A retry returns the previous known decision, not a HEAD rollback.
    assert_eq!(
        accept_ram(&mut registry, &mut book, &token, &mut output, &store).unwrap(),
        decision
    );
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), later);
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn substituted_run_registry_and_competing_stale_anchor_never_consume_or_install_output() {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut a = AdmissionBook::new(4).unwrap();
    let mut b = AdmissionBook::new(4).unwrap();
    let (token_a, mut one) = sealed(
        &wasm,
        registry.agent([0xb0; 32]).unwrap(),
        &mut a,
        0x60,
        0x70,
        "",
    );
    let (token_b, mut two) = sealed(
        &wasm,
        registry.agent([0xb0; 32]).unwrap(),
        &mut b,
        0x61,
        0x71,
        "",
    );
    let mut store = store();
    store.place(one.snapshot().unwrap()).unwrap();
    store.place(two.snapshot().unwrap()).unwrap();
    let mut foreign_registry = admission::registry(&wasm);
    assert!(matches!(
        accept_ram(&mut foreign_registry, &mut a, &token_a, &mut one, &store),
        Err(AcceptanceError::Program)
    ));
    assert!(one.snapshot().is_some());
    assert_eq!(
        a.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Launched
    );
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut raw = registered.start_run(&wasm, token_a.packet()).unwrap();
    let (ticket, _) = raw.pending().unwrap();
    raw.reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    let mut raw = raw.into_sealed().unwrap();
    assert!(matches!(
        accept_ram(&mut registry, &mut a, &token_a, &mut raw, &store),
        Err(AcceptanceError::Admission(_))
    ));
    let decision = accept_ram(&mut registry, &mut a, &token_a, &mut one, &store).unwrap();
    let old_record = b.record([0xb0; 32], [0x61; 32]).unwrap();
    assert!(matches!(
        accept_ram(&mut registry, &mut b, &token_b, &mut two, &store),
        Err(AcceptanceError::StaleAnchor)
    ));
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), decision);
    assert_eq!(b.record([0xb0; 32], [0x61; 32]), Some(old_record));
    assert!(two.snapshot().is_some());
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn yield_installs_the_same_embedded_command_only_as_ram_data_and_pending_or_failed_runs_cannot_seal(
) {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(3).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = book
        .admit_actor_wake(registry.agent([0xb0; 32]).unwrap(), input, grant)
        .unwrap();
    let run = book
        .start_run(&wasm, registry.agent([0xb0; 32]).unwrap(), &token)
        .unwrap();
    assert!(matches!(run.into_sealed(), Err(RunError::Phase)));
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut failed = registered.start_run(&wasm, token.packet()).unwrap();
    let (ticket, _) = failed.pending().unwrap();
    assert!(matches!(
        failed.inference_error(ticket, "synthetic loss"),
        Err(RunError::GuestFailure)
    ));
    assert!(matches!(failed.into_sealed(), Err(RunError::Phase)));
    // A separate agent is an isolated reservation/phase, not a retry of the lost activation.
    let (token, mut output) = sealed(
        &wasm,
        registry.agent([0xb1; 32]).unwrap(),
        &mut book,
        0x61,
        0x71,
        "trade",
    );
    let next = output.snapshot().unwrap();
    let last = next.current_episode().unwrap().boundaries().last().unwrap();
    let raiv::boundary::Exit::AwaitAction(command) = next.boundary(last).unwrap().exit() else {
        panic!()
    };
    let command = command.to_vec();
    let mut store = store();
    store.place(next).unwrap();
    let decision = accept_ram(&mut registry, &mut book, &token, &mut output, &store).unwrap();
    let accepted = registry.agent([0xb1; 32]).unwrap().snapshot();
    let last = accepted
        .current_episode()
        .unwrap()
        .boundaries()
        .last()
        .unwrap();
    let raiv::boundary::Exit::AwaitAction(same) = accepted.boundary(last).unwrap().exit() else {
        panic!()
    };
    assert_eq!(same, command);
    assert_eq!(decision.root, output.candidate().new_root());
    assert_eq!(
        store.get(last),
        Some(accepted.boundary(last).unwrap().as_bytes())
    );
    // ActorWake cannot bypass the resulting AwaitAction, even after reservation consumption.
    let input = admission::input(0x62, 0x72);
    let grant = input.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registry.agent([0xb1; 32]).unwrap(), input, grant),
        Err(crate::admission::AdmissionError::WrongPhase)
    ));
}
