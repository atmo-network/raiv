use super::super::tests::entry;
use super::*;
use ciborium::value::Value;
use raiv::position::Position;
use raiv::receipt::{Outcome, ReceiptSpec};

fn caps() -> ExecutorLimits {
    ExecutorLimits {
        wire: DecodeLimits::default(),
        max_actions: 10,
    }
}

fn encode(v: &Value) -> Vec<u8> {
    let mut b = Vec::new();
    ciborium::ser::into_writer(v, &mut b).unwrap();
    b
}

fn decode(b: &[u8]) -> Value {
    ciborium::de::from_reader(b).unwrap()
}

fn elements(v: &mut Value) -> &mut Vec<Value> {
    let Value::Array(a) = v else { panic!() };
    a
}

fn row(v: &mut Value) -> &mut Vec<Value> {
    elements(&mut elements(&mut elements(v)[4])[0])
}

fn terminal() -> Entry {
    let mut e = entry(1, 1);
    e.intents = 1;
    e.anchor.revision = u64::MAX;
    e.limits.max_context_bytes = u64::MAX;
    e.receipt = Some(
        Receipt::encode(
            &ReceiptSpec {
                action_id: e.command.action_id(),
                command_digest: raiv::identity::command_digest(
                    e.program.execution_domain,
                    &e.command,
                ),
                outcome: Outcome::Failed,
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
        .unwrap(),
    );
    e
}

fn frame(e: &Entry) -> Vec<u8> {
    wire::encode(e.anchor.agent_id, e.program, &[e], &caps()).unwrap()
}

#[test]
fn ready_submitted_terminal_exact_spans_and_full_u64_match_independent_ciborium_without_capabilities(
) {
    for state in 0..3 {
        let mut e = terminal();
        if state < 2 {
            e.receipt = None;
            e.intents = state;
        }
        let bytes = frame(&e);
        assert_eq!(encode(&decode(&bytes)), bytes);
        let data = ExecutorCheckpoint::decode(&bytes, &caps()).unwrap();
        assert_eq!(data.as_bytes(), bytes);
        assert_eq!(data.agent_id(), e.anchor.agent_id);
        assert_eq!(data.program_binding(), e.program);
        assert_eq!(data.records().next().unwrap(), e.record());
        assert_eq!(
            data.command_bytes(e.command.action_id()),
            Some(e.command.as_bytes())
        );
        assert_eq!(
            data.receipt_bytes(e.command.action_id()),
            e.receipt.as_ref().map(|r| r.as_bytes())
        );
        assert_eq!(
            data.recorded_limits(e.command.action_id())
                .unwrap()
                .max_context_bytes,
            u64::MAX
        );
        assert_eq!(data.records().next().unwrap().anchor.revision, u64::MAX);
        assert!(!bytes.windows(b"origin".len()).any(|w| w == b"origin"));
    }
}

#[test]
fn closed_frames_nested_records_state_and_id_digest_joins_reject_before_frame_ownership() {
    let original = frame(&terminal());
    for case in 0..13 {
        let mut value = decode(&original);
        match case {
            0 => elements(&mut value)[0] = Value::Text("unknown".into()),
            1 => elements(&mut value)[1] = Value::Integer(2.into()),
            2 => {
                elements(&mut value).push(Value::Null);
            }
            3 => elements(&mut value)[2] = Value::Bytes(vec![0; 31]),
            4 => row(&mut value)[0] = Value::Integer(0.into()),
            5 => row(&mut value)[1] = Value::Bytes(vec![0; 36]),
            6 => row(&mut value)[4] = Value::Integer(2.into()),
            7 => row(&mut value)[4] = Value::Integer(0.into()),
            8 => row(&mut value)[2] = Value::Bytes(vec![0xa0]),
            10 => row(&mut value)[3] = Value::Bytes(vec![0xa0]),
            11 => {
                row(&mut value)[3] = Value::Bytes(
                    Limits {
                        max_command_bytes: 1,
                        ..Limits::default()
                    }
                    .encode(&caps().wire)
                    .unwrap(),
                )
            }
            12 => elements(&mut elements(&mut value)[3])[2] = Value::Bytes(vec![0xee; 32]),
            _ => {
                let Value::Bytes(b) = &row(&mut value)[5] else {
                    panic!()
                };
                let mut r = decode(b);
                let Value::Map(fields) = &mut r else { panic!() };
                fields
                    .iter_mut()
                    .find(|(k, _)| k == &Value::Text("command_digest".into()))
                    .unwrap()
                    .1 = Value::Bytes(vec![0xee; 32]);
                row(&mut value)[5] = Value::Bytes(encode(&r));
            }
        }
        assert!(
            ExecutorCheckpoint::decode(&encode(&value), &caps()).is_err(),
            "case {case}"
        );
    }
    let mut trailing = original.clone();
    trailing.push(0);
    assert!(ExecutorCheckpoint::decode(&trailing, &caps()).is_err());
    for limits in [
        ExecutorLimits {
            max_actions: 0,
            ..caps()
        },
        ExecutorLimits {
            wire: DecodeLimits {
                max_block_bytes: 1,
                ..caps().wire
            },
            ..caps()
        },
        ExecutorLimits {
            wire: DecodeLimits {
                max_depth: 1,
                ..caps().wire
            },
            ..caps()
        },
        ExecutorLimits {
            wire: DecodeLimits {
                max_collection_entries: 1,
                ..caps().wire
            },
            ..caps()
        },
    ] {
        assert!(ExecutorCheckpoint::decode(&original, &limits).is_err());
    }
}

#[test]
fn canonical_slot_revision_uniqueness_and_only_latest_unresolved_record_are_enforced() {
    let mut a = terminal();
    a.anchor.revision = 1;
    let mut b = entry(1, 2);
    b.anchor.revision = 2;
    let bytes = wire::encode(a.anchor.agent_id, a.program, &[&a, &b], &caps()).unwrap();
    assert_eq!(
        ExecutorCheckpoint::decode(&bytes, &caps())
            .unwrap()
            .records()
            .count(),
        2
    );
    for case in 0..4 {
        let mut value = decode(&bytes);
        let rows = elements(&mut elements(&mut value)[4]);
        match case {
            0 => rows.reverse(),
            1 => rows.push(rows[0].clone()),
            2 => elements(&mut rows[1])[0] = Value::Integer(1.into()),
            _ => elements(&mut rows[0])[5] = Value::Null,
        }
        assert!(
            ExecutorCheckpoint::decode(&encode(&value), &caps()).is_err(),
            "case {case}"
        );
    }
    let mut value = decode(&bytes);
    let rows = elements(&mut elements(&mut value)[4]);
    elements(&mut rows[0])[5] = Value::Null;
    let c = Receipt::encode(
        &ReceiptSpec {
            action_id: b.command.action_id(),
            command_digest: raiv::identity::command_digest(b.program.execution_domain, &b.command),
            outcome: Outcome::Succeeded,
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
    elements(&mut rows[1])[4] = Value::Integer(1.into());
    elements(&mut rows[1])[5] = Value::Bytes(c.as_bytes().to_vec());
    assert!(ExecutorCheckpoint::decode(&encode(&value), &caps()).is_err());
}

#[test]
fn terminal_staging_borrows_exact_receipt_preserves_prior_terminal_command_limits_and_never_mutates_native_book(
) {
    use crate::simulator::tests::{entry, receipt};
    let mut book = ExecutorBook::new(3).unwrap();
    let e = entry(1, 1);
    let i = book.stage(&e).unwrap();
    let first = book.publish(i, e);
    book.begin(first).unwrap();
    let r = receipt(&book, first, Outcome::Failed);
    book.resolve(first, r).unwrap();
    let mut e = entry(1, 2);
    e.anchor.revision = 2;
    let i = book.stage(&e).unwrap();
    let second = book.publish(i, e);
    book.begin(second).unwrap();
    for outcome in [Outcome::Failed, Outcome::Succeeded, Outcome::NotExecuted] {
        let r = receipt(&book, second, outcome);
        let first_record = book.record(first).unwrap();
        let second_record = book.record(second).unwrap();
        let frame = wire::encode_change(
            book.entries[0].anchor.agent_id,
            book.entries[0].program,
            &[&book.entries[0], &book.entries[1]],
            &caps(),
            Some(Change::Terminal(second_record.action_id, &r)),
        )
        .unwrap();
        let data = ExecutorCheckpoint::decode(&frame, &caps()).unwrap();
        assert_eq!(
            data.receipt_bytes(second_record.action_id),
            Some(r.as_bytes())
        );
        assert_eq!(
            data.receipt_bytes(first_record.action_id),
            book.receipt(first).unwrap().map(|r| r.as_bytes())
        );
        assert_eq!(
            data.command_bytes(second_record.action_id),
            Some(book.entries[1].command.as_bytes())
        );
        assert_eq!(
            data.recorded_limits(first_record.action_id),
            Some(book.entries[0].limits)
        );
        assert_eq!(book.record(second).unwrap(), second_record);
        assert!(book.receipt(second).unwrap().is_none());
    }
}
