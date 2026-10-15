use super::*;

#[path = "history_tests.rs"]
mod history;
#[path = "memory_tests.rs"]
mod memories;
#[path = "tool_binding_tests.rs"]
mod tool_bindings;
#[path = "transition_tests.rs"]
mod transitions;
#[path = "usage_tests.rs"]
mod usage;

use crate::boundary::{Boundary, Exit};
use sha2::{Digest, Sha256};

fn identity_graphs() -> Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/format/identity_graphs.json"
    ))
    .unwrap()
}

fn root_of(fixture: &Value) -> DaslCid {
    fixture["root"].as_str().unwrap().parse().unwrap()
}

fn identities(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_identities()
}

fn boundary_change(
    fixture: &Value,
    index: usize,
    edit: impl FnOnce(&mut Cbor),
) -> (Store, DaslCid, DaslCid) {
    let (mut blocks, cid, episode) = episode_for(fixture);
    let mut boundaries = episode.boundaries().collect::<Vec<_>>();
    let next = patch(&mut blocks, boundaries[index], edit);
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

#[test]
fn role_and_two_action_identity_graphs_and_genesis_bind_without_io_or_byte_rewrites() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    let data = roles();
    let actions = identity_graphs();
    let genesis = fixtures();
    for fixture in data
        .as_array()
        .unwrap()
        .iter()
        .chain(actions.as_array().unwrap())
        .chain(genesis.as_array().unwrap())
    {
        let blocks = Counting(store(fixture), Cell::new(0));
        let inspected =
            InspectedClosure::inspect(root_of(fixture), &blocks, &Limits::default()).unwrap();
        let before = inspected
            .blocks()
            .map(|(cid, bytes)| (cid, bytes.to_vec()))
            .collect::<BTreeMap<_, _>>();
        let calls = blocks.1.get();
        assert_eq!(inspected.check_identities(), Ok(()));
        assert_eq!(blocks.1.get(), calls);
        assert_eq!(
            before,
            inspected
                .blocks()
                .map(|(cid, bytes)| (cid, bytes.to_vec()))
                .collect()
        );
        assert_eq!(
            inspected.encoded_bytes(),
            fixture["closure_bytes"].as_u64().unwrap()
        );
    }
}

#[test]
fn complete_graph_identity_preimages_match_independent_ciborium_and_hashlib_witnesses() {
    for fixture in identity_graphs().as_array().unwrap() {
        let blocks = store(fixture);
        for bytes in blocks.0.values() {
            let tree: Cbor = ciborium::de::from_reader(bytes.as_slice()).unwrap();
            assert_eq!(encode(&tree), *bytes);
        }
        let inspected =
            InspectedClosure::inspect(root_of(fixture), &blocks, &Limits::default()).unwrap();
        let (_, _, episode) = episode_for(fixture);
        let domain = inspected.definition().execution_domain();
        let agent = inspected.state().agent_id();
        let preimage = encode(&Cbor::Array(vec![
            Cbor::Text("raiv/episode/1".into()),
            Cbor::Bytes(domain.to_vec()),
            Cbor::Bytes(agent.to_vec()),
            Cbor::Bytes(episode.trigger_event_id().to_vec()),
        ]));
        assert_eq!(
            preimage,
            hex(fixture["episode_witness"]["preimage"].as_str().unwrap())
        );
        assert_eq!(
            &Sha256::digest(&preimage)[..],
            hex(fixture["episode_witness"]["digest"].as_str().unwrap())
        );
        assert_eq!(
            episode.id().to_vec(),
            hex(fixture["episode_witness"]["digest"].as_str().unwrap())
        );
        let mut action_index = 0;
        for cid in episode.boundaries() {
            let boundary = Boundary::decode(blocks.get(cid).unwrap(), &Limits::default()).unwrap();
            if let Exit::AwaitAction(command) = boundary.exit() {
                let preimage = encode(&Cbor::Array(vec![
                    Cbor::Text("raiv/action/1".into()),
                    Cbor::Bytes(domain.to_vec()),
                    Cbor::Bytes(agent.to_vec()),
                    Cbor::Bytes(episode.id().to_vec()),
                    Cbor::Integer((action_index as u64).into()),
                ]));
                let witness = &fixture["action_witnesses"][action_index];
                assert_eq!(preimage, hex(witness["preimage"].as_str().unwrap()));
                assert_eq!(
                    &Sha256::digest(&preimage)[..],
                    hex(witness["digest"].as_str().unwrap())
                );
                assert_eq!(
                    crate::command::Command::action_id_in_validated(command).to_vec(),
                    hex(witness["digest"].as_str().unwrap())
                );
                action_index += 1;
            }
        }
        assert_eq!(action_index, 2);
    }
}

