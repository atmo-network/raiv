use super::*;
use crate::format::oracle::{link, map_bytes as bytes};
use alloc::{collections::BTreeMap, string::String, vec};
use ciborium::value::Value;
use sha2::{Digest, Sha256};

const COUNTERS: [&str; 4] = [
    "activation_units",
    "remaining_episode_units",
    "remaining_model_calls",
    "remaining_actions",
];

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn sample() -> BudgetAdmission {
    BudgetAdmission {
        admission_id: [0x50; 32],
        activation_units: 1,
        remaining_episode_units: 2,
        remaining_model_calls: 3,
        remaining_actions: 4,
    }
}

fn record(admission: BudgetAdmission) -> BTreeMap<String, Value> {
    let values = [
        admission.activation_units,
        admission.remaining_episode_units,
        admission.remaining_model_calls,
        admission.remaining_actions,
    ];
    let mut fields: BTreeMap<_, _> = COUNTERS
        .into_iter()
        .zip(values)
        .map(|(key, value)| (key.into(), Value::Integer(value.into())))
        .collect();
    fields.insert(
        "admission_id".into(),
        Value::Bytes(admission.admission_id.to_vec()),
    );
    fields
}

#[test]
fn complete_admission_vectors_match_independent_codec_hash_and_scalar_mapping() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/format/accounting.json")).unwrap();
    for fixture in fixtures["admissions"].as_array().unwrap() {
        let source = &fixture["record"];
        let admission = BudgetAdmission {
            admission_id: hex(source["admission_id"]["$bytes"].as_str().unwrap())
                .try_into()
                .unwrap(),
            activation_units: source["activation_units"].as_u64().unwrap(),
            remaining_episode_units: source["remaining_episode_units"].as_u64().unwrap(),
            remaining_model_calls: source["remaining_model_calls"].as_u64().unwrap(),
            remaining_actions: source["remaining_actions"].as_u64().unwrap(),
        };
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(record(admission)), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        assert_eq!(
            BudgetAdmission::decode(&expected, &DecodeLimits::default()),
            Ok(admission)
        );
        assert_eq!(
            admission.encode(&DecodeLimits::default()).unwrap(),
            expected
        );
    }
}

#[test]
fn all_admission_fields_are_required_and_unknown_fields_cannot_replace_them() {
    for key in [
        "admission_id",
        "activation_units",
        "remaining_episode_units",
        "remaining_model_calls",
        "remaining_actions",
    ] {
        let mut fields = record(sample());
        fields.remove(key);
        assert_eq!(
            BudgetAdmission::decode(&bytes(fields.clone()), &DecodeLimits::default()),
            Err(BudgetError::InvalidSchema)
        );
        fields.insert("unknown".into(), Value::Null);
        assert_eq!(
            BudgetAdmission::decode(&bytes(fields), &DecodeLimits::default()),
            Err(BudgetError::InvalidSchema)
        );
    }
    for data in [vec![0xf6], vec![0x80], vec![0x00]] {
        assert_eq!(
            BudgetAdmission::decode(&data, &DecodeLimits::default()),
            Err(BudgetError::InvalidSchema)
        );
    }
}

#[test]
fn admission_id_is_exactly_raw_id32_not_text_link_or_nullable() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for value in [
        Value::Null,
        Value::Bytes(vec![0; 31]),
        Value::Bytes(vec![0; 33]),
        Value::Text("admission".into()),
        link,
    ] {
        let mut fields = record(sample());
        fields.insert("admission_id".into(), value);
        assert_eq!(
            BudgetAdmission::decode(&bytes(fields), &DecodeLimits::default()),
            Err(BudgetError::InvalidSchema)
        );
    }
}

#[test]
fn every_counter_is_full_u64_without_null_or_scalar_coercion() {
    for key in COUNTERS {
        for value in [
            Value::Null,
            Value::Integer((-1).into()),
            Value::Bool(false),
            Value::Text("0".into()),
            Value::Bytes(vec![]),
        ] {
            let mut fields = record(sample());
            fields.insert(key.into(), value);
            assert_eq!(
                BudgetAdmission::decode(&bytes(fields), &DecodeLimits::default()),
                Err(BudgetError::InvalidSchema)
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
            let decoded = BudgetAdmission::decode(&expected, &DecodeLimits::default()).unwrap();
            assert_eq!(decoded.encode(&DecodeLimits::default()).unwrap(), expected);
        }
    }
}

#[test]
fn admission_byte_depth_and_collection_bounds_are_caller_supplied() {
    let admission = sample();
    let encoded = admission.encode(&DecodeLimits::default()).unwrap();
    let exact = DecodeLimits {
        max_block_bytes: encoded.len() as u64,
        max_depth: 2,
        max_collection_entries: 5,
    };
    assert_eq!(admission.encode(&exact).unwrap(), encoded);
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
        assert_eq!(admission.encode(&bounds), Err(BudgetError::Codec(error)));
        assert_eq!(
            BudgetAdmission::decode(&encoded, &bounds),
            Err(BudgetError::Codec(error))
        );
    }
}

#[test]
fn malformed_truncated_duplicate_nonminimal_and_trailing_admissions_reject() {
    let encoded = sample().encode(&DecodeLimits::default()).unwrap();
    for end in 0..encoded.len() {
        assert!(BudgetAdmission::decode(&encoded[..end], &DecodeLimits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a26c61646d697373696f6e5f6964f66c61646d697373696f6e5f6964f6"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 5], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            BudgetAdmission::decode(&data, &DecodeLimits::default()),
            Err(BudgetError::Codec(error))
        );
    }
}
