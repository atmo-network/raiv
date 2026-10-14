use super::*;
use crate::{
    format::ModelValue,
    projection::{HistoryProjection, ProjectionError},
};
use alloc::string::{String, ToString};
use core::fmt::Write;

pub(super) fn text_id(value: &Cbor) -> Cbor {
    let Cbor::Bytes(bytes) = value else {
        panic!("runtime ID")
    };
    let mut text = String::new();
    for byte in bytes {
        write!(&mut text, "{byte:02x}").unwrap();
    }
    Cbor::Text(text)
}
pub(super) fn decimal(value: &Cbor) -> Cbor {
    let Cbor::Integer(number) = value else {
        panic!("u64")
    };
    Cbor::Text(u64::try_from(*number).unwrap().to_string())
}
pub(super) fn normalized_receipt(value: &Cbor) -> Cbor {
    let mut result = value.clone();
    for name in ["action_id", "command_digest"] {
        map_mut(&mut result).insert(name.into(), text_id(field(value, name)));
    }
    let original = field(value, "finalized_at");
    let mut position = original.clone();
    map_mut(&mut position).insert("domain".into(), text_id(field(original, "domain")));
    map_mut(&mut position).insert("number".into(), decimal(field(original, "number")));
    if !matches!(field(original, "reference"), Cbor::Null) {
        map_mut(&mut position).insert("reference".into(), text_id(field(original, "reference")));
    }
    map_mut(&mut result).insert("finalized_at".into(), position);
    result
}
fn normalized_episode(snapshot: &RestoredAgent, episode: DaslCid) -> Cbor {
    let blocks = snapshot.blocks().collect::<BTreeMap<_, _>>();
    let episode = cbor(blocks[&episode]);
    let Cbor::Array(boundaries) = field(&episode, "boundaries") else {
        panic!("boundaries")
    };
    let memory = |value: &Cbor| {
        cid(value)
            .map(|cid| field(&cbor(blocks[&cid]), "value").clone())
            .unwrap_or(Cbor::Null)
    };
    let mut actions = Vec::new();
    for (index, boundary) in boundaries.iter().enumerate() {
        let boundary = cbor(blocks[&cid(boundary).unwrap()]);
        let exit = field(&boundary, "exit");
        if field(exit, "tag") != &Cbor::Text("await_action".into()) {
            continue;
        }
        let source = field(exit, "command");
        let mut command = source.clone();
        for name in ["action_id", "authority_binding"] {
            map_mut(&mut command).insert(name.into(), text_id(field(source, name)));
        }
        map_mut(&mut command).insert(
            "tool_version".into(),
            decimal(field(source, "tool_version")),
        );
        let receipt = boundaries
            .get(index + 1)
            .map(|next| {
                let next = cbor(blocks[&cid(next).unwrap()]);
                let input = cbor(blocks[&cid(field(&next, "input")).unwrap()]);
                normalized_receipt(field(&input, "receipt"))
            })
            .unwrap_or(Cbor::Null);
        map_mut(&mut command).insert("receipt".into(), receipt);
        actions.push(command);
    }
    let last = cbor(blocks[&cid(boundaries.last().unwrap()).unwrap()]);
    Cbor::Map(vec![
        (Cbor::Text("id".into()), text_id(field(&episode, "id"))),
        (
            Cbor::Text("closed".into()),
            field(&episode, "closed").clone(),
        ),
        (Cbor::Text("actions".into()), Cbor::Array(actions)),
        (
            Cbor::Text("entry_memory".into()),
            memory(field(&episode, "entry_memory")),
        ),
        (
            Cbor::Text("latest_memory".into()),
            memory(field(&last, "memory")),
        ),
    ])
}
pub(super) fn oracle(snapshot: &RestoredAgent) -> Cbor {
    let state = cbor(snapshot.state().as_bytes());
    let Cbor::Array(history) = field(&state, "history") else {
        panic!("history")
    };
    let history = if snapshot.definition().limits().history_episodes == 0 {
        Vec::new()
    } else {
        history
            .iter()
            .map(|episode| normalized_episode(snapshot, cid(episode).unwrap()))
            .collect()
    };
    let current = cid(field(&state, "current_episode"))
        .map(|cid| normalized_episode(snapshot, cid))
        .unwrap_or(Cbor::Null);
    Cbor::Array(vec![
        Cbor::Map(vec![
            (
                Cbor::Text("kind".into()),
                Cbor::Text("completed_history".into()),
            ),
            (Cbor::Text("episodes".into()), Cbor::Array(history)),
        ]),
        Cbor::Map(vec![
            (
                Cbor::Text("kind".into()),
                Cbor::Text("current_chain".into()),
            ),
            (Cbor::Text("episode".into()), current),
        ]),
    ])
}
fn model_depth(value: &Cbor) -> u64 {
    match value {
        Cbor::Array(values) => 1 + values.iter().map(model_depth).max().unwrap_or(0),
        Cbor::Map(values) => {
            1 + values
                .iter()
                .map(|(key, value)| model_depth(key).max(model_depth(value)))
                .max()
                .unwrap_or(0)
        }
        _ => 1,
    }
}
fn encoded(snapshot: &RestoredAgent, limits: &Limits) -> Result<ModelValue, ProjectionError> {
    HistoryProjection::new(snapshot).encode(limits)
}

