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
    serde_json::from_str(include_str!("../../../fixtures/format/input.json")).unwrap()
}

fn record(index: usize) -> Cbor {
    node(&fixtures()[index]["record"])
}

fn encode(input: &ActivationInput, limits: &Limits) -> Result<ActivationInput, InputError> {
    let receipt = input
        .receipt_bytes()
        .map(|raw| Receipt::decode(raw, &Limits::default()).unwrap());
    ActivationInput::encode(
        &ActivationInputSpec {
            event_id: input.event_id(),
            cause: input.cause(),
            observed_at: input.observed_at(),
            observation: input.observation_bytes(),
            receipt: receipt.as_ref(),
            budget_admission: input.budget_admission(),
        },
        limits,
    )
}

#[test]
fn complete_input_vectors_match_independent_codec_cid_hash_and_owned_encoding() {
    for fixture in fixtures().as_array().unwrap() {
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(&node(&fixture["record"])), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        let input = ActivationInput::decode(&expected, &Limits::default()).unwrap();
        assert_eq!(input.as_bytes(), expected);
        assert_eq!(input.cid().to_string(), fixture["cid"].as_str().unwrap());
        assert_eq!(
            input.event_id().to_vec(),
            hex(fixture["record"]["event_id"]["$bytes"].as_str().unwrap())
        );
        assert_eq!(
            input.cause().as_str(),
            fixture["record"]["cause"].as_str().unwrap()
        );
        assert_eq!(
            input
                .observed_at()
                .encode(&Limits::default().decode_limits())
                .unwrap(),
            bytes(&node(&fixture["record"]["observed_at"]))
        );
        assert_eq!(
            input
                .budget_admission()
                .encode(&Limits::default().decode_limits())
                .unwrap(),
            bytes(&node(&fixture["record"]["budget_admission"]))
        );
        assert_eq!(
            input.observation_bytes(),
            bytes(&node(&fixture["record"]["observation"]))
        );
        if fixture["record"]["receipt"].is_null() {
            assert!(input.receipt_bytes().is_none());
        } else {
            assert_eq!(
                input.receipt_bytes().unwrap(),
                bytes(&node(&fixture["record"]["receipt"]))
            );
        }
        let owned = encode(&input, &Limits::default()).unwrap();
        assert_eq!(owned, input);
        drop(input);
        assert_eq!(owned.as_bytes(), expected);
    }
}

#[test]
fn input_membership_kind_and_versions_are_exact_without_defaults() {
    let mut source = record(0);
    let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let mut changed = source.clone();
        map(&mut changed).remove(&key);
        assert_eq!(
            ActivationInput::decode(&bytes(&changed), &Limits::default()),
            Err(InputError::InvalidSchema)
        );
        map(&mut changed).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            ActivationInput::decode(&bytes(&changed), &Limits::default()),
            Err(InputError::InvalidSchema)
        );
    }
    for (field, value, error) in [
        ("kind", Cbor::Text("memory".into()), InputError::WrongKind),
        (
            "version",
            Cbor::Integer(0.into()),
            InputError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(2.into()),
            InputError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(u64::MAX.into()),
            InputError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer((-1).into()),
            InputError::InvalidSchema,
        ),
        ("kind", Cbor::Null, InputError::InvalidSchema),
    ] {
        let mut changed = source.clone();
        map(&mut changed).insert(field.into(), value);
        assert_eq!(
            ActivationInput::decode(&bytes(&changed), &Limits::default()),
            Err(error)
        );
    }
    for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(0.into())] {
        assert_eq!(
            ActivationInput::decode(&bytes(&value), &Limits::default()),
            Err(InputError::InvalidSchema)
        );
    }
}

