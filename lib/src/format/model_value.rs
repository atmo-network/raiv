//! Owns canonical model values, not runtime Bytes/Links, memory planes, or tool schemas.
//! The owned representation is a flat byte buffer; encoding uses borrowed input and an
//! iterative stack, so neither materialization nor destruction needs recursive trees.

use super::{
    try_push as push, validate_model_value, writer::BoundedWriter, CodecError, DecodeLimits,
};
use alloc::vec::{IntoIter, Vec};
use core::slice::Iter;
use minicbor::Encoder;

/// Borrowed JSON-compatible input to canonical encoding.
///
/// Integers, collection lengths, depth, and encoded bytes are checked by
/// `ModelValue::encode`. Map input order is arbitrary; duplicate keys reject.
#[derive(Clone, Copy, Debug)]
pub enum ValueRef<'a> {
    Null,
    Bool(bool),
    Integer(i64),
    Text(&'a str),
    List(&'a [ValueRef<'a>]),
    Map(&'a [(&'a str, ValueRef<'a>)]),
}

/// An owned canonical model value, validated before copying any input payload.
///
/// This contains no runtime Bytes/Links or float values. Its exact encoded bytes
/// define equality without Unicode normalization or insertion-order dependence.
/// It does not by itself satisfy sparse null-free semantic Memory invariants.
#[derive(Debug, Eq, PartialEq)]
pub struct ModelValue(Vec<u8>);

impl ModelValue {
    pub fn decode(bytes: &[u8], limits: &DecodeLimits) -> Result<Self, CodecError> {
        validate_model_value(bytes, limits)?;
        Ok(Self(super::copy_validated(bytes)?))
    }

    /// Encode borrowed input into bounded canonical bytes, independent of map input order.
    ///
    /// ```
    /// use raiv::format::{DecodeLimits, ModelValue, ValueRef};
    /// let entries = [("aa", ValueRef::Bool(true)), ("b", ValueRef::Null)];
    /// let value = ModelValue::encode(ValueRef::Map(&entries), &DecodeLimits::default())?;
    /// assert_eq!(value.as_bytes(), &[0xa2, 0x61, b'b', 0xf6, 0x62, b'a', b'a', 0xf5]);
    /// # Ok::<(), raiv::format::CodecError>(())
    /// ```
    pub fn encode(value: ValueRef<'_>, limits: &DecodeLimits) -> Result<Self, CodecError> {
        let bytes = encode(value, limits)?;
        Ok(Self(bytes))
    }

