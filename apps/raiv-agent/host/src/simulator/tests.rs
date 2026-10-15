//! Private FSM fixtures are command DATA; public approval/readback is exercised in Wasmi tests.

use super::*;
use raiv::{
    command::CommandSpec,
    format::{DaslCid, DaslCodec},
    position::Position,
    receipt::{Outcome, ReceiptSpec},
};

pub(super) fn entry(agent: u8, action: u8) -> Entry {
    Entry {
        anchor: Anchor {
            agent_id: [agent; 32],
            revision: 1,
            root: DaslCid::for_block(DaslCodec::Drisl, &[0xa0]),
        },
        program: ProgramBinding {
            module_digest: [8; 32],
            definition: DaslCid::for_block(DaslCodec::Drisl, &[0xa0]),
            execution_domain: [5; 32],
        },
        command: Command::encode(
            &CommandSpec {
                action_id: [action; 32],
                tool_id: "trade",
                tool_version: 1,
                authority_binding: [0x33; 32],
                arguments: &[0xa0],
            },
            &Limits::default(),
        )
        .unwrap(),
        limits: Limits::default(),
        intents: 0,
        receipt: None,
    }
}

fn ticket(book: &mut ExecutorBook, entry: Entry) -> ActionTicket {
    let index = book.stage(&entry).unwrap();
    book.publish(index, entry)
}

pub(super) fn receipt(book: &ExecutorBook, ticket: ActionTicket, outcome: Outcome) -> Receipt {
    let record = book.record(ticket).unwrap();
    Receipt::encode(
        &ReceiptSpec {
            action_id: record.action_id,
            command_digest: record.command_digest,
            outcome,
            result: &[0xa0],
            execution_reference: None,
            finalized_at: Position {
                domain: [5; 32],
                number: u64::MAX,
                reference: None,
            },
            costs: &[0xa0],
        },
        &Limits::default(),
    )
    .unwrap()
}

#[test]
fn one_correlated_intent_unknown_wait_and_no_terminal_before_submission_are_atomic() {
    assert!(matches!(
        ExecutorBook::new(0),
        Err(SimulatorError::Capacity)
    ));
    let mut book = ExecutorBook::new(3).unwrap();
    let action = ticket(&mut book, entry(1, 1));
    let ready = book.record(action).unwrap();
    let value = receipt(&book, action, Outcome::Failed);
    assert!(matches!(
        book.resolve(action, value),
        Err(SimulatorError::NotSubmitted)
    ));
    assert_eq!(book.record(action).unwrap(), ready);
    let mut foreign = ExecutorBook::new(3).unwrap();
    assert!(matches!(
        foreign.begin(action),
        Err(SimulatorError::ForeignTicket)
    ));
    assert!(foreign.record(action).is_err());
    assert!(foreign.is_empty());
    let original = entry(1, 1).command.as_bytes().to_vec();
    assert_eq!(book.begin(action).unwrap().as_bytes(), original);
    let submitted = book.record(action).unwrap();
    assert_eq!(submitted.status, Status::Submitted);
    assert_eq!(submitted.submission_intents, 1);
    assert!(book.receipt(action).unwrap().is_none());
    assert!(matches!(
        book.begin(action),
        Err(SimulatorError::AlreadySubmitted)
    ));
    let next = entry(1, 2);
    assert!(matches!(book.stage(&next), Err(SimulatorError::Busy)));
    // Build a valid terminal Receipt for another action, never invent failure from timeout.
    let wrong = Receipt::encode(
        &ReceiptSpec {
            action_id: [2; 32],
            command_digest: submitted.command_digest,
            outcome: Outcome::NotExecuted,
            result: &[0xa0],
            execution_reference: None,
            finalized_at: Position {
                domain: [5; 32],
                number: 0,
                reference: None,
            },
            costs: &[0xa0],
        },
        &Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        book.resolve(action, wrong),
        Err(SimulatorError::ReceiptMismatch)
    ));
    assert_eq!(book.record(action).unwrap(), submitted);
    assert_eq!(book.len(), 1);
}