#[test]
fn event_ids_causes_and_nested_records_are_not_coerced() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for value in [
        Cbor::Null,
        Cbor::Bytes(vec![0; 31]),
        Cbor::Bytes(vec![0; 33]),
        Cbor::Text("provider-id".into()),
        link,
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("event_id".into(), value);
        assert_eq!(
            ActivationInput::decode(&bytes(&changed), &Limits::default()),
            Err(InputError::InvalidSchema)
        );
    }
    for (field, value) in [
        ("cause", Cbor::Text("unknown".into())),
        ("cause", Cbor::Null),
        ("observed_at", Cbor::Null),
        ("budget_admission", Cbor::Null),
    ] {
        let mut changed = record(0);
        map(&mut changed).insert(field.into(), value);
        assert_eq!(
            ActivationInput::decode(&bytes(&changed), &Limits::default()),
            Err(InputError::InvalidSchema)
        );
    }
    for (outer, keys) in [
        ("observed_at", vec!["domain", "number", "reference"]),
        (
            "budget_admission",
            vec![
                "admission_id",
                "activation_units",
                "remaining_episode_units",
                "remaining_model_calls",
                "remaining_actions",
            ],
        ),
    ] {
        for key in keys {
            let mut changed = record(0);
            map(map(&mut changed).get_mut(outer).unwrap()).remove(key);
            assert_eq!(
                ActivationInput::decode(&bytes(&changed), &Limits::default()),
                Err(InputError::InvalidSchema)
            );
            map(map(&mut changed).get_mut(outer).unwrap()).insert("unknown".into(), Cbor::Null);
            assert_eq!(
                ActivationInput::decode(&bytes(&changed), &Limits::default()),
                Err(InputError::InvalidSchema)
            );
        }
    }
}

#[test]
fn cause_receipt_coupling_rejects_both_invalid_shapes_in_encode_and_decode() {
    let action = ActivationInput::decode(&bytes(&record(2)), &Limits::default()).unwrap();
    let receipt = Receipt::decode(action.receipt_bytes().unwrap(), &Limits::default()).unwrap();
    let base = ActivationInputSpec {
        event_id: action.event_id(),
        cause: Cause::ActorWake,
        observed_at: action.observed_at(),
        observation: action.observation_bytes(),
        receipt: Some(&receipt),
        budget_admission: action.budget_admission(),
    };
    assert_eq!(
        ActivationInput::encode(&base, &Limits::default()),
        Err(InputError::InvalidSchema)
    );
    assert_eq!(
        ActivationInput::encode(
            &ActivationInputSpec {
                cause: Cause::ActionResult,
                receipt: None,
                ..base
            },
            &Limits::default()
        ),
        Err(InputError::InvalidSchema)
    );
    let mut changed = record(2);
    map(&mut changed).insert("cause".into(), Cbor::Text("actor_wake".into()));
    assert_eq!(
        ActivationInput::decode(&bytes(&changed), &Limits::default()),
        Err(InputError::InvalidSchema)
    );
    let mut changed = record(0);
    map(&mut changed).insert("cause".into(), Cbor::Text("action_result".into()));
    assert_eq!(
        ActivationInput::decode(&bytes(&changed), &Limits::default()),
        Err(InputError::InvalidSchema)
    );
}

#[test]
fn observation_is_a_canonical_model_map_not_runtime_bytes_or_links() {
    let source = ActivationInput::decode(&bytes(&record(0)), &Limits::default()).unwrap();
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for value in [
        Cbor::Bytes(vec![0]),
        Cbor::Integer((1u64 << 53).into()),
        Cbor::Float(1.5),
        link,
    ] {
        let mut changed = record(0);
        map(map(&mut changed).get_mut("observation").unwrap()).insert("bad".into(), value);
        assert!(ActivationInput::decode(&bytes(&changed), &Limits::default()).is_err());
    }
    for data in [
        "f6",
        "80",
        "a1616140",
        "a0ff",
        "b801616100",
        "a2616100616101",
    ] {
        let raw = hex(data);
        assert!(ActivationInput::encode(
            &ActivationInputSpec {
                event_id: source.event_id(),
                cause: source.cause(),
                observed_at: source.observed_at(),
                observation: &raw,
                receipt: None,
                budget_admission: source.budget_admission()
            },
            &Limits::default()
        )
        .is_err());
    }
}

#[test]
fn nested_receipt_schema_and_terminal_status_are_checked_before_input_ownership() {
    for (key, value) in [
        ("outcome", Cbor::Text("pending".into())),
        ("action_id", Cbor::Bytes(vec![0; 31])),
        ("result", Cbor::Null),
    ] {
        let mut changed = record(2);
        map(map(&mut changed).get_mut("receipt").unwrap()).insert(key.into(), value);
        assert_eq!(
            ActivationInput::decode(&bytes(&changed), &Limits::default()),
            Err(InputError::Receipt(ReceiptError::InvalidSchema))
        );
    }
    let mut changed = record(2);
    map(map(&mut changed).get_mut("receipt").unwrap()).remove("costs");
    assert_eq!(
        ActivationInput::decode(&bytes(&changed), &Limits::default()),
        Err(InputError::Receipt(ReceiptError::InvalidSchema))
    );
    let mut changed = record(2);
    map(map(&mut changed).get_mut("budget_admission").unwrap())
        .insert("admission_id".into(), Cbor::Bytes(vec![0; 31]));
    assert_eq!(
        ActivationInput::decode(&bytes(&changed), &Limits::default()),
        Err(InputError::InvalidSchema)
    );
}

