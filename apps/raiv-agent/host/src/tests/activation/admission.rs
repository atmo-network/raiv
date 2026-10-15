//! Scoped volatile Actor/grant approval and launch reservation, no CAS/Receipt ledger.

use super::*;
use crate::{
    admission::{AdmissionBook, AdmissionError, AdmissionStatus},
    registry::Registry,
    run::{Phase, RunError},
};
use raiv::{input::ActivationInput, limits::Limits};

pub(crate) fn input(event: u8, admission: u8) -> ActivationInput {
    let mut value = packet();
    set(&mut value, "event_id", Value::Bytes(vec![event; 32]));
    let mut grant = field(&value, "budget_admission").clone();
    set(
        &mut grant,
        "admission_id",
        Value::Bytes(vec![admission; 32]),
    );
    set(&mut value, "budget_admission", grant);
    ActivationInput::decode(&encode(&value), &Limits::default()).unwrap()
}

pub(crate) fn registry(wasm: &[u8]) -> Registry {
    let source = routing::source();
    let pin = routing::pin(wasm, &source);
    let mut registry = Registry::new(2).unwrap();
    registry
        .register_genesis(wasm, pin, source, VmLimits::default())
        .unwrap();
    let other = super::registry::different_agent();
    registry
        .register_genesis(wasm, routing::pin(wasm, &other), other, VmLimits::default())
        .unwrap();
    registry
}

#[test]
fn explicit_grant_and_actor_phase_precede_atomic_bounded_exact_input_reservation() {
    assert!(matches!(
        AdmissionBook::new(0),
        Err(AdmissionError::InvalidCapacity)
    ));
    let wasm = output::invalid_output_module();
    let registry = registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(1).unwrap();
    let original = input(0x60, 0x70);
    let grant = original.budget_admission();
    let bad = raiv::budget::BudgetAdmission {
        remaining_episode_units: u64::MAX,
        ..grant
    };
    assert!(matches!(
        book.admit_actor_wake(registered, original, bad),
        Err(AdmissionError::GrantMismatch)
    ));
    assert!(book.is_empty());
    let inputs: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../fixtures/format/input.json")).unwrap();
    let action = inputs
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["record"]["cause"] == "action_result")
        .unwrap();
    let action =
        ActivationInput::decode(&encode(&fixture(&action["record"])), &Limits::default()).unwrap();
    let grant = action.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registered, action, grant),
        Err(AdmissionError::WrongPhase)
    ));
    assert!(book.is_empty());
    let original = input(0x60, 0x70);
    let exact = original.as_bytes().to_vec();
    let grant = original.budget_admission();
    let token = book.admit_actor_wake(registered, original, grant).unwrap();
    assert_eq!(token.packet().input().as_bytes(), exact);
    assert_eq!(
        token.packet().binding().expected_root,
        registered.anchor().root
    );
    assert_eq!(
        token.packet().binding().admitted_input_digest,
        token.packet().input().cid().digest()
    );
    let record = book.record([0xb0; 32], [0x60; 32]).unwrap();
    assert_eq!(record.grant, grant);
    assert_eq!(record.status, AdmissionStatus::Reserved);
    assert_eq!(record.launch_attempts, 0);
    for (event, admission) in [(0x60, 0x71), (0x61, 0x70)] {
        let input = input(event, admission);
        let grant = input.budget_admission();
        assert!(matches!(
            book.admit_actor_wake(registered, input, grant),
            Err(AdmissionError::Replay)
        ));
        assert_eq!(book.len(), 1);
    }
    let new = input(0x61, 0x71);
    let grant = new.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registered, new, grant),
        Err(AdmissionError::Busy)
    ));
    let new = input(0x61, 0x71);
    let grant = new.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registry.agent([0xb1; 32]).unwrap(), new, grant),
        Err(AdmissionError::Capacity)
    ));
    drop(token);
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), Some(record));
}

