use super::*;
use crate::format::oracle::{link, map_bytes as bytes};
use alloc::{collections::BTreeMap, string::String, vec};
use ciborium::value::Value;
use sha2::{Digest, Sha256};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn record(position: Position) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("domain".into(), Value::Bytes(position.domain.to_vec())),
        ("number".into(), Value::Integer(position.number.into())),
        (
            "reference".into(),
            position
                .reference
                .map(|v| Value::Bytes(v.to_vec()))
                .unwrap_or(Value::Null),
        ),
    ])
}

fn sample() -> Position {
    Position {
        domain: [0x30; 32],
        number: 0,
        reference: None,
    }
}

#[test]
fn complete_position_vectors_match_independent_codec_and_hash() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/format/terminal.json")).unwrap();
    for (i, fixture) in fixtures["positions"].as_array().unwrap().iter().enumerate() {
        let expected = hex(fixture["hex"].as_str().unwrap());
        let position = Position {
            number: if i == 0 { 0 } else { u64::MAX },
            reference: if i == 0 { None } else { Some([0x40; 32]) },
            ..sample()
        };
        assert_eq!(bytes(record(position)), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        assert_eq!(
            Position::decode(&expected, &DecodeLimits::default()),
            Ok(position)
        );
        assert_eq!(position.encode(&DecodeLimits::default()).unwrap(), expected);
    }
}

#[test]
fn position_fields_are_closed_required_and_never_defaulted() {
    for key in ["domain", "number", "reference"] {
        let mut fields = record(sample());
        fields.remove(key);
        assert_eq!(
            Position::decode(&bytes(fields.clone()), &DecodeLimits::default()),
            Err(PositionError::InvalidSchema)
        );
        fields.insert("unknown".into(), Value::Null);
        assert_eq!(
            Position::decode(&bytes(fields), &DecodeLimits::default()),
            Err(PositionError::InvalidSchema)
        );
    }
    for data in [vec![0xf6], vec![0x80], vec![0x00]] {
        assert_eq!(
            Position::decode(&data, &DecodeLimits::default()),
            Err(PositionError::InvalidSchema)
        );
    }
}

#[test]
fn position_scalars_are_exact_raw_ids_nullable_reference_and_full_u64() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for key in ["domain", "reference"] {
        for value in [
            Value::Bytes(vec![0; 31]),
            Value::Bytes(vec![0; 33]),
            Value::Text("id".into()),
            link.clone(),
        ] {
            let mut fields = record(sample());
            fields.insert(key.into(), value);
            assert_eq!(
                Position::decode(&bytes(fields), &DecodeLimits::default()),
                Err(PositionError::InvalidSchema)
            );
        }
    }
    for (key, value) in [
        ("domain", Value::Null),
        ("number", Value::Integer((-1).into())),
        ("number", Value::Text("0".into())),
        ("number", Value::Null),
    ] {
        let mut fields = record(sample());
        fields.insert(key.into(), value);
        assert_eq!(
            Position::decode(&bytes(fields), &DecodeLimits::default()),
            Err(PositionError::InvalidSchema)
        );
    }
    for number in [
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
        let position = Position { number, ..sample() };
        assert_eq!(
            Position::decode(
                &position.encode(&DecodeLimits::default()).unwrap(),
                &DecodeLimits::default()
            ),
            Ok(position)
        );
    }
}

#[test]
fn position_byte_depth_and_collection_bounds_are_caller_owned() {
    let position = sample();
    let encoded = position.encode(&DecodeLimits::default()).unwrap();
    let exact = DecodeLimits {
        max_block_bytes: encoded.len() as u64,
        max_depth: 2,
        max_collection_entries: 3,
    };
    assert_eq!(position.encode(&exact).unwrap(), encoded);
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
                max_depth: 1,
                ..exact
            },
            CodecError::DepthLimit,
        ),
        (
            DecodeLimits {
                max_collection_entries: 2,
                ..exact
            },
            CodecError::CollectionLimit,
        ),
    ] {
        assert_eq!(position.encode(&bounds), Err(PositionError::Codec(error)));
        assert_eq!(
            Position::decode(&encoded, &bounds),
            Err(PositionError::Codec(error))
        );
    }
}

#[test]
fn malformed_noncanonical_duplicate_and_trailing_positions_reject() {
    let encoded = sample().encode(&DecodeLimits::default()).unwrap();
    for end in 0..encoded.len() {
        assert!(Position::decode(&encoded[..end], &DecodeLimits::default()).is_err());
    }
    for (data, error) in [
        (
            hex("a2666e756d62657200666e756d62657201"),
            CodecError::DuplicateKey,
        ),
        (
            [vec![0xb8, 3], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            Position::decode(&data, &DecodeLimits::default()),
            Err(PositionError::Codec(error))
        );
    }
}
