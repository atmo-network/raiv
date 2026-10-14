//! Owns the complete policy-limit data contract and demo baselines, not budget accounting.

use crate::format::{encode_unsigned_record, validate_drisl, CodecError, DecodeLimits, ValueView};
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitsError {
    Codec(CodecError),
    InvalidSchema,
}

impl From<CodecError> for LimitsError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for LimitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::InvalidSchema => {
                f.write_str("Limits requires exactly the eighteen unsigned policy fields")
            }
        }
    }
}

impl core::error::Error for LimitsError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// All normative `Limits` fields from specification §17, kept as full-width u64.
///
/// Defaults are initial demo baselines, not performance recommendations or permission
/// to change a restored Definition. Definition integration, coupled policy checks,
/// and budget accounting must enforce these fields at their respective boundaries.
/// Convert to usize only at an actual allocation/index boundary, using checked conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_memory_bytes: u64,
    pub max_observation_bytes: u64,
    pub max_receipt_bytes: u64,
    pub max_command_bytes: u64,
    pub max_block_bytes: u64,
    pub max_materialized_bytes: u64,
    pub max_blocks: u64,
    pub max_depth: u64,
    pub max_collection_entries: u64,
    pub max_context_bytes: u64,
    pub max_model_reply_bytes: u64,
    pub max_patch_entries: u64,
    pub history_episodes: u64,
    pub max_actions_per_episode: u64,
    pub max_model_calls_per_activation: u64,
    pub max_model_calls_per_episode: u64,
    pub max_local_tools_per_activation: u64,
    pub max_corrective_turns: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 4096,
            max_observation_bytes: 8192,
            max_receipt_bytes: 4096,
            max_command_bytes: 2048,
            max_block_bytes: 65_536,
            max_materialized_bytes: 4_194_304,
            max_blocks: 4096,
            max_depth: 32,
            max_collection_entries: 4096,
            max_context_bytes: 131_072,
            max_model_reply_bytes: 32_768,
            max_patch_entries: 32,
            history_episodes: 2,
            max_actions_per_episode: 8,
            max_model_calls_per_activation: 16,
            max_model_calls_per_episode: 64,
            max_local_tools_per_activation: 16,
            max_corrective_turns: 2,
        }
    }
}

impl Limits {
    /// Encode this complete policy subrecord with full-width runtime integers.
    pub fn encode(&self, limits: &DecodeLimits) -> Result<Vec<u8>, LimitsError> {
        Ok(encode_unsigned_record(self.wire_fields(), limits)?)
    }

    /// Decode with caller-supplied bootstrap bounds, not the untrusted record's limits.
    /// This validates data shape only; it does not activate or authorize the policy.
    pub fn decode(bytes: &[u8], limits: &DecodeLimits) -> Result<Self, LimitsError> {
        validate_drisl(bytes, limits)?;
        Self::from_view(ValueView::preflighted(bytes))
    }