#[test]
fn foreign_tokens_wrong_modules_and_failed_launch_cannot_free_or_reuse_admission() {
    let wasm = output::invalid_output_module();
    let registry = registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut a = AdmissionBook::new(2).unwrap();
    let mut b = AdmissionBook::new(2).unwrap();
    let original = input(0x60, 0x70);
    let grant = original.budget_admission();
    let token = a.admit_actor_wake(registered, original, grant).unwrap();
    assert!(matches!(
        b.start_run(&wasm, registered, &token),
        Err(AdmissionError::ForeignToken)
    ));
    assert!(b.is_empty());
    assert!(matches!(
        a.start_run(&wasm, registry.agent([0xb1; 32]).unwrap(), &token),
        Err(AdmissionError::WrongRegistration)
    ));
    assert_eq!(
        a.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Reserved
    );
    assert!(matches!(
        a.start_run(b"other code", registered, &token),
        Err(AdmissionError::Run(RunError::ProgramBinding))
    ));
    assert_eq!(a.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts, 0);
    assert!(matches!(
        a.start_run(&wasm, registered, &token),
        Err(AdmissionError::Run(_))
    ));
    let failed = a.record([0xb0; 32], [0x60; 32]).unwrap();
    assert_eq!(failed.status, AdmissionStatus::Attempted);
    assert_eq!(failed.launch_attempts, 1);
    assert!(matches!(
        a.start_run(&wasm, registered, &token),
        Err(AdmissionError::AlreadyAttempted)
    ));
    assert_eq!(a.record([0xb0; 32], [0x60; 32]), Some(failed));
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn admitted_actual_run_is_bound_to_one_origin_while_unadmitted_recomputation_cannot_claim_it() {
    let wasm = wasm();
    let registry = registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let anchor = registered.anchor();
    let mut book = AdmissionBook::new(2).unwrap();
    let original = input(0x60, 0x70);
    let grant = original.budget_admission();
    let token = book.admit_actor_wake(registered, original, grant).unwrap();
    let mut admitted = book.start_run(&wasm, registered, &token).unwrap();
    assert!(book.matches_run(registered, &token, &admitted));
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().status,
        AdmissionStatus::Launched
    );
    assert!(matches!(
        book.start_run(&wasm, registered, &token),
        Err(AdmissionError::AlreadyAttempted)
    ));
    let raw = registered.start_run(&wasm, token.packet()).unwrap();
    assert_eq!(raw.input_binding(), admitted.input_binding());
    assert_eq!(raw.program_binding(), admitted.program_binding());
    assert!(!book.matches_run(registered, &token, &raw));
    let (ticket, _) = admitted.pending().unwrap();
    admitted
        .reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    assert_eq!(admitted.phase(), Phase::Boundary);
    assert!(book.matches_run(registered, &token, &admitted));
    assert_eq!(registered.anchor(), anchor);
    assert_eq!(
        book.record([0xb0; 32], [0x60; 32]).unwrap().launch_attempts,
        1
    );
    assert!(matches!(
        book.start_run(&wasm, registered, &token),
        Err(AdmissionError::AlreadyAttempted)
    ));
    let input = input(0x61, 0x71);
    let grant = input.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registered, input, grant),
        Err(AdmissionError::Busy)
    ));
    let original = super::admission::input(0x61, 0x71);
    let grant = original.budget_admission();
    let registered = registry.agent([0xb1; 32]).unwrap();
    let failure = book.admit_actor_wake(registered, original, grant).unwrap();
    let mut run = book.start_run(&wasm, registered, &failure).unwrap();
    let (ticket, _) = run.pending().unwrap();
    assert!(matches!(
        run.inference_error(ticket, "synthetic provider failure"),
        Err(RunError::GuestFailure)
    ));
    assert_eq!(run.phase(), Phase::Failed);
    assert!(book.matches_run(registered, &failure, &run));
    assert!(matches!(
        book.start_run(&wasm, registered, &failure),
        Err(AdmissionError::AlreadyAttempted)
    ));
}
