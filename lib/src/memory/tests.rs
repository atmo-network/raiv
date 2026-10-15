use super::*;
use crate::format::oracle::json_bytes;
use alloc::{string::ToString, vec::Vec};
use serde_json::{json, Value};

pub(super) fn memory(value: Value) -> Memory {
    Memory::decode(
        &json_bytes(&json!({"kind":"memory","version":1,"value":value})),
        &DecodeLimits::default(),
    )
    .unwrap()
}

#[test]
fn independent_golden_bytes_cids_and_sparse_roundtrip() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../fixtures/format/memory.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let expected = json_bytes(&fixture["record"]);
        use core::fmt::Write;
        let mut hex = alloc::string::String::new();
        for byte in &expected {
            write!(&mut hex, "{byte:02x}").unwrap();
        }
        assert_eq!(hex, fixture["hex"]);
        let decoded = Memory::decode(&expected, &DecodeLimits::default()).unwrap();
        assert_eq!(decoded.as_bytes(), expected);
        assert_eq!(decoded.cid().to_string(), fixture["cid"]);
        assert_eq!(
            decoded.value_bytes(),
            json_bytes(&fixture["record"]["value"])
        );
    }
    let empty = Memory::encode(&[], &DecodeLimits::default()).unwrap();
    assert_eq!(empty.value_bytes(), &[0xa0]);
    assert_eq!(empty.cid().to_string(), fixtures[0]["cid"]);
    for value in [
        json!({}),
        json!({"intents":{"x":1}}),
        json!({"tactics":{"x":[{}, {"flag":false}, []]}}),
    ] {
        let source = memory(value);
        assert_eq!(
            Memory::decode(source.as_bytes(), &DecodeLimits::default()).unwrap(),
            source
        );
    }
}

#[test]
fn sparse_encoding_is_order_independent_and_rejects_unnormalized_authored_state() {
    let a = [("aa", ValueRef::Integer(3)), ("b", ValueRef::Integer(2))];
    let b = [("b", ValueRef::Integer(2)), ("aa", ValueRef::Integer(3))];
    let first = Memory::encode(
        &[
            ("strategy", ValueRef::Map(&a)),
            ("intents", ValueRef::Map(&a)),
        ],
        &DecodeLimits::default(),
    )
    .unwrap();
    let second = Memory::encode(
        &[
            ("intents", ValueRef::Map(&b)),
            ("strategy", ValueRef::Map(&b)),
        ],
        &DecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(first, second);
    for plane in ["intents", "strategy", "tactics", "intent"] {
        assert_eq!(
            Memory::encode(&[(plane, ValueRef::Map(&[]))], &DecodeLimits::default()),
            Err(MemoryError::InvalidSchema)
        );
    }
    let field = [("x", ValueRef::Null)];
    assert_eq!(
        Memory::encode(
            &[("tactics", ValueRef::Map(&field))],
            &DecodeLimits::default()
        ),
        Err(MemoryError::InvalidSchema)
    );
}

#[test]
fn persisted_null_or_empty_object_fields_and_aliases_are_rejected_not_repaired() {
    for value in [
        json!(null),
        json!([]),
        json!({"intent":{"x":1}}),
        json!({"strategies":{"x":1}}),
        json!({"Strategic":{"x":1}}),
        json!({"Tactics":{"x":1}}),
        json!({"response":{"x":1}}),
        json!({"strategy":null}),
        json!({"strategy":1}),
        json!({"strategy":[]}),
        json!({"strategy":{}}),
        json!({"strategy":{"x":null}}),
        json!({"strategy":{"x":{}}}),
        json!({"tactics":{"a":[null]}}),
        json!({"tactics":{"a":[{"x":null}]}}),
        json!({"tactics":{"a":[[{"x":{"y":null}}]]}}),
        json!({"tactics":{"a":[{"x":{}}]}}),
    ] {
        assert_eq!(
            Memory::decode(
                &json_bytes(&json!({"kind":"memory","version":1,"value":value})),
                &DecodeLimits::default()
            ),
            Err(MemoryError::InvalidSchema),
            "{value}"
        );
    }
    // Generic model/DRISL null remains supported outside this domain.
    assert_eq!(
        crate::format::validate_model_value(&[0xf6], &DecodeLimits::default()),
        Ok(())
    );
    assert_eq!(
        crate::format::validate_drisl(&[0xf6], &DecodeLimits::default()),
        Ok(())
    );
}

#[test]
fn structural_headers_keep_runtime_u64_error_precedence_and_closed_membership() {
    for version in [0, 2, u64::MAX] {
        assert_eq!(
            Memory::decode(
                &json_bytes(&json!({"kind":"memory","version":version,"value":{}})),
                &DecodeLimits::default()
            ),
            Err(MemoryError::UnsupportedVersion)
        );
    }
    assert_eq!(
        Memory::decode(
            &json_bytes(&json!({"kind":"definition","version":1,"value":{}})),
            &DecodeLimits::default()
        ),
        Err(MemoryError::WrongKind)
    );
    for record in [
        json!({}),
        json!({"kind":"memory","version":1}),
        json!({"kind":0,"version":1,"value":{}}),
        json!({"kind":"memory","version":null,"value":{}}),
        json!({"kind":"memory","version":1,"value":{},"extra":null}),
    ] {
        assert_eq!(
            Memory::decode(&json_bytes(&record), &DecodeLimits::default()),
            Err(MemoryError::InvalidSchema)
        );
    }
}

#[test]
fn numeric_bytes_links_and_record_resource_bounds_are_not_widened() {
    let bad = json_bytes(
        &json!({"kind":"memory","version":1,"value":{"strategy":{"x":9007199254740992u64}}}),
    );
    assert_eq!(
        Memory::decode(&bad, &DecodeLimits::default()),
        Err(MemoryError::Codec(CodecError::ModelIntegerRange))
    );
    for value in [
        json!({"$bytes":"01"}),
        json!({"$link":DaslCid::for_block(DaslCodec::Drisl, &[0xa0]).to_string()}),
    ] {
        let record = crate::format::oracle::fixture_bytes(
            &json!({"kind":"memory","version":1,"value":{"strategy":{"x":value}}}),
        );
        assert_eq!(
            Memory::decode(&record, &DecodeLimits::default()),
            Err(MemoryError::Codec(CodecError::UnsupportedValue))
        );
    }
    let empty = memory(json!({}));
    for bounds in [
        DecodeLimits {
            max_block_bytes: empty.as_bytes().len() as u64 - 1,
            ..DecodeLimits::default()
        },
        DecodeLimits {
            max_depth: 1,
            ..DecodeLimits::default()
        },
        DecodeLimits {
            max_collection_entries: 2,
            ..DecodeLimits::default()
        },
    ] {
        assert!(Memory::decode(empty.as_bytes(), &bounds).is_err());
    }
    for length in 0..empty.as_bytes().len() {
        assert!(Memory::decode(&empty.as_bytes()[..length], &DecodeLimits::default()).is_err());
    }
    let mut trailing: Vec<u8> = empty.as_bytes().to_vec();
    trailing.push(0);
    assert_eq!(
        Memory::decode(&trailing, &DecodeLimits::default()),
        Err(MemoryError::Codec(CodecError::TrailingBytes))
    );
}
