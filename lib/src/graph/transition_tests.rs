use super::*;
use crate::input::ActivationInput;
use crate::receipt::Receipt;

fn transitions(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_transitions()
}
pub(super) fn input_change(
    fixture: &Value,
    index: usize,
    edit: impl FnOnce(&mut Cbor),
) -> (Store, DaslCid, DaslCid) {
    let (mut blocks, cid, episode) = episode_for(fixture);
    let mut boundaries = episode.boundaries().collect::<Vec<_>>();
    let boundary =
        Boundary::decode(blocks.get(boundaries[index]).unwrap(), &Limits::default()).unwrap();
    let input = patch(&mut blocks, boundary.input_link(), edit);
    let next = patch(&mut blocks, boundaries[index], |record| {
        map_mut(record).insert("input".into(), link(input));
    });
    boundaries[index] = next;
    let ep = patch(&mut blocks, cid, |record| {
        map_mut(record).insert(
            "boundaries".into(),
            Cbor::Array(boundaries.into_iter().map(link).collect()),
        );
    });
    let root = patch(&mut blocks, root_of(fixture), |record| {
        if episode.closed() {
            map_mut(record).insert("history".into(), Cbor::Array(vec![link(ep)]));
        } else {
            map_mut(record).insert("current_episode".into(), link(ep));
        }
    });
    (blocks, root, next)
}
fn pair(fixture: &Value) -> (Vec<u8>, Vec<u8>, crate::format::Id32) {
    let (blocks, _, episode) = episode_for(fixture);
    let boundaries = episode.boundaries().collect::<Vec<_>>();
    let first = Boundary::decode(blocks.get(boundaries[0]).unwrap(), &Limits::default()).unwrap();
    let second = Boundary::decode(blocks.get(boundaries[1]).unwrap(), &Limits::default()).unwrap();
    let Exit::AwaitAction(command) = first.exit() else {
        unreachable!()
    };
    let input =
        ActivationInput::decode(blocks.get(second.input_link()).unwrap(), &Limits::default())
            .unwrap();
    let state = AgentState::decode(
        blocks.get(root_of(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let definition = Definition::decode(
        blocks.get(state.definition_link()).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    (
        command.to_vec(),
        input.receipt_bytes().unwrap().to_vec(),
        definition.execution_domain(),
    )
}

#[test]
fn complete_role_identity_and_genesis_graphs_pair_without_new_reads_or_payload_changes() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    let base = roles();
    let action = identity_graphs();
    let genesis = fixtures();
    for fixture in base
        .as_array()
        .unwrap()
        .iter()
        .chain(action.as_array().unwrap())
        .chain(genesis.as_array().unwrap())
    {
        let blocks = Counting(store(fixture), Cell::new(0));
        let inspected =
            InspectedClosure::inspect(root_of(fixture), &blocks, &Limits::default()).unwrap();
        let before = inspected
            .blocks()
            .map(|(cid, bytes)| (cid, bytes.to_vec()))
            .collect::<BTreeMap<_, _>>();
        let reads = blocks.1.get();
        assert_eq!(inspected.check_transitions(), Ok(()));
        assert_eq!(blocks.1.get(), reads);
        assert_eq!(
            before,
            inspected
                .blocks()
                .map(|(cid, bytes)| (cid, bytes.to_vec()))
                .collect()
        );
    }
}

#[test]
fn ordinals_start_at_zero_and_remain_contiguous_through_action_and_final_actor_boundaries() {
    for (data, index, ordinal) in [
        (roles()[0].clone(), 0, 1),
        (identity_graphs()[0].clone(), 0, 7),
        (identity_graphs()[0].clone(), 1, 3),
        (identity_graphs()[0].clone(), 1, u64::MAX),
        (identity_graphs()[1].clone(), 2, 4),
    ] {
        let (blocks, root, cid) = boundary_change(&data, index, |record| {
            map_mut(record).insert("ordinal".into(), Cbor::Integer(ordinal.into()));
        });
        assert_eq!(identities(&blocks, root), Ok(()));
        assert_eq!(
            transitions(&blocks, root),
            Err(TransitionError::OrdinalMismatch(cid).into())
        );
    }
}

#[test]
fn first_actor_wake_and_subsequent_action_result_are_not_interchangeable() {
    let fixture = &identity_graphs()[0];
    let (_, receipt, _) = pair(fixture);
    let receipt: Cbor = ciborium::de::from_reader(receipt.as_slice()).unwrap();
    let (blocks, root, cid) = input_change(fixture, 0, |record| {
        map_mut(record).insert("cause".into(), Cbor::Text("action_result".into()));
        map_mut(record).insert("receipt".into(), receipt);
    });
    assert_eq!(identities(&blocks, root), Ok(()));
    assert_eq!(
        transitions(&blocks, root),
        Err(TransitionError::FirstInputCause(cid).into())
    );
    let (blocks, root, cid) = input_change(fixture, 1, |record| {
        map_mut(record).insert("cause".into(), Cbor::Text("actor_wake".into()));
        map_mut(record).insert("receipt".into(), Cbor::Null);
    });
    assert_eq!(identities(&blocks, root), Ok(()));
    assert_eq!(
        transitions(&blocks, root),
        Err(TransitionError::SubsequentInputCause(cid).into())
    );
}

#[test]
fn receipt_id_digest_domain_and_plain_record_hash_are_not_substitutable() {
    let fixture = &identity_graphs()[0];
    let (command, _, _) = pair(fixture);
    let plain = Sha256::digest(&command).to_vec();
    let tree: Cbor = ciborium::de::from_reader(command.as_slice()).unwrap();
    let wrong_domain = Sha256::digest(encode(&Cbor::Array(vec![
        Cbor::Text("raiv/command/1".into()),
        Cbor::Bytes(vec![99; 32]),
        tree,
    ])))
    .to_vec();
    for (key, value) in [
        ("action_id", vec![48; 32]),
        ("command_digest", vec![49; 32]),
        ("command_digest", plain),
        ("command_digest", wrong_domain),
    ] {
        let (blocks, root, cid) = input_change(fixture, 1, |record| {
            map_mut(map_mut(record).get_mut("receipt").unwrap())
                .insert(key.into(), Cbor::Bytes(value));
        });
        assert_eq!(identities(&blocks, root), Ok(()));
        assert_eq!(
            transitions(&blocks, root),
            Err(TransitionError::ReceiptMismatch(cid).into())
        );
    }
}

#[test]
fn receipt_for_a_competing_same_slot_command_cannot_pair_with_changed_exact_arguments() {
    let fixture = &identity_graphs()[0];
    let (_, _, episode) = episode_for(fixture);
    let receipt_boundary = episode.boundaries().nth(1).unwrap();
    let (blocks, root, _) = boundary_change(fixture, 0, |record| {
        let command = map_mut(map_mut(record).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap();
        map_mut(map_mut(command).get_mut("arguments").unwrap())
            .insert("candidate".into(), Cbor::Text("competing".into()));
    });
    assert_eq!(identities(&blocks, root), Ok(()));
    assert_eq!(
        transitions(&blocks, root),
        Err(TransitionError::ReceiptMismatch(receipt_boundary).into())
    );
}

#[test]
fn independent_command_domain_preimage_and_all_terminal_outcomes_match_borrowed_and_owned_paths() {
    for fixture in identity_graphs().as_array().unwrap() {
        let (command, receipt, domain) = pair(fixture);
        let tree: Cbor = ciborium::de::from_reader(command.as_slice()).unwrap();
        let expected: crate::format::Id32 = Sha256::digest(encode(&Cbor::Array(vec![
            Cbor::Text("raiv/command/1".into()),
            Cbor::Bytes(domain.to_vec()),
            tree,
        ])))
        .into();
        assert_eq!(
            crate::identity::command_digest_in_validated(domain, &command),
            expected
        );
        let owned = crate::command::Command::decode(&command, &Limits::default()).unwrap();
        assert_eq!(crate::identity::command_digest(domain, &owned), expected);
        assert_eq!(
            Receipt::decode(&receipt, &Limits::default())
                .unwrap()
                .command_digest(),
            expected
        );
        for outcome in ["succeeded", "failed", "not_executed"] {
            let (blocks, root, _) = input_change(fixture, 1, |record| {
                map_mut(map_mut(record).get_mut("receipt").unwrap())
                    .insert("outcome".into(), Cbor::Text(outcome.into()));
            });
            assert_eq!(transitions(&blocks, root), Ok(()));
            let mut tree: Cbor = ciborium::de::from_reader(receipt.as_slice()).unwrap();
            map_mut(&mut tree).insert("outcome".into(), Cbor::Text(outcome.into()));
            let bytes = encode(&tree);
            assert!(Receipt::matches_in_validated(&bytes, domain, &command));
            assert!(Receipt::decode(&bytes, &Limits::default())
                .unwrap()
                .matches_command(domain, &owned));
        }
    }
}
