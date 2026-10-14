use super::*;
use crate::format::oracle::{link, map_bytes as bytes};
use alloc::{collections::BTreeMap, string::String, vec};
use ciborium::value::Value;
use sha2::{Digest, Sha256};

const KEYS: [&str; 5] = [
    "model_calls",
    "local_tool_calls",
    "input_tokens",
    "output_tokens",
    "billed_units",
];

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn sample() -> Usage {
    Usage {
        model_calls: 1,
        local_tool_calls: 2,
        input_tokens: None,
        output_tokens: None,
        billed_units: None,
    }
}

fn record(usage: Usage) -> BTreeMap<String, Value> {
    KEYS.into_iter()
        .zip([
            Some(usage.model_calls),
            Some(usage.local_tool_calls),
            usage.input_tokens,
            usage.output_tokens,
            usage.billed_units,
        ])
        .map(|(key, value)| {
            (
                key.into(),
                value
                    .map(|v| Value::Integer(v.into()))
                    .unwrap_or(Value::Null),
            )
        })
        .collect()
}

#[test]
fn complete_usage_vectors_match_independent_codec_hash_and_nullable_mapping() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/format/accounting.json")).unwrap();
    for fixture in fixtures["usage"].as_array().unwrap() {
        let source = &fixture["record"];
        let usage = Usage {
            model_calls: source["model_calls"].as_u64().unwrap(),
            local_tool_calls: source["local_tool_calls"].as_u64().unwrap(),
            input_tokens: source["input_tokens"].as_u64(),
            output_tokens: source["output_tokens"].as_u64(),
            billed_units: source["billed_units"].as_u64(),
        };
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(record(usage)), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        assert_eq!(
            Usage::decode(&expected, &DecodeLimits::default()),
            Ok(usage)
        );
        assert_eq!(usage.encode(&DecodeLimits::default()).unwrap(), expected);
    }
}

#[test]
fn every_usage_key_is_required_even_when_its_value_is_null() {
    for key in KEYS {
        let mut fields = record(sample());
        fields.remove(key);
        assert_eq!(
            Usage::decode(&bytes(fields.clone()), &DecodeLimits::default()),
            Err(UsageError::InvalidSchema)
        );
        fields.insert("unknown".into(), Value::Null);
        assert_eq!(
            Usage::decode(&bytes(fields), &DecodeLimits::default()),
            Err(UsageError::InvalidSchema)
        );
    }
    for data in [vec![0xf6], vec![0x80], vec![0x00]] {
        assert_eq!(
            Usage::decode(&data, &DecodeLimits::default()),
            Err(UsageError::InvalidSchema)
        );
    }
}

#[test]
fn null_known_zero_and_known_maximum_are_distinct_for_each_cost() {
    for input_tokens in [None, Some(0), Some(u64::MAX)] {
        for output_tokens in [None, Some(0), Some(u64::MAX)] {
            for billed_units in [None, Some(0), Some(u64::MAX)] {
                let usage = Usage {
                    input_tokens,
                    output_tokens,
                    billed_units,
                    ..sample()
                };
                let encoded = usage.encode(&DecodeLimits::default()).unwrap();
                assert_eq!(encoded, bytes(record(usage)));
                assert_eq!(Usage::decode(&encoded, &DecodeLimits::default()), Ok(usage));
            }
        }
    }
    let unknown = sample().encode(&DecodeLimits::default()).unwrap();
    for field in ["input_tokens", "output_tokens", "billed_units"] {
        let mut known_zero = record(sample());
        known_zero.insert(field.into(), Value::Integer(0.into()));
        assert_ne!(unknown, bytes(known_zero));
    }
}

#[test]
fn counters_and_nullable_costs_are_full_u64_without_other_scalar_coercions() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for key in KEYS {
        for value in [
            Value::Integer((-1).into()),
            Value::Bool(false),
            Value::Text("0".into()),
            Value::Bytes(vec![]),
            link.clone(),
        ] {
            let mut fields = record(sample());
            fields.insert(key.into(), value);
            assert_eq!(
                Usage::decode(&bytes(fields), &DecodeLimits::default()),
                Err(UsageError::InvalidSchema)
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
            let mut fields = record(sample());
            fields.insert(key.into(), Value::Integer(value.into()));
            let expected = bytes(fields);
            assert_eq!(
                Usage::decode(&expected, &DecodeLimits::default())
                    .unwrap()
                    .encode(&DecodeLimits::default())
                    .unwrap(),
                expected
            );
        }
    }
    for key in ["model_calls", "local_tool_calls"] {
        let mut fields = record(sample());
        fields.insert(key.into(), Value::Null);
        assert_eq!(
            Usage::decode(&bytes(fields), &DecodeLimits::default()),
            Err(UsageError::InvalidSchema)
        );
    }
}

#[test]
fn usage_byte_depth_and_collection_bounds_are_caller_supplied() {
    let usage = sample();
    let encoded = usage.encode(&DecodeLimits::default()).unwrap();
    let exact = DecodeLimits {
        max_block_bytes: encoded.len() as u64,
        max_depth: 2,
        max_collection_entries: 5,
    };
    assert_eq!(usage.encode(&exact).unwrap(), encoded);
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
                max_depth: 0,
                ..exact
            },
            CodecError::DepthLimit,
        ),
        (
            DecodeLimits {
                max_depth: 1,
                ..exact
            },
            CodecError::DepthLimit,
        ),
        (
            DecodeLimits {
                max_collection_entries: 4,
                ..exact
            },
            CodecError::CollectionLimit,
        ),
    ] {
        assert_eq!(usage.encode(&bounds), Err(UsageError::Codec(error)));
        assert_eq!(
            Usage::decode(&encoded, &bounds),
            Err(UsageError::Codec(error))
        );
    }
}

#[test]
fn malformed_truncated_duplicate_nonminimal_and_trailing_usage_reject() {
    let encoded = sample().encode(&DecodeLimits::default()).unwrap();
    for end in 0..encoded.len() {
        assert!(Usage::decode(&encoded[..end], &DecodeLimits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a26b6d6f64656c5f63616c6c73006b6d6f64656c5f63616c6c7301"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 5], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            Usage::decode(&data, &DecodeLimits::default()),
            Err(UsageError::Codec(error))
        );
    }
}