#[test]
fn episode_trigger_id_agent_and_domain_bindings_reject_hash_valid_changes() {
    let fixture = &roles()[1];
    let (blocks, root, cid) = episode_change(fixture, |record| {
        map_mut(record).insert("trigger_event_id".into(), Cbor::Bytes(vec![42; 32]));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(IdentityError::TriggerEventMismatch(cid).into())
    );
    let (blocks, root, cid) = episode_change(fixture, |record| {
        map_mut(record).insert("id".into(), Cbor::Bytes(vec![43; 32]));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(IdentityError::EpisodeIdMismatch(cid).into())
    );
    let (mut blocks, cid, _) = episode_for(fixture);
    let root = patch(&mut blocks, root_of(fixture), |record| {
        map_mut(record).insert("agent_id".into(), Cbor::Bytes(vec![44; 32]));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(IdentityError::EpisodeIdMismatch(cid).into())
    );
    let state = AgentState::decode(
        blocks.get(root_of(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let definition = patch(&mut blocks, state.definition_link(), |record| {
        map_mut(record).insert("execution_domain".into(), Cbor::Bytes(vec![45; 32]));
    });
    let root = patch(&mut blocks, root_of(fixture), |record| {
        map_mut(record).insert("definition".into(), link(definition));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(IdentityError::EpisodeIdMismatch(cid).into())
    );
}

#[test]
fn every_command_slot_and_root_pending_id_is_bound_to_the_final_command() {
    for index in [0, 1] {
        let fixture = &identity_graphs()[0];
        let (blocks, root, cid) = boundary_change(fixture, index, |record| {
            let command = map_mut(map_mut(record).get_mut("exit").unwrap())
                .get_mut("command")
                .unwrap();
            map_mut(command).insert("action_id".into(), Cbor::Bytes(vec![46; 32]));
        });
        assert_eq!(
            identities(&blocks, root),
            Err(IdentityError::ActionIdMismatch(cid).into())
        );
    }
    let fixture = &identity_graphs()[0];
    let (mut blocks, _, episode) = episode_for(fixture);
    let tail = episode.boundaries().last().unwrap();
    let root = patch(&mut blocks, root_of(fixture), |record| {
        map_mut(map_mut(record).get_mut("continuation").unwrap())
            .insert("action_id".into(), Cbor::Bytes(vec![47; 32]));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(IdentityError::PendingActionMismatch(tail).into())
    );
}

#[test]
fn action_index_is_not_the_boundary_ordinal_and_does_not_reset_each_boundary() {
    let fixture = &identity_graphs()[0];
    let (blocks, root, _) = boundary_change(fixture, 1, |record| {
        map_mut(record).insert("ordinal".into(), Cbor::Integer(u64::MAX.into()));
    });
    assert_eq!(identities(&blocks, root), Ok(())); // Contiguous ordinal validation is a different obligation.
    let (_, _, episode) = episode_for(fixture);
    let source = store(fixture);
    let first = Boundary::decode(
        source.get(episode.boundaries().next().unwrap()).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    let Exit::AwaitAction(first_command) = first.exit() else {
        unreachable!()
    };
    let first_id = crate::command::Command::action_id_in_validated(first_command);
    let (blocks, root, cid) = boundary_change(fixture, 1, |record| {
        let command = map_mut(map_mut(record).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap();
        map_mut(command).insert("action_id".into(), Cbor::Bytes(first_id.to_vec()));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(IdentityError::ActionIdMismatch(cid).into())
    );
}

#[test]
fn identities_first_require_roles_so_empty_or_bad_phase_graphs_do_not_panic() {
    let fixture = &roles()[1];
    let (blocks, root, cid) = episode_change(fixture, |record| {
        map_mut(record).insert("boundaries".into(), Cbor::Array(vec![]));
    });
    assert_eq!(
        identities(&blocks, root),
        Err(EpisodeRoleError::EmptyEpisode(cid).into())
    );
}
