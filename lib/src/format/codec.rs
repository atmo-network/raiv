//! Private CBOR preflight: borrows payloads and allocates only an iterative depth stack.
//! Typed schema/graph validation and canonical state encoding are separate responsibilities.

use super::{try_push as push, DaslCid};
use alloc::vec::Vec;
use core::{cmp::Ordering, fmt};
use minicbor::Decoder;

/// Bounds for one encoded object, checked before materializing owned values.
///
/// Depth counts the root as one and each nested item as one more, including a
/// tag-42 byte-string payload. Map keys and values have the same child depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    pub max_block_bytes: u64,
    pub max_depth: u64,
    pub max_collection_entries: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_block_bytes: 65_536,
            max_depth: 32,
            max_collection_entries: 4096,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    ByteLimit,
    DepthLimit,
    CollectionLimit,
    Malformed,
    NonCanonical,
    DuplicateKey,
    InvalidMapKey,
    UnsupportedValue,
    InvalidLink,
    ModelIntegerRange,
    TrailingBytes,
    AllocationFailure,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ByteLimit => "encoded object exceeds byte limit",
            Self::DepthLimit => "encoded object exceeds depth limit",
            Self::CollectionLimit => "collection exceeds entry limit",
            Self::Malformed => "malformed or truncated CBOR",
            Self::NonCanonical => "noncanonical CBOR width or map ordering",
            Self::DuplicateKey => "duplicate map key",
            Self::InvalidMapKey => "map key is not text",
            Self::UnsupportedValue => "value is outside the Raiv wire profile",
            Self::InvalidLink => "invalid structural DRISL link",
            Self::ModelIntegerRange => "integer is outside the model-value range",
            Self::TrailingBytes => "bytes remain after the root object",
            Self::AllocationFailure => "cannot allocate bounded codec data",
        })
    }
}

impl core::error::Error for CodecError {}

impl From<minicbor::encode::Error<CodecError>> for CodecError {
    fn from(error: minicbor::encode::Error<CodecError>) -> Self {
        core::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<Self>())
            .copied()
            .unwrap_or(Self::Malformed)
    }
}

impl From<minicbor::decode::Error> for CodecError {
    fn from(_: minicbor::decode::Error) -> Self {
        Self::Malformed
    }
}

/// Validate one canonical Raiv wire object without copying its payloads.
///
/// This is preflight only: a scalar can be valid wire data without being a
/// structural block. Callers must additionally validate the exact typed schema,
/// linked block kinds, closure, and authority. Every tag-42 target uses codec 0x71.
pub fn validate_drisl(bytes: &[u8], limits: &DecodeLimits) -> Result<(), CodecError> {
    validate(bytes, limits, Scope::Structural)
}

/// Validate a canonical JSON-compatible model value: no Bytes, Links, floats,
/// or integers outside ±(2^53−1). Does not impose a particular memory schema.
pub fn validate_model_value(bytes: &[u8], limits: &DecodeLimits) -> Result<(), CodecError> {
    validate(bytes, limits, Scope::Model)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Scope {
    Structural,
    Model,
}

struct Frame<'a> {
    remaining: u64,
    map: bool,
    next_key: bool,
    previous_key: Option<&'a [u8]>,
}

impl Frame<'_> {
    fn new(remaining: u64, map: bool) -> Self {
        Self {
            remaining,
            map,
            next_key: map,
            previous_key: None,
        }
    }
}

// minicbor parses CBOR primitives; this narrow guard rejects its permissive widths.
fn minimal(bytes: &[u8], start: usize, value: u64) -> Result<(), CodecError> {
    let expected = match value {
        0..=23 => value as u8,
        24..=255 => 24,
        256..=65_535 => 25,
        65_536..=4_294_967_295 => 26,
        _ => 27,
    };
    if bytes[start] & 0x1f != expected {
        return Err(CodecError::NonCanonical);
    }
    Ok(())
}

