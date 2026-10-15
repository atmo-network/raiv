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
    serde_json::from_str(include_str!("../../../fixtures/format/boundary.json")).unwrap()
}

fn record(index: usize) -> Cbor {
    node(&fixtures()[index]["record"])
}

fn encode(boundary: &Boundary, limits: &Limits) -> Result<Boundary, BoundaryError> {
    let command = match boundary.exit() {
        Exit::AwaitActor => None,
        Exit::AwaitAction(raw) => Some(Command::decode(raw, &Limits::default()).unwrap()),
    };
    Boundary::encode(
        &BoundarySpec {
            ordinal: boundary.ordinal(),
            input: boundary.input_link(),
            memory: boundary.memory_link(),
            exit: match command.as_ref() {
                Some(command) => ExitSpec::AwaitAction(command),
                None => ExitSpec::AwaitActor,
            },
            usage: boundary.usage(),
        },
        limits,
    )
}

#[test]
fn complete_boundary_vectors_match_independent_codec_hash_cid_and_owned_encoding() {
    for fixture in fixtures().as_array().unwrap() {
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(&node(&fixture["record"])), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        let boundary = Boundary::decode(&expected, &Limits::default()).unwrap();
        assert_eq!(boundary.as_bytes(), expected);
        assert_eq!(boundary.cid().to_string(), fixture["cid"].as_str().unwrap());
        assert_eq!(
            boundary.ordinal(),
            fixture["record"]["ordinal"].as_u64().unwrap()
        );
        assert_eq!(
            boundary.input_link().to_string(),
            fixture["record"]["input"]["$link"].as_str().unwrap()
        );
        assert_eq!(
            boundary.memory_link().map(|v| v.to_string()).as_deref(),
            fixture["record"]["memory"]["$link"].as_str()
        );
        assert_eq!(
            boundary
                .usage()
                .encode(&Limits::default().decode_limits())
                .unwrap(),
            bytes(&node(&fixture["record"]["usage"]))
        );
        match boundary.exit() {
            Exit::AwaitActor => assert_eq!(fixture["record"]["exit"]["tag"], "await_actor"),
            Exit::AwaitAction(raw) => {
                assert_eq!(fixture["record"]["exit"]["tag"], "await_action");
                assert_eq!(raw, bytes(&node(&fixture["record"]["exit"]["command"])));
                let start = boundary.as_bytes().as_ptr() as usize;
                assert!(raw.as_ptr() as usize >= start);
                assert!(raw.as_ptr() as usize + raw.len() <= start + boundary.as_bytes().len());
            }
        }
        let owned = encode(&boundary, &Limits::default()).unwrap();
        assert_eq!(owned, boundary);
        drop(boundary);
        assert_eq!(owned.as_bytes(), expected);
    }
}

#[test]
fn boundary_fields_kind_and_version_are_closed_required_and_not_defaulted() {
    let mut source = record(0);
    let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let mut changed = source.clone();
        map(&mut changed).remove(&key);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
        map(&mut changed).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
    }
    for (key, value, error) in [
        (
            "kind",
            Cbor::Text("episode".into()),
            BoundaryError::WrongKind,
        ),
        ("kind", Cbor::Null, BoundaryError::InvalidSchema),
        (
            "version",
            Cbor::Integer(0.into()),
            BoundaryError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(2.into()),
            BoundaryError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(u64::MAX.into()),
            BoundaryError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Text("1".into()),
            BoundaryError::InvalidSchema,
        ),
    ] {
        let mut changed = source.clone();
        map(&mut changed).insert(key.into(), value);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(error)
        );
    }
    for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(0.into())] {
        assert_eq!(
            Boundary::decode(&bytes(&value), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
    }
}

#[test]
fn links_are_structural_tag42_not_nullable_input_raw_ids_text_or_raw_codec() {
    let source = Boundary::decode(&bytes(&record(1)), &Limits::default()).unwrap();
    let raw = DaslCid::from_digest(DaslCodec::Raw, [0; 32]);
    for key in ["input", "memory"] {
        for value in [
            Cbor::Bytes(source.input_link().as_bytes().to_vec()),
            Cbor::Bytes(vec![0; 32]),
            Cbor::Text(source.input_link().to_string()),
        ] {
            let mut changed = record(1);
            map(&mut changed).insert(key.into(), value);
            assert_eq!(
                Boundary::decode(&bytes(&changed), &Limits::default()),
                Err(BoundaryError::InvalidSchema)
            );
        }
        let mut changed = record(1);
        map(&mut changed).insert(key.into(), link(raw));
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::Codec(CodecError::InvalidLink))
        );
    }
    let mut changed = record(1);
    map(&mut changed).insert("input".into(), Cbor::Null);
    assert_eq!(
        Boundary::decode(&bytes(&changed), &Limits::default()),
        Err(BoundaryError::InvalidSchema)
    );
    let base = BoundarySpec {
        ordinal: 0,
        input: source.input_link(),
        memory: source.memory_link(),
        exit: ExitSpec::AwaitActor,
        usage: source.usage(),
    };
    for spec in [
        BoundarySpec { input: raw, ..base },
        BoundarySpec {
            memory: Some(raw),
            ..base
        },
    ] {
        assert_eq!(
            Boundary::encode(&spec, &Limits::default()),
            Err(BoundaryError::Codec(CodecError::InvalidLink))
        );
    }
    let original = bytes(&record(2));
    let prefix = original
        .windows(5)
        .position(|v| v == [0xd8, 0x2a, 0x58, 0x25, 0])
        .unwrap();
    for (offset, value) in [(4, 1), (6, 0x55), (7, 0x13)] {
        let mut changed = original.clone();
        changed[prefix + offset] = value;
        assert_eq!(
            Boundary::decode(&changed, &Limits::default()),
            Err(BoundaryError::Codec(CodecError::InvalidLink))
        );
    }
}