#[test]
fn complete_fragment_bytes_match_independent_ciborium_normalization_and_section_order() {
    let mut snapshots = restored::valid_snapshots();
    snapshots.push(super::super::super::history::completed_pair());
    for (blocks, root) in snapshots {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let result = encoded(&snapshot, &Limits::default()).unwrap();
        assert_eq!(result.as_bytes(), encode(&oracle(&snapshot)));
        crate::format::validate_model_value(
            result.as_bytes(),
            &DecodeLimits {
                max_block_bytes: 131072,
                ..DecodeLimits::default()
            },
        )
        .unwrap();
    }
}

#[test]
fn caller_and_definition_context_caps_bound_the_whole_fragment_without_window_truncation() {
    let fixture = &identity_graphs()[1];
    let mut blocks = store(fixture);
    let root = root_of(fixture);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let full = encoded(&snapshot, &Limits::default()).unwrap();
    let size = full.as_bytes().len() as u64;
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_context_bytes: size,
                ..Limits::default()
            }
        )
        .unwrap(),
        full
    );
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_context_bytes: size - 1,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::ContextLimit)
    );
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_context_bytes: 0,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::ContextLimit)
    );
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_block_bytes: 1,
                ..Limits::default()
            }
        )
        .unwrap(),
        full
    ); // Transient model data is not a structural block.
    let next = definition_change(&mut blocks, root, |r| {
        map_mut(map_mut(r).get_mut("limits").unwrap())
            .insert("max_context_bytes".into(), Cbor::Integer((size - 1).into()));
    });
    let narrow = RestoredAgent::restore(next, &blocks, &Limits::default()).unwrap();
    assert_eq!(
        encoded(
            &narrow,
            &Limits {
                max_context_bytes: u64::MAX,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::ContextLimit)
    );
    assert_eq!(narrow.state().history_count(), 1);
    assert_eq!(snapshot.state().history_count(), 1);
}

#[test]
fn zero_history_policy_keeps_current_pending_null_and_memory_none_not_empty_objects() {
    for (mut blocks, root) in restored::valid_snapshots() {
        let root = definition_change(&mut blocks, root, |r| {
            map_mut(map_mut(r).get_mut("limits").unwrap())
                .insert("history_episodes".into(), Cbor::Integer(0.into()));
        });
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        assert_eq!(
            encoded(&snapshot, &Limits::default()).unwrap().as_bytes(),
            encode(&oracle(&snapshot))
        );
    }
}

