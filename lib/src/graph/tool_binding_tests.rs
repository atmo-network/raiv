use super::*;
use crate::command::Command;

#[path = "projection_tests.rs"]
mod projection;
#[path = "restored_tests.rs"]
mod restored;

fn check_tools(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_tool_bindings()
}
fn command_at(blocks: &Store, root: DaslCid) -> (DaslCid, Vec<u8>) {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let cid = state
        .current_episode_link()
        .unwrap_or_else(|| state.history().last().unwrap());
    let episode = Episode::decode(blocks.get(cid).unwrap(), &DecodeLimits::default()).unwrap();
    let boundary_cid = episode.boundaries().next().unwrap();
    let boundary = Boundary::decode(blocks.get(boundary_cid).unwrap(), &Limits::default()).unwrap();
    let Exit::AwaitAction(command) = boundary.exit() else {
        unreachable!()
    };
    (boundary_cid, command.to_vec())
}
fn definition_change(blocks: &mut Store, root: DaslCid, edit: impl FnOnce(&mut Cbor)) -> DaslCid {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let definition = patch(blocks, state.definition_link(), edit);
    patch(blocks, root, |r| {
        map_mut(r).insert("definition".into(), link(definition));
    })
}
fn descriptor_change(
    blocks: &mut Store,
    root: DaslCid,
    id: &str,
    edit: impl FnOnce(&mut Cbor),
) -> DaslCid {
    definition_change(blocks, root, |r| {
        let Cbor::Array(tools) = map_mut(r).get_mut("tools").unwrap() else {
            unreachable!()
        };
        let tool = tools
            .iter_mut()
            .find(|tool| {
                let Cbor::Map(fields) = tool else {
                    unreachable!()
                };
                fields
                    .iter()
                    .any(|(k, v)| k == &Cbor::Text("id".into()) && v == &Cbor::Text(id.into()))
            })
            .unwrap();
        edit(tool);
    })
}
fn single_pending() -> (Store, DaslCid) {
    let fixture = &identity_graphs()[0];
    let (mut blocks, cid, episode) = episode_for(fixture);
    let first = episode.boundaries().next().unwrap();
    let boundary = Boundary::decode(blocks.get(first).unwrap(), &Limits::default()).unwrap();
    let Exit::AwaitAction(command) = boundary.exit() else {
        unreachable!()
    };
    let action = Command::action_id_in_validated(command);
    let episode = patch(&mut blocks, cid, |r| {
        map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(first)]));
    });
    let root = patch(&mut blocks, root_of(fixture), |r| {
        map_mut(r).insert("current_episode".into(), link(episode));
        map_mut(r).insert("revision".into(), Cbor::Integer(1.into()));
        map_mut(map_mut(r).get_mut("continuation").unwrap())
            .insert("action_id".into(), Cbor::Bytes(action.to_vec()));
    });
    (blocks, root)
}

#[test]
fn independent_descriptor_bound_graphs_and_genesis_keep_exact_bytes_without_new_reads() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    let role = roles();
    let actions = identity_graphs();
    let genesis = fixtures();
    for fixture in role
        .as_array()
        .unwrap()
        .iter()
        .take(1)
        .chain(actions.as_array().unwrap())
        .chain(genesis.as_array().unwrap())
    {
        let blocks = Counting(store(fixture), Cell::new(0));
        let inspected =
            InspectedClosure::inspect(root_of(fixture), &blocks, &Limits::default()).unwrap();
        let bytes = inspected
            .blocks()
            .map(|(cid, b)| (cid, b.to_vec()))
            .collect::<BTreeMap<_, _>>();
        let reads = blocks.1.get();
        assert_eq!(
            inspected.check_tool_bindings(),
            Ok(()),
            "{}",
            fixture["name"]
        );
        assert_eq!(blocks.1.get(), reads);
        assert_eq!(
            bytes,
            inspected
                .blocks()
                .map(|(cid, b)| (cid, b.to_vec()))
                .collect()
        );
    }
}

#[test]
fn historical_role_only_pending_fixtures_do_not_claim_descriptor_binding() {
    for (index, fixture) in roles().as_array().unwrap().iter().enumerate().skip(1) {
        let blocks = store(fixture);
        let (boundary, _) = command_at(&blocks, root_of(fixture));
        let error = if index == 1 {
            ToolBindingError::UnknownTool(boundary)
        } else {
            ToolBindingError::AuthorityMismatch(boundary)
        };
        assert_eq!(
            check_tools(&blocks, root_of(fixture)),
            Err(error.into()),
            "{}",
            fixture["name"]
        );
    }
}

