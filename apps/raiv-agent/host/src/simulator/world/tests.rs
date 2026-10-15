use super::*;
use crate::{disk::tests::Directory, simulator::tests::entry};
use ciborium::value::Value;

fn caps() -> ExecutorLimits {
    ExecutorLimits {
        wire: DecodeLimits::default(),
        max_actions: 8,
    }
}

fn decode(bytes: &[u8]) -> Value {
    ciborium::de::from_reader(bytes).unwrap()
}

fn encode(v: &Value) -> Vec<u8> {
    let mut b = Vec::new();
    ciborium::ser::into_writer(v, &mut b).unwrap();
    b
}

fn fields(v: &mut Value) -> &mut Vec<Value> {
    let Value::Array(v) = v else { panic!() };
    v
}

fn frame(status: Status, initial: u64, debit: u64, outcome: Outcome) -> WorldState {
    let mut e = entry(1, 1);
    if status != Status::Ready {
        e.intents = 1;
    }
    if status == Status::Terminal {
        e.receipt = Some(
            Receipt::encode(
                &ReceiptSpec {
                    action_id: e.command.action_id(),
                    command_digest: e.record().command_digest,
                    outcome,
                    result: &[0xa0],
                    execution_reference: None,
                    finalized_at: Position {
                        domain: [7; 32],
                        number: 1,
                        reference: None,
                    },
                    costs: &wire::costs(debit).unwrap(),
                },
                &e.limits,
            )
            .unwrap(),
        );
    }
    let ledger = ExecutorCheckpoint::test_frame(e.anchor.agent_id, e.program, &[&e], &caps(), None);
    WorldState::compose([7; 32], initial, ledger, &caps()).unwrap()
}

#[test]
fn conservation_known_debits_and_unknown_reservations_exact_oracle_full_u64() {
    for (status, debit, outcome, used) in [
        (Status::Ready, 0, Outcome::Failed, 0),
        (Status::Submitted, 0, Outcome::Failed, 1),
        (Status::Terminal, 1, Outcome::Succeeded, 1),
        (Status::Terminal, 1, Outcome::Failed, 1),
        (Status::Terminal, 0, Outcome::NotExecuted, 0),
    ] {
        let state = frame(status, u64::MAX, debit, outcome);
        let bytes = state.encode(&caps()).unwrap();
        assert_eq!(state.available_units(), u64::MAX - used);
        assert_eq!(encode(&decode(&bytes)), bytes);
        let round = WorldState::decode(&bytes, &caps()).unwrap();
        assert_eq!(round.encode(&caps()).unwrap(), bytes);
        assert_eq!(round.ledger().as_bytes(), state.ledger.as_bytes());
        let mut wrong = decode(&bytes);
        fields(&mut wrong)[4] = (u64::MAX - used - 1).into();
        assert!(WorldState::decode(&encode(&wrong), &caps()).is_err());
    }
}

#[test]
fn malformed_header_inner_state_costs_stamp_caps_and_underflow_fail_closed() {
    let state = frame(Status::Terminal, 1, 1, Outcome::Succeeded);
    let bytes = state.encode(&caps()).unwrap();
    for field in [0, 1, 2, 3, 4, 5] {
        let mut wrong = decode(&bytes);
        fields(&mut wrong)[field] = Value::Null;
        assert!(WorldState::decode(&encode(&wrong), &caps()).is_err());
    }
    let mut foreign = decode(&bytes);
    fields(&mut foreign)[2] = Value::Bytes(vec![8; 32]);
    assert!(WorldState::decode(&encode(&foreign), &caps()).is_err());
    let mut underflow = decode(&bytes);
    fields(&mut underflow)[3] = 0.into();
    assert!(WorldState::decode(&encode(&underflow), &caps()).is_err());
    for costs in [
        Value::Map(vec![]),
        Value::Map(vec![(Value::Text("virtual_units".into()), Value::Null)]),
        Value::Map(vec![(Value::Text("virtual_units".into()), 2.into())]),
    ] {
        let mut outer = decode(&bytes);
        let Value::Bytes(inner) = &mut fields(&mut outer)[5] else {
            panic!()
        };
        let mut ledger = decode(inner);
        let row = &mut fields(&mut fields(&mut fields(&mut ledger)[4])[0]);
        let Value::Bytes(receipt) = &mut row[5] else {
            panic!()
        };
        let mut r = decode(receipt);
        let Value::Map(kv) = &mut r else { panic!() };
        kv.iter_mut()
            .find(|(k, _)| *k == Value::Text("costs".into()))
            .unwrap()
            .1 = costs;
        *receipt = encode(&r);
        *inner = encode(&ledger);
        assert!(WorldState::decode(&encode(&outer), &caps()).is_err());
    }
    let tight = ExecutorLimits {
        wire: DecodeLimits {
            max_block_bytes: bytes.len() as u64 - 1,
            ..caps().wire
        },
        ..caps()
    };
    assert!(state.encode(&tight).is_err());
    assert!(WorldState::decode(&bytes, &tight).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(WorldState::decode(&extra, &caps()).is_err());
}

#[test]
fn independent_world_slot_cas_unknown_acknowledgement_and_reopen_keep_whole_effect_receipt() {
    let directory = Directory::new();
    let mut store = DiskBlockStore::open_existing(&directory.0, caps().wire).unwrap();
    let old = frame(Status::Submitted, 2, 0, Outcome::Failed)
        .encode(&caps())
        .unwrap();
    let next = frame(Status::Terminal, 2, 0, Outcome::NotExecuted)
        .encode(&caps())
        .unwrap();
    ManifestFile::world(&mut store, caps().wire)
        .replace(None, &old)
        .unwrap();
    store.fail_manifest_after_publish = true;
    assert!(matches!(
        ManifestFile::world(&mut store, caps().wire).replace(Some(&old), &next),
        Err(ManifestError::UnknownOutcome(_))
    ));
    store.fail_manifest_after_publish = false;
    assert_eq!(
        WorldState::read(&mut store, &caps())
            .unwrap()
            .available_units(),
        2
    );
    assert_eq!(
        ManifestFile::world(&mut store, caps().wire)
            .replace(Some(&old), &next)
            .unwrap(),
        Publication::AlreadyPresent
    );
    assert!(store.manifest(caps().wire).read().unwrap().is_none());
    drop(store);
    let mut store = DiskBlockStore::open_existing(&directory.0, caps().wire).unwrap();
    let state = WorldState::read(&mut store, &caps()).unwrap();
    assert_eq!(state.encode(&caps()).unwrap(), next);
    assert!(matches!(
        ManifestFile::world(&mut store, caps().wire).replace(Some(&old), &old),
        Err(ManifestError::Conflict)
    ));
    assert!(state
        .ledger()
        .records()
        .all(|r| r.status == Status::Terminal));
}