#[test]
fn contextual_depth_and_count_caps_reject_coupled_records_and_deep_model_maps() {
    let fixture = &identity_graphs()[1];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let depth = model_depth(&oracle(&snapshot));
    assert!(encoded(
        &snapshot,
        &Limits {
            max_depth: depth,
            ..Limits::default()
        }
    )
    .is_ok());
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_depth: depth - 1,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::Codec(CodecError::DepthLimit))
    );
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_collection_entries: 6,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::Codec(CodecError::CollectionLimit))
    );
    let (blocks, root, _) = super::super::super::transitions::input_change(fixture, 1, |r| {
        let mut nested = Cbor::Null;
        for _ in 0..26 {
            nested = Cbor::Array(vec![nested]);
        }
        map_mut(map_mut(r).get_mut("receipt").unwrap()).insert(
            "result".into(),
            Cbor::Map(vec![(Cbor::Text("deep".into()), nested)]),
        );
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    assert_eq!(
        encoded(&snapshot, &Limits::default()),
        Err(ProjectionError::Codec(CodecError::DepthLimit))
    );
}

#[test]
fn full_u64_metadata_and_terminal_outcomes_normalize_losslessly_without_float_or_runtime_bytes() {
    let fixture = &identity_graphs()[0];
    for version in [0, 23, 24, 255, 256, u64::MAX] {
        let (mut blocks, episode_cid, episode) = episode_for(fixture);
        let old = episode.boundaries().collect::<Vec<_>>();
        let update = |r: &mut Cbor| {
            let command = map_mut(map_mut(r).get_mut("exit").unwrap())
                .get_mut("command")
                .unwrap();
            map_mut(command).insert("tool_version".into(), Cbor::Integer(version.into()));
        };
        let first = patch(&mut blocks, old[0], update);
        let first_record = cbor(blocks.get(first).unwrap());
        let command = field(field(&first_record, "exit"), "command");
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
        let digest = Sha256::digest(encode(&Cbor::Array(vec![
            Cbor::Text("raiv/command/1".into()),
            Cbor::Bytes(definition.execution_domain().to_vec()),
            command.clone(),
        ])))
        .to_vec();
        let second_record = cbor(blocks.get(old[1]).unwrap());
        let input = patch(
            &mut blocks,
            cid(field(&second_record, "input")).unwrap(),
            |r| {
                map_mut(map_mut(r).get_mut("receipt").unwrap())
                    .insert("command_digest".into(), Cbor::Bytes(digest));
            },
        );
        let second = patch(&mut blocks, old[1], |r| {
            update(r);
            map_mut(r).insert("input".into(), link(input));
        });
        let next = patch(&mut blocks, episode_cid, |r| {
            map_mut(r).insert(
                "boundaries".into(),
                Cbor::Array(vec![link(first), link(second)]),
            );
        });
        let root = patch(&mut blocks, root_of(fixture), |r| {
            map_mut(r).insert("current_episode".into(), link(next));
        });
        let Cbor::Text(id) = field(command, "tool_id") else {
            panic!("tool")
        };
        let root = descriptor_change(&mut blocks, root, id, |r| {
            map_mut(r).insert("version".into(), Cbor::Integer(version.into()));
        });
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        assert_eq!(
            encoded(&snapshot, &Limits::default()).unwrap().as_bytes(),
            encode(&oracle(&snapshot))
        );
    }
    for outcome in ["succeeded", "failed", "not_executed"] {
        let (blocks, root, _) = super::super::super::transitions::input_change(fixture, 1, |r| {
            let receipt = map_mut(r).get_mut("receipt").unwrap();
            map_mut(receipt).insert("outcome".into(), Cbor::Text(outcome.into()));
            map_mut(receipt).insert("execution_reference".into(), Cbor::Text("".into()));
            map_mut(map_mut(receipt).get_mut("finalized_at").unwrap())
                .insert("number".into(), Cbor::Integer(u64::MAX.into()));
        });
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        assert_eq!(
            encoded(&snapshot, &Limits::default()).unwrap().as_bytes(),
            encode(&oracle(&snapshot))
        );
    }
}

#[test]
fn rejected_encoding_preserves_the_snapshot_and_fresh_runs_produce_identical_fragments_without_store_io(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let before = snapshot
        .blocks()
        .map(|(cid, b)| (cid, (b.to_vec(), b.as_ptr())))
        .collect::<BTreeMap<_, _>>();
    let first = encoded(&snapshot, &Limits::default()).unwrap();
    assert_eq!(
        encoded(
            &snapshot,
            &Limits {
                max_context_bytes: 0,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::ContextLimit)
    );
    assert_eq!(encoded(&snapshot, &Limits::default()).unwrap(), first);
    assert_eq!(
        snapshot
            .blocks()
            .map(|(cid, b)| (cid, (b.to_vec(), b.as_ptr())))
            .collect::<BTreeMap<_, _>>(),
        before
    );
}
