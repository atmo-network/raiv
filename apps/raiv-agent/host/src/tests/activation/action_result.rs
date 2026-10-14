//! Local administrative Receipt DATA approval, not an executor ledger/effect proof.
use super::*;
use crate::{
    admission::{AdmissionBook, AdmissionError, AdmissionStatus},
    registry::Registry,
};
use raiv::{
    boundary::Exit,
    command::Command,
    graph::RestoredAgent,
    input::{ActivationInput, ActivationInputSpec, Cause},
    limits::Limits,
    position::Position,
    receipt::{Outcome, Receipt, ReceiptSpec},
};
pub(super) fn receipt(source: &RestoredAgent, outcome: Outcome) -> Receipt {
    let current = source.current_episode().unwrap();
    let boundary = source
        .boundary(current.boundaries().last().unwrap())
        .unwrap();
    let Exit::AwaitAction(bytes) = boundary.exit() else {
        panic!()
    };
    let command = Command::decode(bytes, &Limits::default()).unwrap();
    Receipt::encode(
        &ReceiptSpec {
            action_id: command.action_id(),
            command_digest: raiv::identity::command_digest(
                source.definition().execution_domain(),
                &command,
            ),
            outcome,
            result: &[0xa0],
            execution_reference: None,
            finalized_at: Position {
                domain: [0xcc; 32],
                number: 1,
                reference: None,
            },
            costs: &[0xa0],
        },
        &Limits::default(),
    )
    .unwrap()
}
pub(super) fn input(receipt: &Receipt, event: u8, grant: u8) -> ActivationInput {
    ActivationInput::encode(
        &ActivationInputSpec {
            event_id: [event; 32],
            cause: Cause::ActionResult,
            observed_at: Position {
                domain: [0xcc; 32],
                number: 2,
                reference: None,
            },
            observation: &[0xa0],
            receipt: Some(receipt),
            budget_admission: admission::input(event, grant).budget_admission(),
        },
        &Limits::default(),
    )
    .unwrap()
}
fn pending_registry(wasm: &[u8]) -> Registry {
    let source = candidate::source();
    let packet = candidate::prepared(&source, &packet());
    let frame = candidate::boundary(&source, &packet, None, "trade", map(vec![]));
    let pending = candidate::check(&frame, &source, &packet, &Limits::default()).unwrap();
    let pin = routing::pin(wasm, &pending);
    let mut registry = Registry::new(1).unwrap();
    let approved = registry
        .stage_approved(wasm, pin, pending, VmLimits::default())
        .unwrap();
    registry.publish_approved(approved);
    registry
}
#[test]
fn action_result_native_approval_and_pending_identity_precede_atomic_reservation() {
    let wasm = output::invalid_output_module();
    let registry = pending_registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let receipt = receipt(registered.snapshot(), Outcome::Succeeded);
    let mut book = AdmissionBook::new(2).unwrap();
    let wake = admission::input(0x61, 0x71);
    let grant = wake.budget_admission();
    assert!(matches!(
        book.admit_action_result(registered, wake, &receipt, grant),
        Err(AdmissionError::WrongPhase)
    ));
    let genesis = admission::registry(&wasm);
    let value = input(&receipt, 0x61, 0x71);
    let grant = value.budget_admission();
    assert!(matches!(
        book.admit_action_result(genesis.agent([0xb0; 32]).unwrap(), value, &receipt, grant),
        Err(AdmissionError::WrongPhase)
    ));
    let different = self::receipt(registered.snapshot(), Outcome::Failed);
    let value = input(&receipt, 0x61, 0x71);
    let grant = value.budget_admission();
    assert!(matches!(
        book.admit_action_result(registered, value, &different, grant),
        Err(AdmissionError::ReceiptMismatch)
    ));
    for name in ["action_id", "command_digest"] {
        let mut wrong = decode(receipt.as_bytes());
        set(&mut wrong, name, Value::Bytes(vec![0xee; 32]));
        let wrong = Receipt::decode(&encode(&wrong), &Limits::default()).unwrap();
        let value = input(&wrong, 0x61, 0x71);
        let grant = value.budget_admission();
        assert!(matches!(
            book.admit_action_result(registered, value, &wrong, grant),
            Err(AdmissionError::ReceiptMismatch)
        ));
    }
    let value = input(&receipt, 0x61, 0x71);
    let grant = raiv::budget::BudgetAdmission {
        activation_units: 1,
        ..value.budget_admission()
    };
    assert!(matches!(
        book.admit_action_result(registered, value, &receipt, grant),
        Err(AdmissionError::GrantMismatch)
    ));
    assert!(book.is_empty());
    assert_eq!(registered.anchor().revision, 1);
}
#[test]
fn every_terminal_outcome_preserves_exact_receipt_input_grant_and_reserved_ids_without_releasing_on_drop(
) {
    let wasm = output::invalid_output_module();
    let registry = pending_registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    for outcome in [Outcome::Succeeded, Outcome::Failed, Outcome::NotExecuted] {
        let receipt = receipt(registered.snapshot(), outcome);
        let value = input(&receipt, 0x61, 0x71);
        let exact = value.as_bytes().to_vec();
        let grant = value.budget_admission();
        let mut book = AdmissionBook::new(2).unwrap();
        let token = book
            .admit_action_result(registered, value, &receipt, grant)
            .unwrap();
        assert_eq!(token.packet().input().as_bytes(), exact);
        assert_eq!(
            token.packet().input().receipt_bytes(),
            Some(receipt.as_bytes())
        );
        assert_eq!(
            token.packet().binding().expected_root,
            registered.anchor().root
        );
        let before = book.record([0xb0; 32], [0x61; 32]).unwrap();
        assert_eq!(before.status, AdmissionStatus::Reserved);
        assert_eq!(before.launch_attempts, 0);
        for (event, grant) in [(0x61, 0x72), (0x62, 0x71), (0x62, 0x72)] {
            let value = input(&receipt, event, grant);
            let approval = value.budget_admission();
            assert!(matches!(
                book.admit_action_result(registered, value, &receipt, approval),
                Err(AdmissionError::Replay | AdmissionError::Busy)
            ));
        }
        drop(token);
        assert_eq!(book.record([0xb0; 32], [0x61; 32]), Some(before));
    }
}
#[test]
fn retained_events_and_embedding_receipt_bounds_fail_before_native_ownership() {
    let wasm = output::invalid_output_module();
    let registry = pending_registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let receipt = receipt(registered.snapshot(), Outcome::Succeeded);
    let mut book = AdmissionBook::new(2).unwrap();
    let mut value = decode(input(&receipt, 0x61, 0x71).as_bytes());
    set(
        &mut value,
        "event_id",
        Value::Bytes(
            registered
                .snapshot()
                .current_episode()
                .unwrap()
                .trigger_event_id()
                .to_vec(),
        ),
    );
    let value = ActivationInput::decode(&encode(&value), &Limits::default()).unwrap();
    let grant = value.budget_admission();
    assert!(matches!(
        book.admit_action_result(registered, value, &receipt, grant),
        Err(AdmissionError::Replay)
    ));
    let mut big = decode(receipt.as_bytes());
    set(
        &mut big,
        "result",
        map(vec![("large", Value::Text("x".repeat(5000)))]),
    );
    let wide = Limits {
        max_receipt_bytes: 10_000,
        ..Limits::default()
    };
    let big = Receipt::decode(&encode(&big), &wide).unwrap();
    let mut value = decode(input(&receipt, 0x61, 0x71).as_bytes());
    set(&mut value, "receipt", decode(big.as_bytes()));
    let value = ActivationInput::decode(&encode(&value), &wide).unwrap();
    let grant = value.budget_admission();
    assert!(matches!(
        book.admit_action_result(registered, value, &big, grant),
        Err(AdmissionError::Input(_))
    ));
    assert!(book.is_empty());
}

