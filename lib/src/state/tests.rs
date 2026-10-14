use super::*;
use crate::format::oracle::{encode as bytes, fixture_value as node, link, map_mut as map};
use alloc::{string::ToString, vec};
use ciborium::value::Value as Cbor;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/state.json")).unwrap()
}
fn record(index: usize) -> Cbor {
    node(&fixtures()[index]["record"])
}
fn spec<'a>(state: &AgentState, history: &'a [DaslCid]) -> AgentStateSpec<'a> {
    AgentStateSpec {
        agent_id: state.agent_id(),
        revision: state.revision(),
        definition: state.definition_link(),
        memory: state.memory_link(),
        continuation: state.continuation(),
        current_episode: state.current_episode_link(),
        history,
    }
}
fn encode(state: &AgentState, bounds: &DecodeLimits) -> Result<AgentState, StateError> {
    AgentState::encode(&spec(state, &state.history().collect::<Vec<_>>()), bounds)
}

#[test]
fn complete_state_vectors_match_independent_codec_hash_cid_and_owned_encoding() {
    for fixture in fixtures().as_array().unwrap() {
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(&node(&fixture["record"])), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        let state = AgentState::decode(&expected, &DecodeLimits::default()).unwrap();
        assert_eq!(state.as_bytes(), expected);
        assert_eq!(state.cid().to_string(), fixture["cid"].as_str().unwrap());
        assert_eq!(
            state.agent_id().to_vec(),
            hex(fixture["record"]["agent_id"]["$bytes"].as_str().unwrap())
        );
        assert_eq!(
            state.revision(),
            fixture["record"]["revision"].as_u64().unwrap()
        );
        assert_eq!(
            state.definition_link().to_string(),
            fixture["record"]["definition"]["$link"].as_str().unwrap()
        );
        assert_eq!(
            state.memory_link().map(|v| v.to_string()).as_deref(),
            fixture["record"]["memory"]["$link"].as_str()
        );
        assert_eq!(
            state
                .current_episode_link()
                .map(|v| v.to_string())
                .as_deref(),
            fixture["record"]["current_episode"]["$link"].as_str()
        );
        match state.continuation() {
            Continuation::AwaitActor => {
                assert_eq!(fixture["record"]["continuation"]["tag"], "await_actor")
            }
            Continuation::AwaitAction { action_id } => assert_eq!(
                action_id.to_vec(),
                hex(fixture["record"]["continuation"]["action_id"]["$bytes"]
                    .as_str()
                    .unwrap())
            ),
        }
        let history = fixture["record"]["history"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["$link"].as_str().unwrap().parse::<DaslCid>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(state.history_count(), history.len() as u64);
        assert_eq!(state.history().collect::<Vec<_>>(), history);
        let owned = encode(&state, &DecodeLimits::default()).unwrap();
        assert_eq!(owned, state);
        drop(state);
        assert_eq!(owned.history().collect::<Vec<_>>(), history);
    }
}

#[test]
fn membership_kind_version_and_format_are_exact_without_defaults() {
    let mut source = record(0);
    let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let mut changed = source.clone();
        map(&mut changed).remove(&key);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
        map(&mut changed).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
    for (key, value, error) in [
        ("kind", Cbor::Text("episode".into()), StateError::WrongKind),
        ("kind", Cbor::Null, StateError::InvalidSchema),
        (
            "version",
            Cbor::Integer(0.into()),
            StateError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(2.into()),
            StateError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(u64::MAX.into()),
            StateError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer((-1).into()),
            StateError::InvalidSchema,
        ),
        (
            "format",
            Cbor::Text("raiv-state/2".into()),
            StateError::UnsupportedFormat,
        ),
        ("format", Cbor::Null, StateError::InvalidSchema),
    ] {
        let mut changed = source.clone();
        map(&mut changed).insert(key.into(), value);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(error)
        );
    }
    for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(0.into())] {
        assert_eq!(
            AgentState::decode(&bytes(&value), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
}

#[test]
fn raw_agent_ids_and_full_u64_revisions_are_not_coerced() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for value in [
        Cbor::Null,
        Cbor::Bytes(vec![0; 31]),
        Cbor::Bytes(vec![0; 33]),
        Cbor::Text("agent".into()),
        link,
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("agent_id".into(), value);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
    for value in [
        Cbor::Null,
        Cbor::Integer((-1).into()),
        Cbor::Text("0".into()),
        Cbor::Bool(false),
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("revision".into(), value);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
    for revision in [
        0,
        23,
        24,
        255,
        256,
        65_535,
        65_536,
        u32::MAX as u64,
        1 << 32,
        1 << 53,
        u64::MAX,
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("revision".into(), Cbor::Integer(revision.into()));
        let expected = bytes(&changed);
        let state = AgentState::decode(&expected, &DecodeLimits::default()).unwrap();
        assert_eq!(state.revision(), revision);
        assert_eq!(
            encode(&state, &DecodeLimits::default()).unwrap().as_bytes(),
            expected
        );
    }
}

#[test]
fn all_links_and_history_elements_require_structural_tag42_not_scalar_cids() {
    let source = AgentState::decode(&bytes(&record(4)), &DecodeLimits::default()).unwrap();
    let cid = source.definition_link();
    let raw = DaslCid::from_digest(DaslCodec::Raw, [0; 32]);
    let raw_link = link(raw);
    for key in ["definition", "memory", "current_episode", "history"] {
        for value in [
            Cbor::Bytes(cid.as_bytes().to_vec()),
            Cbor::Bytes(vec![0; 32]),
            Cbor::Text(cid.to_string()),
            Cbor::Integer(0.into()),
        ] {
            let mut changed = record(4);
            map(&mut changed).insert(
                key.into(),
                if key == "history" {
                    Cbor::Array(vec![value])
                } else {
                    value
                },
            );
            assert_eq!(
                AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(StateError::InvalidSchema)
            );
        }
        let mut changed = record(4);
        map(&mut changed).insert(
            key.into(),
            if key == "history" {
                Cbor::Array(vec![raw_link.clone()])
            } else {
                raw_link.clone()
            },
        );
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::Codec(CodecError::InvalidLink))
        );
    }
    let mut changed = record(0);
    map(&mut changed).insert("definition".into(), Cbor::Null);
    assert_eq!(
        AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
        Err(StateError::InvalidSchema)
    );
    for value in [
        Cbor::Null,
        Cbor::Text("history".into()),
        Cbor::Array(vec![Cbor::Null]),
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("history".into(), value);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
    let history = source.history().collect::<Vec<_>>();
    let base = spec(&source, &history);
    for input in [
        AgentStateSpec {
            definition: raw,
            ..base
        },
        AgentStateSpec {
            memory: Some(raw),
            ..base
        },
        AgentStateSpec {
            current_episode: Some(raw),
            ..base
        },
        AgentStateSpec {
            history: &[raw],
            ..base
        },
    ] {
        assert_eq!(
            AgentState::encode(&input, &DecodeLimits::default()),
            Err(StateError::Codec(CodecError::InvalidLink))
        );
    }
}

#[test]
fn continuation_variants_have_closed_membership_and_exact_raw_action_id() {
    for index in [0, 3] {
        for key in ["tag", "unknown"] {
            let mut changed = record(index);
            let fields = map(map(&mut changed).get_mut("continuation").unwrap());
            if key == "tag" {
                fields.remove(key);
            } else {
                fields.insert(key.into(), Cbor::Null);
            }
            assert_eq!(
                AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(StateError::InvalidSchema)
            );
        }
    }
    for value in [
        Cbor::Null,
        Cbor::Text("await_inference".into()),
        Cbor::Text("AwaitActor".into()),
    ] {
        let mut changed = record(0);
        map(map(&mut changed).get_mut("continuation").unwrap()).insert("tag".into(), value);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
    for value in [
        Cbor::Null,
        Cbor::Bytes(vec![0; 31]),
        Cbor::Bytes(vec![0; 33]),
        Cbor::Text("action".into()),
        node(&fixtures()[3]["record"]["current_episode"]),
    ] {
        let mut changed = record(3);
        map(map(&mut changed).get_mut("continuation").unwrap()).insert("action_id".into(), value);
        assert_eq!(
            AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(StateError::InvalidSchema)
        );
    }
    let mut changed = record(3);
    map(map(&mut changed).get_mut("continuation").unwrap()).remove("action_id");
    assert_eq!(
        AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
        Err(StateError::InvalidSchema)
    );
    let mut changed = record(0);
    map(map(&mut changed).get_mut("continuation").unwrap())
        .insert("action_id".into(), Cbor::Bytes(vec![0; 32]));
    assert_eq!(
        AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
        Err(StateError::InvalidSchema)
    );
}

#[test]
fn continuation_requires_matching_current_episode_presence_in_both_directions() {
    let actor = AgentState::decode(&bytes(&record(0)), &DecodeLimits::default()).unwrap();
    let action = AgentState::decode(&bytes(&record(3)), &DecodeLimits::default()).unwrap();
    assert_eq!(
        AgentState::encode(
            &AgentStateSpec {
                current_episode: action.current_episode_link(),
                ..spec(&actor, &[])
            },
            &DecodeLimits::default()
        ),
        Err(StateError::InvalidSchema)
    );
    assert_eq!(
        AgentState::encode(
            &AgentStateSpec {
                current_episode: None,
                ..spec(&action, &[])
            },
            &DecodeLimits::default()
        ),
        Err(StateError::InvalidSchema)
    );
    let mut changed = record(0);
    map(&mut changed).insert(
        "current_episode".into(),
        node(&fixtures()[3]["record"]["current_episode"]),
    );
    assert_eq!(
        AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
        Err(StateError::InvalidSchema)
    );
    let mut changed = record(3);
    map(&mut changed).insert("current_episode".into(), Cbor::Null);
    assert_eq!(
        AgentState::decode(&bytes(&changed), &DecodeLimits::default()),
        Err(StateError::InvalidSchema)
    );
}

#[test]
fn ordered_history_retains_duplicates_without_claiming_accepted_episode_consistency() {
    let source = AgentState::decode(&bytes(&record(0)), &DecodeLimits::default()).unwrap();
    let links = [
        DaslCid::from_digest(DaslCodec::Drisl, [3; 32]),
        DaslCid::from_digest(DaslCodec::Drisl, [1; 32]),
        DaslCid::from_digest(DaslCodec::Drisl, [3; 32]),
    ];
    let state = AgentState::encode(&spec(&source, &links), &DecodeLimits::default()).unwrap();
    assert_eq!(state.history().collect::<Vec<_>>(), links);
    assert_eq!(state.history_count(), 3);
    let swapped = [links[1], links[0], links[2]];
    assert_ne!(
        AgentState::encode(&spec(&source, &swapped), &DecodeLimits::default())
            .unwrap()
            .cid(),
        state.cid()
    );
    let list = &state.as_bytes()[state.history.clone()];
    let expected = [
        vec![0x83],
        links
            .iter()
            .flat_map(|cid| [vec![0xd8, 0x2a, 0x58, 0x25, 0], cid.as_bytes().to_vec()].concat())
            .collect(),
    ]
    .concat();
    assert_eq!(list, expected);
}

#[test]
fn root_history_byte_and_combined_depth_bounds_are_caller_supplied() {
    let source = AgentState::decode(&bytes(&record(4)), &DecodeLimits::default()).unwrap();
    let exact = DecodeLimits {
        max_block_bytes: source.as_bytes().len() as u64,
        max_depth: 4,
        max_collection_entries: 10,
    };
    assert_eq!(encode(&source, &exact).unwrap(), source);
    for (bounds, error) in [
        (
            DecodeLimits {
                max_block_bytes: exact.max_block_bytes - 1,
                ..exact
            },
            CodecError::ByteLimit,
        ),
        (
            DecodeLimits {
                max_collection_entries: 9,
                ..exact
            },
            CodecError::CollectionLimit,
        ),
        (
            DecodeLimits {
                max_depth: 3,
                ..exact
            },
            CodecError::DepthLimit,
        ),
    ] {
        assert_eq!(
            AgentState::decode(source.as_bytes(), &bounds),
            Err(StateError::Codec(error))
        );
        assert_eq!(encode(&source, &bounds), Err(StateError::Codec(error)));
    }
    let actor = AgentState::decode(&bytes(&record(0)), &DecodeLimits::default()).unwrap();
    let history = vec![source.definition_link(); 11];
    let bounds = DecodeLimits {
        max_collection_entries: 11,
        ..DecodeLimits::default()
    };
    let eleven = AgentState::encode(&spec(&actor, &history), &bounds).unwrap();
    let short = DecodeLimits {
        max_collection_entries: 10,
        ..bounds
    };
    assert_eq!(
        AgentState::encode(&spec(&actor, &history), &short),
        Err(StateError::Codec(CodecError::CollectionLimit))
    );
    assert_eq!(
        AgentState::decode(eleven.as_bytes(), &short),
        Err(StateError::Codec(CodecError::CollectionLimit))
    );
    let three = DecodeLimits {
        max_depth: 3,
        ..DecodeLimits::default()
    };
    for index in [0, 1, 3] {
        let state = AgentState::decode(&bytes(&record(index)), &three).unwrap();
        assert_eq!(encode(&state, &three).unwrap(), state);
        let two = DecodeLimits {
            max_depth: 2,
            ..three
        };
        assert_eq!(
            encode(&state, &two),
            Err(StateError::Codec(CodecError::DepthLimit))
        );
        assert_eq!(
            AgentState::decode(state.as_bytes(), &two),
            Err(StateError::Codec(CodecError::DepthLimit))
        );
    }
}

#[test]
fn malformed_truncated_duplicate_nonminimal_and_trailing_states_reject() {
    let encoded = bytes(&record(0));
    for end in 0..encoded.len() {
        assert!(AgentState::decode(&encoded[..end], &DecodeLimits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a2646b696e646161646b696e646162"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 10], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            AgentState::decode(&data, &DecodeLimits::default()),
            Err(StateError::Codec(error))
        );
    }
}