#[test]
fn exit_variants_are_exact_and_command_presence_is_not_inferred() {
    for index in [0, 2] {
        let mut changed = record(index);
        map(map(&mut changed).get_mut("exit").unwrap()).remove("tag");
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
        let mut changed = record(index);
        map(map(&mut changed).get_mut("exit").unwrap()).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
    }
    for tag in [
        Cbor::Null,
        Cbor::Text("await_inference".into()),
        Cbor::Text("AwaitActor".into()),
    ] {
        let mut changed = record(2);
        map(map(&mut changed).get_mut("exit").unwrap()).insert("tag".into(), tag);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
    }
    let mut changed = record(0);
    map(map(&mut changed).get_mut("exit").unwrap()).remove("command");
    assert_eq!(
        Boundary::decode(&bytes(&changed), &Limits::default()),
        Err(BoundaryError::InvalidSchema)
    );
    let mut changed = record(0);
    map(map(&mut changed).get_mut("exit").unwrap())
        .insert("tag".into(), Cbor::Text("await_actor".into()));
    assert_eq!(
        Boundary::decode(&bytes(&changed), &Limits::default()),
        Err(BoundaryError::InvalidSchema)
    );
    let mut changed = record(0);
    map(map(&mut changed).get_mut("exit").unwrap()).insert("command".into(), Cbor::Null);
    assert_eq!(
        Boundary::decode(&bytes(&changed), &Limits::default()),
        Err(BoundaryError::Command(CommandError::InvalidSchema))
    );
}

#[test]
fn usage_is_complete_and_ordinals_remain_full_u64_data_not_episode_proof() {
    for key in [
        "model_calls",
        "local_tool_calls",
        "input_tokens",
        "output_tokens",
        "billed_units",
    ] {
        let mut changed = record(2);
        map(map(&mut changed).get_mut("usage").unwrap()).remove(key);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
        map(map(&mut changed).get_mut("usage").unwrap()).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
    }
    for value in [
        Cbor::Null,
        Cbor::Integer((-1).into()),
        Cbor::Text("0".into()),
    ] {
        let mut changed = record(2);
        map(&mut changed).insert("ordinal".into(), value);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::InvalidSchema)
        );
    }
    for value in [
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
        let mut changed = record(2);
        map(&mut changed).insert("ordinal".into(), Cbor::Integer(value.into()));
        let expected = bytes(&changed);
        let boundary = Boundary::decode(&expected, &Limits::default()).unwrap();
        assert_eq!(boundary.ordinal(), value);
        assert_eq!(
            encode(&boundary, &Limits::default()).unwrap().as_bytes(),
            expected
        );
    }
    let mut wrong_target = record(2);
    let memory = map(&mut wrong_target).get_mut("memory").unwrap().clone();
    map(&mut wrong_target).insert("input".into(), memory);
    assert!(Boundary::decode(&bytes(&wrong_target), &Limits::default()).is_ok());
}

