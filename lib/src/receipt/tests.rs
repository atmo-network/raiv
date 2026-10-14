use super::*;
use crate::format::oracle::{encode as bytes, fixture_value as node, link, map_mut as map};
use alloc::vec;
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
    serde_json::from_str(include_str!("../../../fixtures/format/terminal.json")).unwrap()
}

fn record() -> Cbor {
    node(&fixtures()["receipts"][0]["record"])
}

#[test]
fn borrowed_receipt_views_match_ciborium_owned_metadata_and_exact_model_spans() {
    for fixture in fixtures()["receipts"].as_array().unwrap() {
        let source = bytes(&node(&fixture["record"]));
        assert_eq!(source, hex(fixture["hex"].as_str().unwrap()));
        let view = ReceiptView::decode(&source, &Limits::default()).unwrap();
        let owned = Receipt::decode(&source, &Limits::default()).unwrap();
        assert_eq!(view.as_bytes().as_ptr(), source.as_ptr());
        assert_eq!(view.action_id(), owned.action_id());
        assert_eq!(view.command_digest(), owned.command_digest());
        assert_eq!(view.outcome(), owned.outcome());
        assert_eq!(view.finalized_at(), owned.finalized_at());
        assert_eq!(view.execution_reference(), owned.execution_reference());
        for (name, actual) in [
            ("result", view.result_bytes()),
            ("costs", view.costs_bytes()),
        ] {
            assert_eq!(actual, bytes(&node(&fixture["record"][name])));
            let offset = actual.as_ptr() as usize - source.as_ptr() as usize;
            assert_eq!(&source[offset..offset + actual.len()], actual);
        }
        assert_eq!(owned.view().as_bytes().as_ptr(), owned.as_bytes().as_ptr());
        assert_eq!(
            owned.view().result_bytes().as_ptr(),
            owned.result_bytes().as_ptr()
        );
        assert_eq!(
            owned.view().costs_bytes().as_ptr(),
            owned.costs_bytes().as_ptr()
        );
    }
}

#[test]
fn borrowed_receipt_validation_rejects_the_same_bounds_prefixes_and_invalid_records_as_owned() {
    let valid = bytes(&record());
    for limits in [
        Limits::default(),
        Limits {
            max_receipt_bytes: valid.len() as u64 - 1,
            ..Limits::default()
        },
        Limits {
            max_block_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_depth: 1,
            ..Limits::default()
        },
        Limits {
            max_collection_entries: 6,
            ..Limits::default()
        },
    ] {
        for end in 0..=valid.len() {
            let data = &valid[..end];
            assert_eq!(
                ReceiptView::decode(data, &limits).err(),
                Receipt::decode(data, &limits).err()
            );
        }
    }
    for name in RECEIPT_FIELDS {
        let mut value = record();
        map(&mut value).remove(name);
        let data = bytes(&value);
        assert_eq!(
            ReceiptView::decode(&data, &Limits::default()).err(),
            Some(ReceiptError::InvalidSchema)
        );
    }
    for outcome in ["pending", "unknown", "timeout"] {
        let mut value = record();
        map(&mut value).insert("outcome".into(), Cbor::Text(outcome.into()));
        let data = bytes(&value);
        assert_eq!(
            ReceiptView::decode(&data, &Limits::default()).err(),
            Some(ReceiptError::InvalidSchema)
        );
    }
    for costs in [
        Cbor::Bytes(vec![]),
        Cbor::Array(vec![]),
        Cbor::Map(vec![(
            Cbor::Text("x".into()),
            Cbor::Integer(u64::MAX.into()),
        )]),
    ] {
        let mut value = record();
        map(&mut value).insert("costs".into(), costs);
        let data = bytes(&value);
        assert!(ReceiptView::decode(&data, &Limits::default()).is_err());
        assert_eq!(
            ReceiptView::decode(&data, &Limits::default()).err(),
            Receipt::decode(&data, &Limits::default()).err()
        );
    }
}

fn spec(receipt: &Receipt) -> ReceiptSpec<'_> {
    ReceiptSpec {
        action_id: receipt.action_id(),
        command_digest: receipt.command_digest(),
        outcome: receipt.outcome(),
        result: receipt.result_bytes(),
        execution_reference: receipt.execution_reference(),
        finalized_at: receipt.finalized_at(),
        costs: receipt.costs_bytes(),
    }
}