fn validate(bytes: &[u8], limits: &DecodeLimits, scope: Scope) -> Result<(), CodecError> {
    if bytes.len() as u64 > limits.max_block_bytes {
        return Err(CodecError::ByteLimit);
    }
    if limits.max_depth == 0 {
        return Err(CodecError::DepthLimit);
    }
    let mut decoder = Decoder::new(bytes);
    let mut stack = Vec::new();
    push(&mut stack, Frame::new(1, false))?;
    while !stack.is_empty() {
        let depth = stack.len() as u64;
        let frame = stack.last_mut().expect("nonempty frame stack");
        if frame.remaining == 0 {
            stack.pop();
            continue;
        }
        let start = decoder.position();
        let first = *bytes.get(start).ok_or(CodecError::Malformed)?;
        if frame.map && frame.next_key {
            if first >> 5 != 3 {
                return Err(CodecError::InvalidMapKey);
            }
            let key = decoder.str()?;
            minimal(bytes, start, key.len() as u64)?;
            let encoded = &bytes[start..decoder.position()];
            if let Some(previous) = frame.previous_key {
                match previous.cmp(encoded) {
                    Ordering::Equal => return Err(CodecError::DuplicateKey),
                    Ordering::Greater => return Err(CodecError::NonCanonical),
                    Ordering::Less => {}
                }
            }
            frame.previous_key = Some(encoded);
            frame.remaining -= 1;
            frame.next_key = false;
            continue;
        }
        frame.remaining -= 1;
        frame.next_key = frame.map;
        match first >> 5 {
            0 | 1 => {
                let value = i128::from(decoder.int()?);
                let magnitude = if value < 0 { -1 - value } else { value } as u64;
                minimal(bytes, start, magnitude)?;
                const MAX_MODEL_INTEGER: i128 = (1 << 53) - 1;
                if scope == Scope::Model
                    && !(-MAX_MODEL_INTEGER..=MAX_MODEL_INTEGER).contains(&value)
                {
                    return Err(CodecError::ModelIntegerRange);
                }
            }
            2 => {
                if scope == Scope::Model {
                    return Err(CodecError::UnsupportedValue);
                }
                let payload = decoder.bytes()?;
                minimal(bytes, start, payload.len() as u64)?;
            }
            3 => {
                let text = decoder.str()?;
                minimal(bytes, start, text.len() as u64)?;
            }
            4 | 5 => {
                let map = first >> 5 == 5;
                let count = if map {
                    decoder.map()?
                } else {
                    decoder.array()?
                }
                .ok_or(CodecError::UnsupportedValue)?;
                minimal(bytes, start, count)?;
                if count > limits.max_collection_entries {
                    return Err(CodecError::CollectionLimit);
                }
                let remaining = count
                    .checked_mul(if map { 2 } else { 1 })
                    .ok_or(CodecError::CollectionLimit)?;
                // Every child consumes at least one byte; no count-driven allocation.
                if remaining > (bytes.len() - decoder.position()) as u64 {
                    return Err(CodecError::Malformed);
                }
                if remaining != 0 {
                    if depth >= limits.max_depth {
                        return Err(CodecError::DepthLimit);
                    }
                    push(&mut stack, Frame::new(remaining, map))?;
                }
            }
            6 => {
                if scope == Scope::Model {
                    return Err(CodecError::UnsupportedValue);
                }
                let tag = decoder.tag()?.as_u64();
                if tag != 42 {
                    return Err(CodecError::UnsupportedValue);
                }
                minimal(bytes, start, tag)?;
                if depth >= limits.max_depth {
                    return Err(CodecError::DepthLimit);
                }
                let payload_start = decoder.position();
                let payload = decoder.bytes()?;
                minimal(bytes, payload_start, payload.len() as u64)?;
                DaslCid::from_link_payload(payload)?;
            }
            7 => match first {
                0xf4 | 0xf5 => {
                    decoder.bool()?;
                }
                0xf6 => {
                    decoder.null()?;
                }
                _ => return Err(CodecError::UnsupportedValue),
            },
            _ => unreachable!("CBOR has exactly eight major types"),
        }
    }
    if decoder.position() != bytes.len() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{collections::BTreeMap, string::ToString, vec};
    use serde_json::Value;

    fn hex(text: &str) -> Vec<u8> {
        assert_eq!(text.len() % 2, 0);
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn pinned_raiv_profile_runs_all_125_rows() {
        let profile: Value =
            serde_json::from_str(include_str!("../../../fixtures/raiv-profile.json")).unwrap();
        let sources = [
            (
                "dasl-testing/cid.json",
                include_str!("../../../fixtures/dasl-testing/cid.json"),
            ),
            (
                "dasl-testing/concat.json",
                include_str!("../../../fixtures/dasl-testing/concat.json"),
            ),
            (
                "dasl-testing/floats.json",
                include_str!("../../../fixtures/dasl-testing/floats.json"),
            ),
            (
                "dasl-testing/indefinite.json",
                include_str!("../../../fixtures/dasl-testing/indefinite.json"),
            ),
            (
                "dasl-testing/integer_range.json",
                include_str!("../../../fixtures/dasl-testing/integer_range.json"),
            ),
            (
                "dasl-testing/map_keys.json",
                include_str!("../../../fixtures/dasl-testing/map_keys.json"),
            ),
            (
                "dasl-testing/numeric_reduction.json",
                include_str!("../../../fixtures/dasl-testing/numeric_reduction.json"),
            ),
            (
                "dasl-testing/short_form.json",
                include_str!("../../../fixtures/dasl-testing/short_form.json"),
            ),
            (
                "dasl-testing/simple.json",
                include_str!("../../../fixtures/dasl-testing/simple.json"),
            ),
            (
                "dasl-testing/tags.json",
                include_str!("../../../fixtures/dasl-testing/tags.json"),
            ),
            (
                "dasl-testing/utf8.json",
                include_str!("../../../fixtures/dasl-testing/utf8.json"),
            ),
            (
                "raiv-format.json",
                include_str!("../../../fixtures/raiv-format.json"),
            ),
        ];
        let families = profile["families"].as_array().unwrap();
        assert_eq!(families.len(), sources.len());
        let mut seen = BTreeMap::new();
        let mut count = 0;
        for (family, (file, source)) in families.iter().zip(sources) {
            assert_eq!(family["file"], file);
            let rows: Value = serde_json::from_str(source).unwrap();
            let rows = rows.as_array().unwrap();
            let selections = family["cases"].as_array().unwrap();
            assert_eq!(rows.len(), selections.len());
            let scope = match family["scope"].as_str().unwrap() {
                "structural_wire" => Scope::Structural,
                "model_value" => Scope::Model,
                _ => panic!("unknown fixture scope"),
            };
            for (index, (row, selection)) in rows.iter().zip(selections).enumerate() {
                assert_eq!(selection["index"].as_u64(), Some(index as u64));
                assert_eq!(selection["name"], row["name"]);
                let data = row["data"].as_str().unwrap();
                let accept = match selection["expected"].as_str().unwrap() {
                    "accept" => true,
                    "reject" => false,
                    _ => panic!("unknown fixture expectation"),
                };
                let key = (
                    family["scope"].as_str().unwrap().to_string(),
                    data.to_string(),
                );
                if let Some(previous) = seen.insert(key, accept) {
                    assert_eq!(previous, accept, "conflicting expectations: {file}:{index}");
                }
                let result = validate(&hex(data), &DecodeLimits::default(), scope);
                assert_eq!(
                    result.is_ok(),
                    accept,
                    "{file}:{index}: {}: {result:?}",
                    row["name"]
                );
                count += 1;
            }
        }
        assert_eq!(count, 125);
    }

    #[test]
    fn depth_is_bounded_including_keys_and_link_payloads() {
        let limits = DecodeLimits {
            max_depth: 1,
            ..DecodeLimits::default()
        };
        for bytes in [hex("00"), hex("80"), hex("a0")] {
            assert_eq!(validate_drisl(&bytes, &limits), Ok(()));
        }
        for bytes in [hex("8100"), hex("a1616100")] {
            assert_eq!(validate_drisl(&bytes, &limits), Err(CodecError::DepthLimit));
        }
        let link = hex(
            "d82a58250001711220c19a797fa1fd590cd2e5b42d1cf5f246e29b91684e2f87404b81dc345c7a56a0",
        );
        assert_eq!(validate_drisl(&link, &limits), Err(CodecError::DepthLimit));
        let limits = DecodeLimits {
            max_depth: 2,
            ..limits
        };
        assert_eq!(validate_drisl(&link, &limits), Ok(()));
        assert_eq!(
            validate_drisl(&hex("818100"), &limits),
            Err(CodecError::DepthLimit)
        );
        let limits = DecodeLimits {
            max_depth: 0,
            ..limits
        };
        assert_eq!(validate_drisl(&[0], &limits), Err(CodecError::DepthLimit));
    }

    #[test]
    fn collection_and_byte_limits_apply_at_exact_boundaries() {
        let limits = DecodeLimits {
            max_collection_entries: 1,
            max_block_bytes: 4,
            ..DecodeLimits::default()
        };
        assert_eq!(validate_drisl(&hex("a1616100"), &limits), Ok(()));
        assert_eq!(
            validate_drisl(&hex("820000"), &limits),
            Err(CodecError::CollectionLimit)
        );
        assert_eq!(
            validate_drisl(&hex("a2616100616200"), &limits),
            Err(CodecError::ByteLimit)
        );
        let limits = DecodeLimits {
            max_block_bytes: u64::MAX,
            ..limits
        };
        assert_eq!(
            validate_drisl(&hex("a2616100616200"), &limits),
            Err(CodecError::CollectionLimit)
        );
        let limits = DecodeLimits {
            max_collection_entries: 0,
            ..limits
        };
        assert_eq!(validate_drisl(&hex("80"), &limits), Ok(()));
        assert_eq!(
            validate_drisl(&hex("8100"), &limits),
            Err(CodecError::CollectionLimit)
        );
        let limits = DecodeLimits {
            max_block_bytes: 0,
            ..limits
        };
        assert_eq!(validate_drisl(&[0], &limits), Err(CodecError::ByteLimit));
    }

    #[test]
    fn malicious_counts_lengths_and_truncation_never_allocate_payloads() {
        let limits = DecodeLimits {
            max_collection_entries: u64::MAX,
            ..DecodeLimits::default()
        };
        for data in [
            "9bffffffffffffffff",
            "bbffffffffffffffff",
            "5bffffffffffffffff",
            "7bffffffffffffffff",
            "",
        ] {
            assert!(validate_drisl(&hex(data), &limits).is_err(), "{data}");
        }
        let data = hex("a161618201626869");
        assert_eq!(validate_drisl(&data, &limits), Ok(()));
        for length in 0..data.len() {
            assert!(
                validate_drisl(&data[..length], &limits).is_err(),
                "prefix {length}"
            );
        }
    }

    #[test]
    fn nested_map_order_is_local_and_encoded_not_string_order() {
        assert_eq!(
            validate_drisl(
                &hex("a26161a261620062616100616280"),
                &DecodeLimits::default()
            ),
            Ok(())
        );
        assert_eq!(
            validate_drisl(&hex("a2616100616101"), &DecodeLimits::default()),
            Err(CodecError::DuplicateKey)
        );
        assert_eq!(
            validate_drisl(&hex("a2616100610061"), &DecodeLimits::default()),
            Err(CodecError::NonCanonical)
        );
    }

    #[test]
    fn model_restrictions_apply_inside_containers_without_narrowing_runtime_u64() {
        let limits = DecodeLimits::default();
        for data in [
            "8140",
            "a1616140",
            "1b0020000000000000",
            "3b001fffffffffffff",
            "fb0000000000000000",
        ] {
            assert!(validate_model_value(&hex(data), &limits).is_err(), "{data}");
        }
        for data in ["1b001fffffffffffff", "3b001ffffffffffffe", "a1616182f6f5"] {
            assert_eq!(validate_model_value(&hex(data), &limits), Ok(()));
        }
        assert_eq!(validate_drisl(&hex("1bffffffffffffffff"), &limits), Ok(()));
    }

    #[test]
    fn deeply_nested_input_uses_a_heap_stack_not_recursive_calls() {
        let mut bytes = vec![0x81; 10_000];
        bytes.push(0);
        assert_eq!(
            validate_drisl(&bytes, &DecodeLimits::default()),
            Err(CodecError::DepthLimit)
        );
        let limits = DecodeLimits {
            max_depth: 10_001,
            ..DecodeLimits::default()
        };
        assert_eq!(validate_drisl(&bytes, &limits), Ok(()));
    }
}