#[test]
fn unknown_command_names_do_not_gain_whitelist_membership_and_empty_whitelists_are_not_fallbacks() {
    let fixture = &identity_graphs()[0];
    let (blocks, root, boundary) = boundary_change(fixture, 1, |r| {
        let command = map_mut(map_mut(r).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap();
        map_mut(command).insert("tool_id".into(), Cbor::Text("unlisted🐕".into()));
    });
    assert_eq!(
        check_tools(&blocks, root),
        Err(ToolBindingError::UnknownTool(boundary).into())
    );
    let mut blocks = store(fixture);
    let (first, _) = command_at(&blocks, root_of(fixture));
    let root = definition_change(&mut blocks, root_of(fixture), |r| {
        map_mut(r).insert("tools".into(), Cbor::Array(vec![]));
    });
    assert_eq!(
        check_tools(&blocks, root),
        Err(ToolBindingError::UnknownTool(first).into())
    );
}

#[test]
fn local_continue_or_finish_descriptors_cannot_describe_a_yield_command() {
    for control in ["continue", "finish"] {
        let (mut blocks, root) = single_pending();
        let (boundary, command) = command_at(&blocks, root);
        let (id, _, _) = Command::descriptor_in_validated(&command);
        let root = descriptor_change(&mut blocks, root, id, |r| {
            map_mut(r).insert("control".into(), Cbor::Text(control.into()));
            map_mut(r).insert("authority_binding".into(), Cbor::Null);
        });
        assert_eq!(
            check_tools(&blocks, root),
            Err(ToolBindingError::NotYield(boundary).into())
        );
    }
}

#[test]
fn versions_and_authority_bindings_match_exactly_without_narrowing_or_registration_claims() {
    for version in [0, u64::MAX] {
        let (mut blocks, root) = single_pending();
        let (boundary, command) = command_at(&blocks, root);
        let (id, actual, _) = Command::descriptor_in_validated(&command);
        assert_ne!(version, actual);
        let root = descriptor_change(&mut blocks, root, id, |r| {
            map_mut(r).insert("version".into(), Cbor::Integer(version.into()));
        });
        assert_eq!(
            check_tools(&blocks, root),
            Err(ToolBindingError::VersionMismatch(boundary).into())
        );
    }
    let (mut blocks, root) = single_pending();
    let (boundary, command) = command_at(&blocks, root);
    let (id, _, authority) = Command::descriptor_in_validated(&command);
    let mut wrong = authority;
    wrong[0] ^= 1;
    let root = descriptor_change(&mut blocks, root, id, |r| {
        map_mut(r).insert("authority_binding".into(), Cbor::Bytes(wrong.to_vec()));
    });
    assert_eq!(
        check_tools(&blocks, root),
        Err(ToolBindingError::AuthorityMismatch(boundary).into())
    );
}

#[test]
fn full_u64_versions_match_on_borrowed_command_bytes_and_reader_agrees_with_owned_views() {
    let (mut blocks, root) = single_pending();
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let cid = state.current_episode_link().unwrap();
    let episode = Episode::decode(blocks.get(cid).unwrap(), &DecodeLimits::default()).unwrap();
    let next = patch(&mut blocks, episode.boundaries().next().unwrap(), |r| {
        let command = map_mut(map_mut(r).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap();
        map_mut(command).insert("tool_version".into(), Cbor::Integer(u64::MAX.into()));
    });
    let episode = patch(&mut blocks, cid, |r| {
        map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(next)]));
    });
    let root = patch(&mut blocks, root, |r| {
        map_mut(r).insert("current_episode".into(), link(episode));
    });
    let (_, bytes) = command_at(&blocks, root);
    let (id, version, authority) = Command::descriptor_in_validated(&bytes);
    let owned = Command::decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(
        (id, version, authority),
        (
            owned.tool_id(),
            owned.tool_version(),
            owned.authority_binding()
        )
    );
    assert_eq!(version, u64::MAX);
    let start = bytes.as_ptr() as usize;
    let end = start + bytes.len();
    assert!((id.as_ptr() as usize) >= start && (id.as_ptr() as usize) + id.len() <= end);
    let root = descriptor_change(&mut blocks, root, id, |r| {
        map_mut(r).insert("version".into(), Cbor::Integer(u64::MAX.into()));
    });
    assert_eq!(check_tools(&blocks, root), Ok(()));
}

#[test]
fn descriptor_order_and_arguments_schema_are_not_a_generic_registry_or_application_validator() {
    let (mut blocks, root) = single_pending();
    let (_, command) = command_at(&blocks, root);
    let (id, _, _) = Command::descriptor_in_validated(&command);
    let root = descriptor_change(&mut blocks, root, id, |r| {
        // A deliberately incompatible application schema remains only a bounded model Map.
        map_mut(r).insert(
            "arguments_schema".into(),
            Cbor::Map(vec![(
                Cbor::Text("type".into()),
                Cbor::Text("string".into()),
            )]),
        );
    });
    let root = definition_change(&mut blocks, root, |r| {
        let Cbor::Array(tools) = map_mut(r).get_mut("tools").unwrap() else {
            unreachable!()
        };
        tools.reverse();
    });
    assert_eq!(check_tools(&blocks, root), Ok(())); // Equality is not schema/code validation or registration authority.
    let fixture = &identity_graphs()[1];
    let mut blocks = store(fixture);
    let (first, _) = command_at(&blocks, root_of(fixture));
    let root = definition_change(&mut blocks, root_of(fixture), |r| {
        map_mut(r).insert("tools".into(), Cbor::Array(vec![]));
    });
    assert_eq!(
        check_tools(&blocks, root),
        Err(ToolBindingError::UnknownTool(first).into())
    ); // Completed history is checked too.
}