#[cfg(target_os = "linux")]
pub(super) mod durable {
    use super::*;
    use crate::{
        acceptance::accept_local,
        checkpoint::ManifestLimits,
        disk::{tests::Directory, DiskBlockStore},
        launch::{launch_local, record_reserved},
        policy::{current_pin, initialize_local},
        recovery::{adopt_local, resume_reserved},
    };
    use raiv::{format::DecodeLimits, state::Continuation};
    fn caps() -> ManifestLimits {
        ManifestLimits {
            wire: DecodeLimits::default(),
            max_blocks: 100,
            max_admissions: 10,
        }
    }
    pub(in crate::tests::activation) fn pending(
    ) -> (Vec<u8>, Registry, AdmissionBook, DiskBlockStore, Directory) {
        let wasm = wasm();
        let mut registry = admission::registry(&wasm);
        let registered = registry.agent([0xb0; 32]).unwrap();
        let mut book = AdmissionBook::new(6).unwrap();
        let directory = Directory::new();
        let mut disk =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        let source_pin =
            initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
        let executor = crate::simulator::ExecutorBook::new(4).unwrap();
        let bounds = crate::simulator::local::LocalExecutorLimits {
            executor: crate::simulator::checkpoint::ExecutorLimits {
                wire: DecodeLimits::default(),
                max_actions: 10,
            },
            manifest: caps(),
            caller: Limits::default(),
        };
        crate::simulator::local::initialize_genesis(
            registered,
            &book,
            &executor,
            &mut disk,
            &source_pin,
            &bounds,
        )
        .unwrap();
        let value = admission::input(0x60, 0x70);
        let grant = value.budget_admission();
        let token = book.admit_actor_wake(registered, value, grant).unwrap();
        record_reserved(
            registered,
            &book,
            &token,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
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
        run.reply(ticket, &routing::normalized(1, "trade", map(vec![])))
            .unwrap();
        let mut sealed = run.into_sealed().unwrap();
        accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        (wasm, registry, book, disk, directory)
    }
    #[test]
    #[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
    fn failed_receipt_resumes_same_episode_and_retry_gets_new_action_then_success_finishes_with_retained_counts(
    ) {
        let (wasm, mut registry, mut book, mut disk, _directory) = pending();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let first_episode = registered.snapshot().current_episode().unwrap().id();
        let failed = receipt(registered.snapshot(), Outcome::Failed);
        let value = input(&failed, 0x61, 0x71);
        let grant = value.budget_admission();
        let exact = value.as_bytes().to_vec();
        let token = book
            .admit_action_result(registered, value, &failed, grant)
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
        run.reply(ticket, &routing::normalized(1, "trade", map(vec![])))
            .unwrap();
        let mut sealed = run.into_sealed().unwrap();
        let second = accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(second.revision, 2);
        let registered = registry.agent([0xb0; 32]).unwrap();
        let episode = registered.snapshot().current_episode().unwrap();
        assert_eq!(episode.id(), first_episode);
        assert_eq!(episode.boundary_count(), 2);
        let total: u64 = episode
            .boundaries()
            .map(|cid| {
                registered
                    .snapshot()
                    .boundary(cid)
                    .unwrap()
                    .usage()
                    .model_calls
            })
            .sum();
        assert_eq!(total, 2);
        let succeeded = receipt(registered.snapshot(), Outcome::Succeeded);
        assert_ne!(failed.action_id(), succeeded.action_id());
        assert_ne!(failed.command_digest(), succeeded.command_digest());
        assert_eq!(
            registered
                .snapshot()
                .input(
                    episode
                        .boundaries()
                        .last()
                        .and_then(|cid| registered.snapshot().boundary(cid))
                        .unwrap()
                        .input_link()
                )
                .unwrap()
                .as_bytes(),
            exact
        );
        let value = input(&succeeded, 0x62, 0x72);
        let grant = value.budget_admission();
        let token = book
            .admit_action_result(registered, value, &succeeded, grant)
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
        let mut sealed = run.into_sealed().unwrap();
        let third = accept_local(
            &mut registry,
            &mut book,
            &token,
            &mut sealed,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(third.revision, 3);
        let registered = registry.agent([0xb0; 32]).unwrap();
        assert_eq!(
            registered.snapshot().state().continuation(),
            Continuation::AwaitActor
        );
        let completed = registered.snapshot().history().last().unwrap();
        assert_eq!(completed.id(), first_episode);
        assert_eq!(completed.boundary_count(), 3);
        assert_eq!(
            completed
                .boundaries()
                .map(|cid| registered
                    .snapshot()
                    .boundary(cid)
                    .unwrap()
                    .usage()
                    .model_calls)
                .sum::<u64>(),
            3
        );
        assert!(completed.boundaries().all(|cid| registered
            .snapshot()
            .boundary(cid)
            .unwrap()
            .usage()
            .billed_units
            .is_none()));
    }
    #[test]
    #[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
    fn action_result_reserved_restart_preserves_approved_exact_receipt_grant_and_matches_pending_before_reissue(
    ) {
        let (wasm, registry, mut book, mut disk, _directory) = pending();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let receipt = receipt(registered.snapshot(), Outcome::NotExecuted);
        let value = input(&receipt, 0x61, 0x71);
        let grant = value.budget_admission();
        let old_token = book
            .admit_action_result(registered, value, &receipt, grant)
            .unwrap();
        record_reserved(
            registered,
            &book,
            &old_token,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        let pin = current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
        let mut restored = Registry::new(2).unwrap();
        let mut recovered = AdmissionBook::new(6).unwrap();
        adopt_local(
            &wasm,
            &mut disk,
            &mut restored,
            &mut recovered,
            pin,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        let registered = restored.agent([0xb0; 32]).unwrap();
        let before = recovered.record([0xb0; 32], [0x61; 32]);
        let tight = Limits {
            max_receipt_bytes: 0,
            ..Limits::default()
        };
        assert!(resume_reserved(
            registered,
            &mut recovered,
            &mut disk,
            [0x61; 32],
            pin,
            &caps(),
            &tight
        )
        .is_err());
        assert_eq!(recovered.record([0xb0; 32], [0x61; 32]), before);
        let token = resume_reserved(
            registered,
            &mut recovered,
            &mut disk,
            [0x61; 32],
            pin,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            token.packet().input().as_bytes(),
            old_token.packet().input().as_bytes()
        );
        assert_eq!(
            token.packet().input().receipt_bytes(),
            Some(receipt.as_bytes())
        );
        assert_eq!(token.packet().input().budget_admission(), grant);
        assert!(matches!(
            recovered.start_run(&wasm, registered, &old_token),
            Err(AdmissionError::ForeignToken)
        ));
        let mut run = launch_local(
            &wasm,
            registered,
            &mut recovered,
            &token,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        let (ticket, _) = run.pending().unwrap();
        run.reply(ticket, &routing::normalized(1, "", map(vec![])))
            .unwrap();
        let mut sealed = run.into_sealed().unwrap();
        assert_eq!(
            accept_local(
                &mut restored,
                &mut recovered,
                &token,
                &mut sealed,
                &mut disk,
                &caps(),
                &Limits::default()
            )
            .unwrap()
            .revision,
            2
        );
    }
}