#[test]
fn complete_terminal_vectors_match_independent_codec_hash_and_owned_encoding() {
    for fixture in fixtures()["receipts"].as_array().unwrap() {
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(&node(&fixture["record"])), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        let receipt = Receipt::decode(&expected, &Limits::default()).unwrap();
        assert_eq!(receipt.as_bytes(), expected);
        assert_eq!(receipt.action_id(), [0x10; 32]);
        assert_eq!(
            receipt.command_digest().to_vec(),
            hex(fixture["record"]["command_digest"]["$bytes"]
                .as_str()
                .unwrap())
        );
        assert_eq!(
            receipt.outcome().as_str(),
            fixture["record"]["outcome"].as_str().unwrap()
        );
        assert_eq!(
            receipt.execution_reference(),
            fixture["record"]["execution_reference"].as_str()
        );
        assert_eq!(
            receipt.finalized_at().number,
            fixture["record"]["finalized_at"]["number"]
                .as_u64()
                .unwrap()
        );
        assert_eq!(
            receipt.result_bytes(),
            bytes(&node(&fixture["record"]["result"]))
        );
        assert_eq!(
            receipt.costs_bytes(),
            bytes(&node(&fixture["record"]["costs"]))
        );
        let owned = Receipt::encode(&spec(&receipt), &Limits::default()).unwrap();
        assert_eq!(owned, receipt);
        drop(receipt);
        assert_eq!(
            owned.execution_reference(),
            fixture["record"]["execution_reference"].as_str()
        );
    }
}