#[test]
fn block_observation_receipt_and_collection_caps_use_caller_policy() {
    let input = ActivationInput::decode(&bytes(&record(2)), &Limits::default()).unwrap();
    let exact = Limits {
        max_block_bytes: input.as_bytes().len() as u64,
        max_observation_bytes: input.observation_bytes().len() as u64,
        max_receipt_bytes: input.receipt_bytes().unwrap().len() as u64,
        max_depth: 5,
        max_collection_entries: 8,
        ..Limits::default()
    };
    assert_eq!(encode(&input, &exact).unwrap(), input);
    for (limits, error) in [
        (
            Limits {
                max_block_bytes: exact.max_block_bytes - 1,
                ..exact
            },
            InputError::Codec(CodecError::ByteLimit),
        ),
        (
            Limits {
                max_observation_bytes: exact.max_observation_bytes - 1,
                ..exact
            },
            InputError::Codec(CodecError::ByteLimit),
        ),
        (
            Limits {
                max_receipt_bytes: exact.max_receipt_bytes - 1,
                ..exact
            },
            InputError::Receipt(ReceiptError::Codec(CodecError::ByteLimit)),
        ),
        (
            Limits {
                max_collection_entries: 7,
                ..exact
            },
            InputError::Codec(CodecError::CollectionLimit),
        ),
        (
            Limits {
                max_depth: 4,
                ..exact
            },
            InputError::Codec(CodecError::DepthLimit),
        ),
    ] {
        assert_eq!(
            ActivationInput::decode(input.as_bytes(), &limits),
            Err(error)
        );
        assert_eq!(encode(&input, &limits), Err(error));
    }
    let max_budget = ActivationInput::decode(&bytes(&record(1)), &Limits::default()).unwrap();
    let tight = Limits {
        max_observation_bytes: 1,
        ..Limits::default()
    };
    assert_eq!(
        ActivationInput::decode(max_budget.as_bytes(), &tight),
        Err(InputError::Codec(CodecError::ByteLimit))
    );
    assert_eq!(
        encode(&max_budget, &tight),
        Err(InputError::Codec(CodecError::ByteLimit))
    );
}

#[test]
fn combined_minimum_depth_and_borrowed_spans_preserve_one_immutable_buffer() {
    let wake = ActivationInput::decode(&bytes(&record(0)), &Limits::default()).unwrap();
    let three = Limits {
        max_depth: 3,
        ..Limits::default()
    };
    assert_eq!(encode(&wake, &three).unwrap(), wake);
    assert_eq!(
        ActivationInput::decode(wake.as_bytes(), &three).unwrap(),
        wake
    );
    assert!(encode(
        &wake,
        &Limits {
            max_depth: 2,
            ..three
        }
    )
    .is_err());
    let mut terminal = record(3);
    map(&mut terminal).insert("observation".into(), Cbor::Map(Vec::new()));
    let action = ActivationInput::decode(&bytes(&terminal), &Limits::default()).unwrap();
    let four = Limits {
        max_depth: 4,
        ..Limits::default()
    };
    assert_eq!(encode(&action, &four).unwrap(), action);
    assert_eq!(
        ActivationInput::decode(action.as_bytes(), &four).unwrap(),
        action
    );
    assert!(encode(&action, &three).is_err());
    assert!(ActivationInput::decode(action.as_bytes(), &three).is_err());
    let start = action.as_bytes().as_ptr() as usize;
    let end = start + action.as_bytes().len();
    for view in [action.observation_bytes(), action.receipt_bytes().unwrap()] {
        assert!((view.as_ptr() as usize) >= start);
        assert!((view.as_ptr() as usize) + view.len() <= end);
    }
}

#[test]
fn malformed_truncated_duplicate_nonminimal_and_trailing_inputs_reject() {
    let encoded = bytes(&record(0));
    for end in 0..encoded.len() {
        assert!(ActivationInput::decode(&encoded[..end], &Limits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a2646b696e646161646b696e646162"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 8], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            ActivationInput::decode(&data, &Limits::default()),
            Err(InputError::Codec(error))
        );
    }
}