#[test]
fn every_terminal_outcome_is_immutable_exact_duplicate_known_and_never_resubmitted() {
    for outcome in [Outcome::Succeeded, Outcome::Failed, Outcome::NotExecuted] {
        let mut book = ExecutorBook::new(2).unwrap();
        let action = ticket(&mut book, entry(1, 1));
        book.begin(action).unwrap();
        let value = receipt(&book, action, outcome);
        let exact = value.as_bytes().to_vec();
        let terminal = book.resolve(action, value).unwrap();
        assert_eq!(terminal.status, Status::Terminal);
        assert_eq!(terminal.submission_intents, 1);
        let duplicate = Receipt::decode(&exact, &Limits::default()).unwrap();
        assert_eq!(book.resolve(action, duplicate).unwrap(), terminal);
        let conflict = receipt(
            &book,
            action,
            if outcome == Outcome::Succeeded {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            },
        );
        assert!(matches!(
            book.resolve(action, conflict),
            Err(SimulatorError::TerminalConflict)
        ));
        assert_eq!(book.receipt(action).unwrap().unwrap().as_bytes(), exact);
        assert!(matches!(
            book.begin(action),
            Err(SimulatorError::AlreadySubmitted)
        ));
        assert_eq!(ticket(&mut book, entry(1, 1)), action);
        let second = ticket(&mut book, entry(1, 2));
        assert_eq!(book.record(second).unwrap().status, Status::Ready);
        assert_eq!(book.record(action).unwrap(), terminal);
    }
}

#[test]
fn slot_anchor_program_command_policy_conflicts_capacity_and_receipt_limits_preserve_state() {
    let mut book = ExecutorBook::new(1).unwrap();
    let action = ticket(&mut book, entry(1, 1));
    for case in 0..4 {
        let mut changed = entry(1, 1);
        match case {
            0 => changed.anchor.revision = 2,
            1 => changed.program.module_digest = [9; 32],
            2 => {
                changed.command = Command::encode(
                    &CommandSpec {
                        action_id: [1; 32],
                        tool_id: "set_wake",
                        tool_version: 1,
                        authority_binding: [0x44; 32],
                        arguments: &[0xa0],
                    },
                    &Limits::default(),
                )
                .unwrap()
            }
            _ => changed.limits.max_receipt_bytes = 1,
        };
        assert!(matches!(book.stage(&changed), Err(SimulatorError::Binding)));
        assert_eq!(book.record(action).unwrap().status, Status::Ready);
    }
    assert!(matches!(
        book.stage(&entry(2, 2)),
        Err(SimulatorError::Capacity)
    ));
    book.begin(action).unwrap();
    let original = receipt(&book, action, Outcome::Succeeded);
    let big = Receipt::encode(
        &ReceiptSpec {
            action_id: original.action_id(),
            command_digest: original.command_digest(),
            outcome: Outcome::Succeeded,
            result: &[0xa0],
            execution_reference: Some(&"x".repeat(5000)),
            finalized_at: original.finalized_at(),
            costs: &[0xa0],
        },
        &Limits {
            max_receipt_bytes: 10_000,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        book.resolve(action, big),
        Err(SimulatorError::Receipt(_))
    ));
    assert_eq!(book.record(action).unwrap().status, Status::Submitted);
    assert!(book.receipt(action).unwrap().is_none());
}

#[test]
fn correlation_and_submitted_checks_precede_current_phase_and_missing_file_without_resetting_intent(
) {
    use crate::{
        admission::AdmissionBook,
        checkpoint::ManifestLimits,
        disk::{tests::Directory, DiskBlockStore},
        tests::activation::{admission, output},
    };
    use raiv::format::DecodeLimits;
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let admissions = AdmissionBook::new(2).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let caps = ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    };
    let mut book = ExecutorBook::new(2).unwrap();
    let ticket = ticket(&mut book, entry(0xb0, 1));
    let ready = book.record(ticket).unwrap();
    assert!(matches!(
        book.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps,
            &Limits::default()
        ),
        Err(SimulatorError::WrongPhase)
    ));
    assert_eq!(book.record(ticket).unwrap(), ready);
    book.begin(ticket).unwrap();
    let submitted = book.record(ticket).unwrap();
    assert!(matches!(
        book.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps,
            &Limits::default()
        ),
        Err(SimulatorError::AlreadySubmitted)
    ));
    assert_eq!(book.record(ticket).unwrap(), submitted);
    assert!(disk.manifest(caps.wire).read().unwrap().is_none());
}