    pub(crate) fn wire_fields(&self) -> [(&'static str, u64); 18] {
        [
            ("max_memory_bytes", self.max_memory_bytes),
            ("max_observation_bytes", self.max_observation_bytes),
            ("max_receipt_bytes", self.max_receipt_bytes),
            ("max_command_bytes", self.max_command_bytes),
            ("max_block_bytes", self.max_block_bytes),
            ("max_materialized_bytes", self.max_materialized_bytes),
            ("max_blocks", self.max_blocks),
            ("max_depth", self.max_depth),
            ("max_collection_entries", self.max_collection_entries),
            ("max_context_bytes", self.max_context_bytes),
            ("max_model_reply_bytes", self.max_model_reply_bytes),
            ("max_patch_entries", self.max_patch_entries),
            ("history_episodes", self.history_episodes),
            ("max_actions_per_episode", self.max_actions_per_episode),
            (
                "max_model_calls_per_activation",
                self.max_model_calls_per_activation,
            ),
            (
                "max_model_calls_per_episode",
                self.max_model_calls_per_episode,
            ),
            (
                "max_local_tools_per_activation",
                self.max_local_tools_per_activation,
            ),
            ("max_corrective_turns", self.max_corrective_turns),
        ]
    }

    /// Caller must have preflighted the enclosing record before inspecting this view.
    pub(crate) fn from_view(value: ValueView<'_>) -> Result<Self, LimitsError> {
        let fields = value.map().map_err(|_| LimitsError::InvalidSchema)?;
        if fields.len() != 18 {
            return Err(LimitsError::InvalidSchema);
        }
        // Preflight guarantees unique keys. Exact count plus the closed whitelist
        // proves every field is present; these initial values can never fill a gap.
        let mut result = Self::default();
        for field in fields {
            let (key, value) = field?;
            let value = value.unsigned().map_err(|_| LimitsError::InvalidSchema)?;
            match key {
                "max_memory_bytes" => result.max_memory_bytes = value,
                "max_observation_bytes" => result.max_observation_bytes = value,
                "max_receipt_bytes" => result.max_receipt_bytes = value,
                "max_command_bytes" => result.max_command_bytes = value,
                "max_block_bytes" => result.max_block_bytes = value,
                "max_materialized_bytes" => result.max_materialized_bytes = value,
                "max_blocks" => result.max_blocks = value,
                "max_depth" => result.max_depth = value,
                "max_collection_entries" => result.max_collection_entries = value,
                "max_context_bytes" => result.max_context_bytes = value,
                "max_model_reply_bytes" => result.max_model_reply_bytes = value,
                "max_patch_entries" => result.max_patch_entries = value,
                "history_episodes" => result.history_episodes = value,
                "max_actions_per_episode" => result.max_actions_per_episode = value,
                "max_model_calls_per_activation" => result.max_model_calls_per_activation = value,
                "max_model_calls_per_episode" => result.max_model_calls_per_episode = value,
                "max_local_tools_per_activation" => result.max_local_tools_per_activation = value,
                "max_corrective_turns" => result.max_corrective_turns = value,
                _ => return Err(LimitsError::InvalidSchema),
            }
        }
        Ok(result)
    }

    /// Non-lossy projection of wire bounds; does not enforce other policy budgets.
    pub fn decode_limits(&self) -> DecodeLimits {
        DecodeLimits {
            max_block_bytes: self.max_block_bytes,
            max_depth: self.max_depth,
            max_collection_entries: self.max_collection_entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../fixtures/format/limits.json")).unwrap()
    }

    fn bytes(value: &Value) -> Vec<u8> {
        crate::format::oracle::json_bytes(value)
    }

    #[test]
    fn complete_limits_golden_bytes_agree_with_an_independent_codec_and_hash() {
        let fixtures = fixtures();
        let bounds = DecodeLimits::default();
        for fixture in fixtures.as_array().unwrap() {
            let expected = hex(fixture["hex"].as_str().unwrap());
            assert_eq!(bytes(&fixture["record"]), expected);
            let digest = Sha256::digest(&expected);
            assert_eq!(&digest[..], &hex(fixture["sha256"].as_str().unwrap())[..]);
            let decoded = Limits::decode(&expected, &bounds).unwrap();
            for (key, value) in decoded.wire_fields() {
                assert_eq!(fixture["record"][key].as_u64(), Some(value));
            }
            assert_eq!(decoded.encode(&bounds).unwrap(), expected);
        }
        let demo = hex(fixtures[0]["hex"].as_str().unwrap());
        assert_eq!(Limits::decode(&demo, &bounds), Ok(Limits::default()));
        let extremes = hex(fixtures[1]["hex"].as_str().unwrap());
        assert_eq!(
            crate::format::validate_model_value(&extremes, &bounds),
            Err(CodecError::ModelIntegerRange)
        );
        assert!(Limits::decode(&extremes, &bounds)
            .unwrap()
            .wire_fields()
            .iter()
            .all(|(_, value)| *value == u64::MAX));
    }

    #[test]
    fn every_policy_field_is_required_and_unknown_replacements_reject() {
        let fixture = fixtures();
        let record = fixture[0]["record"].as_object().unwrap();
        for key in record.keys() {
            let mut changed = record.clone();
            changed.remove(key);
            assert_eq!(
                Limits::decode(
                    &bytes(&Value::Object(changed.clone())),
                    &DecodeLimits::default()
                ),
                Err(LimitsError::InvalidSchema),
                "missing {key}"
            );
            changed.insert("unknown".into(), json!(0));
            assert_eq!(
                Limits::decode(&bytes(&Value::Object(changed)), &DecodeLimits::default()),
                Err(LimitsError::InvalidSchema),
                "replaced {key}"
            );
        }
        for value in [json!(null), json!([]), json!(1)] {
            assert_eq!(
                Limits::decode(&bytes(&value), &DecodeLimits::default()),
                Err(LimitsError::InvalidSchema)
            );
        }
    }

    #[test]
    fn each_policy_field_requires_unsigned_integer_type_without_coercion() {
        let fixture = fixtures();
        let record = fixture[0]["record"].as_object().unwrap();
        for key in record.keys() {
            for invalid in [
                json!(null),
                json!(true),
                json!("1"),
                json!([]),
                json!({}),
                json!(-1),
            ] {
                let mut changed = record.clone();
                changed.insert(key.clone(), invalid);
                assert_eq!(
                    Limits::decode(&bytes(&Value::Object(changed)), &DecodeLimits::default()),
                    Err(LimitsError::InvalidSchema),
                    "type of {key}"
                );
            }
        }
        let mut changed = record.clone();
        changed.insert("max_depth".into(), json!(1.5));
        assert_eq!(
            Limits::decode(&bytes(&Value::Object(changed)), &DecodeLimits::default()),
            Err(LimitsError::Codec(CodecError::UnsupportedValue))
        );
    }

    #[test]
    fn distinct_values_are_not_swapped_between_policy_fields() {
        let fixture = fixtures();
        let record = fixture[0]["record"].as_object().unwrap();
        let mut changed = record.clone();
        for (index, key) in record.keys().enumerate() {
            changed.insert(key.clone(), json!(100 + index as u64));
        }
        let expected = bytes(&Value::Object(changed));
        let decoded = Limits::decode(&expected, &DecodeLimits::default()).unwrap();
        assert_eq!(decoded.encode(&DecodeLimits::default()).unwrap(), expected);
    }

    #[test]
    fn runtime_record_byte_depth_and_count_limits_match_at_boundaries() {
        let policy = Limits::default();
        let bytes = policy.encode(&DecodeLimits::default()).unwrap();
        let exact = DecodeLimits {
            max_block_bytes: bytes.len() as u64,
            max_depth: 2,
            max_collection_entries: 18,
        };
        assert_eq!(policy.encode(&exact), Ok(bytes.clone()));
        assert_eq!(Limits::decode(&bytes, &exact), Ok(policy));
        let short = DecodeLimits {
            max_block_bytes: exact.max_block_bytes - 1,
            ..exact
        };
        assert_eq!(
            policy.encode(&short),
            Err(LimitsError::Codec(CodecError::ByteLimit))
        );
        assert_eq!(
            Limits::decode(&bytes, &short),
            Err(LimitsError::Codec(CodecError::ByteLimit))
        );
        let shallow = DecodeLimits {
            max_depth: 1,
            ..exact
        };
        assert_eq!(
            policy.encode(&shallow),
            Err(LimitsError::Codec(CodecError::DepthLimit))
        );
        assert_eq!(
            Limits::decode(&bytes, &shallow),
            Err(LimitsError::Codec(CodecError::DepthLimit))
        );
        let narrow = DecodeLimits {
            max_collection_entries: 17,
            ..exact
        };
        assert_eq!(
            policy.encode(&narrow),
            Err(LimitsError::Codec(CodecError::CollectionLimit))
        );
        assert_eq!(
            Limits::decode(&bytes, &narrow),
            Err(LimitsError::Codec(CodecError::CollectionLimit))
        );
    }

    #[test]
    fn noncanonical_duplicate_truncated_and_trailing_records_never_decode() {
        let record = Limits::default().encode(&DecodeLimits::default()).unwrap();
        for length in 0..record.len() {
            assert!(
                Limits::decode(&record[..length], &DecodeLimits::default()).is_err(),
                "prefix {length}"
            );
        }
        let mut nonminimal = alloc::vec![0xb8, 18];
        nonminimal.extend_from_slice(&record[1..]);
        assert_eq!(
            Limits::decode(&nonminimal, &DecodeLimits::default()),
            Err(LimitsError::Codec(CodecError::NonCanonical))
        );
        let mut trailing = record;
        trailing.push(0);
        assert_eq!(
            Limits::decode(&trailing, &DecodeLimits::default()),
            Err(LimitsError::Codec(CodecError::TrailingBytes))
        );
        let duplicate = hex("a2696d61785f646570746800696d61785f646570746801");
        assert_eq!(
            Limits::decode(&duplicate, &DecodeLimits::default()),
            Err(LimitsError::Codec(CodecError::DuplicateKey))
        );
    }

    #[test]
    fn untrusted_policy_fields_never_replace_the_callers_bootstrap_bounds() {
        let fixture = fixtures();
        let large = hex(fixture[1]["hex"].as_str().unwrap());
        let bootstrap = DecodeLimits {
            max_block_bytes: 1,
            ..DecodeLimits::default()
        };
        assert_eq!(
            Limits::decode(&large, &bootstrap),
            Err(LimitsError::Codec(CodecError::ByteLimit))
        );
        let mut record = fixture[0]["record"].clone();
        record["max_depth"] = json!(0);
        let decoded = Limits::decode(&bytes(&record), &DecodeLimits::default()).unwrap();
        assert_eq!(decoded.max_depth, 0);
    }

    #[test]
    fn all_eighteen_demo_defaults_match_the_normative_profile() {
        let limits = Limits::default();
        assert_eq!(
            [
                limits.max_memory_bytes,
                limits.max_observation_bytes,
                limits.max_receipt_bytes,
                limits.max_command_bytes,
                limits.max_block_bytes,
                limits.max_materialized_bytes,
                limits.max_blocks,
                limits.max_depth,
                limits.max_collection_entries,
                limits.max_context_bytes,
                limits.max_model_reply_bytes,
                limits.max_patch_entries,
                limits.history_episodes,
                limits.max_actions_per_episode,
                limits.max_model_calls_per_activation,
                limits.max_model_calls_per_episode,
                limits.max_local_tools_per_activation,
                limits.max_corrective_turns,
            ],
            [
                4096, 8192, 4096, 2048, 65_536, 4_194_304, 4096, 32, 4096, 131_072, 32_768, 32, 2,
                8, 16, 64, 16, 2
            ]
        );
        assert_eq!(limits.decode_limits(), DecodeLimits::default());
    }

    #[test]
    fn wire_projection_preserves_u64_extremes_without_native_width_conversion() {
        for value in [0, u64::MAX] {
            let limits = Limits {
                max_block_bytes: value,
                max_depth: value,
                max_collection_entries: value,
                max_model_calls_per_episode: value,
                ..Limits::default()
            };
            assert_eq!(
                limits.decode_limits(),
                DecodeLimits {
                    max_block_bytes: value,
                    max_depth: value,
                    max_collection_entries: value,
                }
            );
            assert_eq!(limits.max_model_calls_per_episode, value);
            assert_eq!(limits.max_memory_bytes, Limits::default().max_memory_bytes);
        }
    }
}