#[test]
fn embedded_command_schema_and_model_arguments_are_checked_without_payload_copy() {
    for (key, value) in [
        ("action_id", Cbor::Bytes(vec![0; 31])),
        ("authority_binding", Cbor::Null),
        ("arguments", Cbor::Null),
        ("tool_version", Cbor::Integer((-1).into())),
    ] {
        let mut changed = record(0);
        map(map(map(&mut changed).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap())
        .insert(key.into(), value);
        assert_eq!(
            Boundary::decode(&bytes(&changed), &Limits::default()),
            Err(BoundaryError::Command(CommandError::InvalidSchema))
        );
    }
    for value in [
        Cbor::Bytes(vec![0]),
        Cbor::Integer((1u64 << 53).into()),
        Cbor::Float(1.5),
    ] {
        let mut changed = record(0);
        let command = map(map(map(&mut changed).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap());
        map(command.get_mut("arguments").unwrap()).insert("bad".into(), value);
        assert!(Boundary::decode(&bytes(&changed), &Limits::default()).is_err());
    }
    let mut changed = record(0);
    map(map(map(&mut changed).get_mut("exit").unwrap())
        .get_mut("command")
        .unwrap())
    .remove("tool_id");
    assert_eq!(
        Boundary::decode(&bytes(&changed), &Limits::default()),
        Err(BoundaryError::Command(CommandError::InvalidSchema))
    );
}

#[test]
fn whole_block_command_and_collection_caps_use_caller_policy() {
    let source = Boundary::decode(&bytes(&record(0)), &Limits::default()).unwrap();
    let command_len = match source.exit() {
        Exit::AwaitAction(raw) => raw.len(),
        _ => unreachable!(),
    };
    let exact = Limits {
        max_block_bytes: source.as_bytes().len() as u64,
        max_command_bytes: command_len as u64,
        max_depth: 5,
        max_collection_entries: 7,
        ..Limits::default()
    };
    assert_eq!(encode(&source, &exact).unwrap(), source);
    for (limits, error) in [
        (
            Limits {
                max_block_bytes: exact.max_block_bytes - 1,
                ..exact
            },
            BoundaryError::Codec(CodecError::ByteLimit),
        ),
        (
            Limits {
                max_command_bytes: exact.max_command_bytes - 1,
                ..exact
            },
            BoundaryError::Command(CommandError::Codec(CodecError::ByteLimit)),
        ),
        (
            Limits {
                max_collection_entries: 6,
                ..exact
            },
            BoundaryError::Codec(CodecError::CollectionLimit),
        ),
    ] {
        assert_eq!(Boundary::decode(source.as_bytes(), &limits), Err(error));
        assert_eq!(encode(&source, &limits), Err(error));
    }
    let shallow = Limits {
        max_depth: 4,
        ..exact
    };
    assert_eq!(
        Boundary::decode(source.as_bytes(), &shallow),
        Err(BoundaryError::Codec(CodecError::DepthLimit))
    );
    assert_eq!(
        encode(&source, &shallow),
        Err(BoundaryError::Command(CommandError::Codec(
            CodecError::DepthLimit
        )))
    );
}

#[test]
fn minimum_variant_depth_includes_tag_payload_and_command_argument_map() {
    let actor = Boundary::decode(&bytes(&record(2)), &Limits::default()).unwrap();
    let three = Limits {
        max_depth: 3,
        ..Limits::default()
    };
    assert_eq!(encode(&actor, &three).unwrap(), actor);
    assert_eq!(Boundary::decode(actor.as_bytes(), &three).unwrap(), actor);
    assert!(encode(
        &actor,
        &Limits {
            max_depth: 2,
            ..three
        }
    )
    .is_err());
    assert!(Boundary::decode(
        actor.as_bytes(),
        &Limits {
            max_depth: 2,
            ..three
        }
    )
    .is_err());
    let mut changed = record(0);
    map(map(map(&mut changed).get_mut("exit").unwrap())
        .get_mut("command")
        .unwrap())
    .insert("arguments".into(), Cbor::Map(Vec::new()));
    let action = Boundary::decode(&bytes(&changed), &Limits::default()).unwrap();
    let four = Limits {
        max_depth: 4,
        ..Limits::default()
    };
    assert_eq!(encode(&action, &four).unwrap(), action);
    assert_eq!(Boundary::decode(action.as_bytes(), &four).unwrap(), action);
    assert!(encode(&action, &three).is_err());
    assert!(Boundary::decode(action.as_bytes(), &three).is_err());
}

#[test]
fn malformed_truncated_duplicate_nonminimal_and_trailing_boundaries_reject() {
    let encoded = bytes(&record(2));
    for end in 0..encoded.len() {
        assert!(Boundary::decode(&encoded[..end], &Limits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a2646b696e646161646b696e646162"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 7], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            Boundary::decode(&data, &Limits::default()),
            Err(BoundaryError::Codec(error))
        );
    }
}