#[test]
fn receipt_and_nested_position_membership_are_exact() {
    let mut source = record();
    let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let mut changed = source.clone();
        map(&mut changed).remove(&key);
        assert_eq!(
            Receipt::decode(&bytes(&changed), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
        map(&mut changed).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            Receipt::decode(&bytes(&changed), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
    }
    for key in ["domain", "number", "reference"] {
        let mut changed = source.clone();
        let fields = map(map(&mut changed).get_mut("finalized_at").unwrap());
        fields.remove(key);
        assert_eq!(
            Receipt::decode(&bytes(&changed), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
        map(map(&mut changed).get_mut("finalized_at").unwrap())
            .insert("unknown".into(), Cbor::Null);
        assert_eq!(
            Receipt::decode(&bytes(&changed), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
    }
    for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(0.into())] {
        assert_eq!(
            Receipt::decode(&bytes(&value), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
    }
}

#[test]
fn ids_outcomes_and_reference_types_are_not_coerced_or_inferred() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for field in ["action_id", "command_digest"] {
        for value in [
            Cbor::Null,
            Cbor::Bytes(vec![0; 31]),
            Cbor::Bytes(vec![0; 33]),
            Cbor::Text("id".into()),
            link.clone(),
        ] {
            let mut changed = record();
            map(&mut changed).insert(field.into(), value);
            assert_eq!(
                Receipt::decode(&bytes(&changed), &Limits::default()),
                Err(ReceiptError::InvalidSchema)
            );
        }
    }
    for outcome in [
        "unknown",
        "pending",
        "rpc_interrupted",
        "not-included",
        "Succeeded",
        "",
    ] {
        let mut changed = record();
        map(&mut changed).insert("outcome".into(), Cbor::Text(outcome.into()));
        assert_eq!(
            Receipt::decode(&bytes(&changed), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
    }
    for (field, value) in [
        ("outcome", Cbor::Null),
        ("execution_reference", Cbor::Bytes(vec![])),
        ("execution_reference", Cbor::Integer(0.into())),
        ("result", Cbor::Null),
        ("costs", Cbor::Array(vec![])),
        ("finalized_at", Cbor::Null),
    ] {
        let mut changed = record();
        map(&mut changed).insert(field.into(), value);
        assert_eq!(
            Receipt::decode(&bytes(&changed), &Limits::default()),
            Err(ReceiptError::InvalidSchema)
        );
    }
}

#[test]
fn result_and_cost_maps_remain_constrained_model_values() {
    let source = Receipt::decode(&bytes(&record()), &Limits::default()).unwrap();
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for field in ["result", "costs"] {
        for value in [
            Cbor::Bytes(vec![0]),
            Cbor::Integer((1u64 << 53).into()),
            Cbor::Float(1.5),
            link.clone(),
        ] {
            let mut changed = record();
            map(map(&mut changed).get_mut(field).unwrap()).insert("bad".into(), value);
            assert!(Receipt::decode(&bytes(&changed), &Limits::default()).is_err());
        }
        for data in [
            "f6",
            "80",
            "b801616100",
            "a0ff",
            "a2616100616101",
            "a1616140",
        ] {
            let raw = hex(data);
            let mut input = spec(&source);
            if field == "result" {
                input.result = &raw;
            } else {
                input.costs = &raw;
            }
            assert!(
                Receipt::encode(&input, &Limits::default()).is_err(),
                "{field} {data}"
            );
        }
    }
}

#[test]
fn whole_receipt_limits_and_actual_embedded_depth_are_enforced() {
    let source = Receipt::decode(&bytes(&record()), &Limits::default()).unwrap();
    let exact = Limits {
        max_receipt_bytes: source.as_bytes().len() as u64,
        max_depth: 4,
        ..Limits::default()
    };
    assert_eq!(Receipt::encode(&spec(&source), &exact).unwrap(), source);
    for (limits, error) in [
        (
            Limits {
                max_receipt_bytes: exact.max_receipt_bytes - 1,
                ..exact
            },
            CodecError::ByteLimit,
        ),
        (
            Limits {
                max_block_bytes: exact.max_receipt_bytes - 1,
                max_receipt_bytes: u64::MAX,
                ..exact
            },
            CodecError::ByteLimit,
        ),
        (
            Limits {
                max_depth: 3,
                ..exact
            },
            CodecError::DepthLimit,
        ),
        (
            Limits {
                max_collection_entries: 6,
                ..exact
            },
            CodecError::CollectionLimit,
        ),
    ] {
        assert_eq!(
            Receipt::decode(source.as_bytes(), &limits),
            Err(ReceiptError::Codec(error))
        );
        assert_eq!(
            Receipt::encode(&spec(&source), &limits),
            Err(ReceiptError::Codec(error))
        );
    }
    let empty = ReceiptSpec {
        result: &[0xa0],
        costs: &[0xa0],
        ..spec(&source)
    };
    let shallow = Limits {
        max_depth: 3,
        ..exact
    };
    let receipt = Receipt::encode(&empty, &shallow).unwrap();
    assert_eq!(
        Receipt::decode(receipt.as_bytes(), &shallow).unwrap(),
        receipt
    );
    assert_eq!(
        Receipt::encode(
            &empty,
            &Limits {
                max_depth: 2,
                ..exact
            }
        ),
        Err(ReceiptError::Codec(CodecError::DepthLimit))
    );
}

#[test]
fn malformed_truncated_duplicate_and_trailing_receipts_reject() {
    let encoded = bytes(&record());
    for end in 0..encoded.len() {
        assert!(Receipt::decode(&encoded[..end], &Limits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a265636f737473a065636f737473a0"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 7], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            Receipt::decode(&data, &Limits::default()),
            Err(ReceiptError::Codec(error))
        );
    }
}

#[test]
fn matching_command_binds_both_id_and_domain_digest_but_not_provenance_or_phase() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/format/command.json")).unwrap();
    let command_record = node(&fixture[0]["record"]);
    let command = Command::decode(&bytes(&command_record), &Limits::default()).unwrap();
    let receipt = Receipt::decode(&bytes(&record()), &Limits::default()).unwrap();
    assert!(receipt.matches_command([0xa0; 32], &command));
    assert!(!receipt.matches_command([0xa1; 32], &command));
    let mut competing = command_record.clone();
    map(map(&mut competing).get_mut("arguments").unwrap())
        .insert("different".into(), Cbor::Bool(true));
    let competing = Command::decode(&bytes(&competing), &Limits::default()).unwrap();
    assert_eq!(competing.action_id(), command.action_id());
    assert!(!receipt.matches_command([0xa0; 32], &competing));
    for field in ["action_id", "command_digest"] {
        let mut changed = record();
        map(&mut changed).insert(field.into(), Cbor::Bytes(vec![0; 32]));
        let changed = Receipt::decode(&bytes(&changed), &Limits::default()).unwrap();
        assert!(!changed.matches_command([0xa0; 32], &command));
    }
}