    /// Validate and adopt one encoder-owned buffer without a second ownership copy.
    pub(crate) fn from_encoded(bytes: Vec<u8>, limits: &DecodeLimits) -> Result<Self, CodecError> {
        validate_model_value(&bytes, limits)?;
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

enum Frame<'a> {
    Value(ValueRef<'a>, u64),
    List(Iter<'a, ValueRef<'a>>, u64),
    Map(IntoIter<(&'a str, ValueRef<'a>)>, u64),
}

fn collection(length: usize, depth: u64, limits: &DecodeLimits) -> Result<(), CodecError> {
    if length as u64 > limits.max_collection_entries {
        return Err(CodecError::CollectionLimit);
    }
    if length != 0 && depth >= limits.max_depth {
        return Err(CodecError::DepthLimit);
    }
    Ok(())
}

fn encode(value: ValueRef<'_>, limits: &DecodeLimits) -> Result<Vec<u8>, CodecError> {
    if limits.max_depth == 0 {
        return Err(CodecError::DepthLimit);
    }
    let writer = BoundedWriter {
        bytes: Vec::new(),
        max_bytes: limits.max_block_bytes,
    };
    let mut encoder = Encoder::new(writer);
    let mut stack = Vec::new();
    push(&mut stack, Frame::Value(value, 1))?;
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::List(mut values, depth) => {
                if let Some(value) = values.next() {
                    push(&mut stack, Frame::List(values, depth))?;
                    push(&mut stack, Frame::Value(*value, depth))?;
                }
            }
            Frame::Map(mut entries, depth) => {
                if let Some((key, value)) = entries.next() {
                    encoder.str(key)?;
                    push(&mut stack, Frame::Map(entries, depth))?;
                    push(&mut stack, Frame::Value(value, depth))?;
                }
            }
            Frame::Value(value, depth) => match value {
                ValueRef::Null => {
                    encoder.null()?;
                }
                ValueRef::Bool(value) => {
                    encoder.bool(value)?;
                }
                ValueRef::Integer(value) => {
                    const MAX: i64 = (1 << 53) - 1;
                    if !(-MAX..=MAX).contains(&value) {
                        return Err(CodecError::ModelIntegerRange);
                    }
                    encoder.i64(value)?;
                }
                ValueRef::Text(value) => {
                    encoder.str(value)?;
                }
                ValueRef::List(values) => {
                    collection(values.len(), depth, limits)?;
                    encoder.array(values.len() as u64)?;
                    if !values.is_empty() {
                        push(&mut stack, Frame::List(values.iter(), depth + 1))?;
                    }
                }
                ValueRef::Map(entries) => {
                    collection(entries.len(), depth, limits)?;
                    encoder.map(entries.len() as u64)?;
                    // Bound sort scratch before allocation: every key has a header and every
                    // value takes at least one byte, even before visiting nested values.
                    let minimum_body = entries.iter().try_fold(0usize, |length, (key, _)| {
                        length
                            .checked_add(key.len())
                            .and_then(|n| n.checked_add(2))
                            .ok_or(CodecError::ByteLimit)
                    })?;
                    encoder.writer().check_additional(minimum_body)?;
                    let mut sorted = Vec::new();
                    sorted
                        .try_reserve_exact(entries.len())
                        .map_err(|_| CodecError::AllocationFailure)?;
                    sorted.extend_from_slice(entries);
                    // Minimal CBOR text headers sort by UTF-8 byte length, then payload.
                    sorted.sort_unstable_by_key(|(key, _)| (key.len(), *key));
                    if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                        return Err(CodecError::DuplicateKey);
                    }
                    if !sorted.is_empty() {
                        push(&mut stack, Frame::Map(sorted.into_iter(), depth + 1))?;
                    }
                }
            },
        }
    }
    Ok(encoder.into_writer().bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::String, vec};
    use minicbor::encode::Write;
    use serde_json::Value;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn canonical_map_bytes_match_independent_fixture_for_all_input_orders() {
        let permutations = [
            [
                ("aa", ValueRef::Integer(3)),
                ("b", ValueRef::Integer(2)),
                ("a", ValueRef::Integer(1)),
            ],
            [
                ("a", ValueRef::Integer(1)),
                ("b", ValueRef::Integer(2)),
                ("aa", ValueRef::Integer(3)),
            ],
            [
                ("b", ValueRef::Integer(2)),
                ("aa", ValueRef::Integer(3)),
                ("a", ValueRef::Integer(1)),
            ],
            [
                ("aa", ValueRef::Integer(3)),
                ("a", ValueRef::Integer(1)),
                ("b", ValueRef::Integer(2)),
            ],
            [
                ("a", ValueRef::Integer(1)),
                ("aa", ValueRef::Integer(3)),
                ("b", ValueRef::Integer(2)),
            ],
            [
                ("b", ValueRef::Integer(2)),
                ("a", ValueRef::Integer(1)),
                ("aa", ValueRef::Integer(3)),
            ],
        ];
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/dasl-testing/map_keys.json"))
                .unwrap();
        let expected = hex(fixture[3]["data"].as_str().unwrap());
        for entries in permutations {
            let value =
                ModelValue::encode(ValueRef::Map(&entries), &DecodeLimits::default()).unwrap();
            assert_eq!(value.as_bytes(), expected);
            assert_eq!(
                ModelValue::decode(&expected, &DecodeLimits::default()),
                Ok(value)
            );
        }
    }

    #[test]
    fn integers_match_independent_vectors_at_both_model_boundaries() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../fixtures/dasl-testing/integer_range.json"
        ))
        .unwrap();
        let limits = DecodeLimits::default();
        for (index, integer) in [(2, (1 << 53) - 1), (4, -((1 << 53) - 1))] {
            let expected = hex(fixtures[index]["data"].as_str().unwrap());
            let value = ModelValue::encode(ValueRef::Integer(integer), &limits).unwrap();
            assert_eq!(value.as_bytes(), expected);
            assert_eq!(ModelValue::decode(&expected, &limits), Ok(value));
        }
        for integer in [1 << 53, -(1 << 53), i64::MAX, i64::MIN] {
            assert_eq!(
                ModelValue::encode(ValueRef::Integer(integer), &limits),
                Err(CodecError::ModelIntegerRange)
            );
        }
    }

    #[test]
    fn scalar_encodings_match_pinned_bytes_without_unicode_normalization() {
        let limits = DecodeLimits::default();
        for (value, expected) in [
            (ValueRef::Null, "f6"),
            (ValueRef::Bool(false), "f4"),
            (ValueRef::Bool(true), "f5"),
            (ValueRef::Integer(0), "00"),
            (ValueRef::Integer(-1), "20"),
            (ValueRef::Integer(24), "1818"),
            (ValueRef::Integer(-25), "3818"),
            (ValueRef::Text("e\u{301}"), "6365cc81"),
            (ValueRef::List(&[]), "80"),
            (ValueRef::Map(&[]), "a0"),
        ] {
            let encoded = ModelValue::encode(value, &limits).unwrap();
            assert_eq!(encoded.as_bytes(), hex(expected));
            assert_eq!(validate_model_value(encoded.as_bytes(), &limits), Ok(()));
        }
    }

    #[test]
    fn map_order_uses_utf8_byte_length_and_minimal_length_headers() {
        let short = "z".repeat(23);
        let long = "a".repeat(24);
        let entries = [
            (long.as_str(), ValueRef::Null),
            ("é", ValueRef::Null),
            (short.as_str(), ValueRef::Null),
            ("b", ValueRef::Null),
        ];
        let value = ModelValue::encode(ValueRef::Map(&entries), &DecodeLimits::default()).unwrap();
        // Independently assembled preferred CBOR: b, é, 23-byte key, 24-byte key.
        let expected = format!(
            "a46162f662c3a9f677{}f67818{}f6",
            "7a".repeat(23),
            "61".repeat(24)
        );
        assert_eq!(value.as_bytes(), hex(&expected));
        assert_eq!(
            validate_model_value(value.as_bytes(), &DecodeLimits::default()),
            Ok(())
        );
    }

    #[test]
    fn nested_collections_are_canonical_and_owned_bytes_survive_input_drop() {
        let owned = {
            let text = String::from("hi");
            let children = [ValueRef::Text(&text), ValueRef::Bool(true)];
            let entries = [("b", ValueRef::List(&children)), ("aa", ValueRef::Null)];
            ModelValue::encode(ValueRef::Map(&entries), &DecodeLimits::default()).unwrap()
        };
        let bytes = hex("a2616282626869f5626161f6");
        assert_eq!(owned.as_bytes(), bytes);
        let decoded = ModelValue::decode(&bytes, &DecodeLimits::default()).unwrap();
        assert_eq!(decoded, owned);
        assert_eq!(decoded.into_bytes(), bytes);
    }

    #[test]
    fn decoding_rejects_noncanonical_or_runtime_values_before_owning_bytes() {
        for data in [
            "1800",
            "a2616100616101",
            "40",
            "d82a4100",
            "fb0000000000000000",
            "00ff",
            "",
        ] {
            assert!(
                ModelValue::decode(&hex(data), &DecodeLimits::default()).is_err(),
                "{data}"
            );
        }
    }

    #[test]
    fn encoding_and_decoding_share_exact_byte_depth_and_count_limits() {
        let limits = DecodeLimits {
            max_block_bytes: 4,
            max_depth: 2,
            max_collection_entries: 1,
        };
        let entries = [("a", ValueRef::Integer(0))];
        let value = ModelValue::encode(ValueRef::Map(&entries), &limits).unwrap();
        assert_eq!(value.as_bytes(), hex("a1616100"));
        assert_eq!(ModelValue::decode(value.as_bytes(), &limits), Ok(value));
        let short = DecodeLimits {
            max_block_bytes: 3,
            ..limits
        };
        assert_eq!(
            ModelValue::encode(ValueRef::Map(&entries), &short),
            Err(CodecError::ByteLimit)
        );
        assert_eq!(
            ModelValue::decode(&hex("a1616100"), &short),
            Err(CodecError::ByteLimit)
        );
        let shallow = DecodeLimits {
            max_depth: 1,
            ..limits
        };
        assert_eq!(
            ModelValue::encode(ValueRef::Map(&entries), &shallow),
            Err(CodecError::DepthLimit)
        );
        let children = [ValueRef::Null, ValueRef::Null];
        assert_eq!(
            ModelValue::encode(ValueRef::List(&children), &limits),
            Err(CodecError::CollectionLimit)
        );
        let empty = DecodeLimits {
            max_collection_entries: 0,
            max_depth: 1,
            ..limits
        };
        assert_eq!(
            ModelValue::encode(ValueRef::List(&[]), &empty)
                .unwrap()
                .as_bytes(),
            &[0x80]
        );
        let no_depth = DecodeLimits {
            max_depth: 0,
            ..limits
        };
        assert_eq!(
            ModelValue::encode(ValueRef::Null, &no_depth),
            Err(CodecError::DepthLimit)
        );
        let no_bytes = DecodeLimits {
            max_block_bytes: 0,
            ..limits
        };
        assert_eq!(
            ModelValue::encode(ValueRef::Null, &no_bytes),
            Err(CodecError::ByteLimit)
        );
    }

    #[test]
    fn duplicate_keys_reject_at_every_depth_without_normalizing_distinct_keys() {
        let entries = [("a", ValueRef::Null), ("a", ValueRef::Bool(false))];
        let values = [ValueRef::Map(&entries)];
        assert_eq!(
            ModelValue::encode(ValueRef::List(&values), &DecodeLimits::default()),
            Err(CodecError::DuplicateKey)
        );
        let distinct = [("é", ValueRef::Null), ("e\u{301}", ValueRef::Null)];
        assert!(ModelValue::encode(ValueRef::Map(&distinct), &DecodeLimits::default()).is_ok());
    }

    #[test]
    fn independent_codec_matches_integer_width_partitions_and_nested_maps() {
        let limits = DecodeLimits::default();
        for magnitude in [
            0,
            23,
            24,
            255,
            256,
            65_535,
            65_536,
            4_294_967_295,
            4_294_967_296,
            (1 << 53) - 1,
        ] {
            for integer in [magnitude, -magnitude] {
                let value = ModelValue::encode(ValueRef::Integer(integer), &limits).unwrap();
                let oracle = crate::format::oracle::json_bytes(&serde_json::json!(integer));
                assert_eq!(value.as_bytes(), oracle, "integer {integer}");
            }
        }
        let children = [
            ValueRef::Integer(24),
            ValueRef::Text("e\u{301}"),
            ValueRef::Null,
        ];
        let short = "z".repeat(23);
        let long = "a".repeat(24);
        let nested = [
            ("aa", ValueRef::Bool(true)),
            ("b", ValueRef::List(&children)),
        ];
        let entries = [
            (long.as_str(), ValueRef::Map(&nested)),
            ("é", ValueRef::Null),
            (short.as_str(), ValueRef::Bool(false)),
        ];
        let value = ModelValue::encode(ValueRef::Map(&entries), &limits).unwrap();
        let mut json = serde_json::Map::new();
        json.insert(
            long,
            serde_json::json!({"aa": true, "b": [24, "e\u{301}", null]}),
        );
        json.insert("é".into(), serde_json::Value::Null);
        json.insert(short, serde_json::Value::Bool(false));
        let oracle = crate::format::oracle::json_bytes(&serde_json::Value::Object(json));
        assert_eq!(value.as_bytes(), oracle);
        assert_eq!(ModelValue::decode(&oracle, &limits), Ok(value));
    }

    #[test]
    fn cyclic_borrowed_inputs_are_bounded_and_deep_owned_values_drop_flatly() {
        static CYCLE: [ValueRef<'static>; 1] = [ValueRef::List(&CYCLE)];
        assert_eq!(
            ModelValue::encode(ValueRef::List(&CYCLE), &DecodeLimits::default()),
            Err(CodecError::DepthLimit)
        );
        let limits = DecodeLimits {
            max_depth: u64::MAX,
            max_block_bytes: 10_000,
            ..DecodeLimits::default()
        };
        assert_eq!(
            ModelValue::encode(ValueRef::List(&CYCLE), &limits),
            Err(CodecError::ByteLimit)
        );
        let mut bytes = vec![0x81; 9999];
        bytes.push(0);
        let owned = ModelValue::decode(&bytes, &limits).unwrap();
        assert_eq!(owned.as_bytes(), bytes);
        drop(owned);
    }

    #[test]
    fn map_sort_scratch_is_rejected_when_keys_alone_exceed_the_byte_budget() {
        let long = "a".repeat(4096);
        let entries = [(long.as_str(), ValueRef::Null)];
        let limits = DecodeLimits {
            max_block_bytes: 4096,
            ..DecodeLimits::default()
        };
        assert_eq!(
            ModelValue::encode(ValueRef::Map(&entries), &limits),
            Err(CodecError::ByteLimit)
        );
    }

    #[test]
    fn byte_writer_never_commits_an_over_budget_write() {
        let mut writer = BoundedWriter {
            bytes: vec![0],
            max_bytes: 2,
        };
        assert_eq!(writer.write_all(&[1, 2]), Err(CodecError::ByteLimit));
        assert_eq!(writer.bytes, &[0]);
        assert_eq!(writer.write_all(&[1]), Ok(()));
        assert_eq!(writer.bytes, &[0, 1]);
    }
}
