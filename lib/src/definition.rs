//! Owns Definition and descriptor schema validation, not planner execution or host authority.

use crate::format::{
    validate_drisl, validate_model_value, CodecError, DaslCid, DaslCodec, DecodeLimits,
    RecordEncoder, ValueView,
};
use crate::limits::{Limits, LimitsError};
use alloc::vec::Vec;
use core::{fmt, ops::Range};

/// Disabled memory or the first sparse intents/strategy/tactics standard.
/// Incompatible profile strings are rejected; no read-time aliases or migration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryProfile {
    None,
    IntentsStrategyTactics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolControl {
    Continue,
    Yield,
    Finish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionError {
    Codec(CodecError),
    Limits(LimitsError),
    WrongKind,
    UnsupportedVersion,
    InvalidSchema,
    DuplicateToolId,
    InvalidAuthorityBinding,
}

impl From<CodecError> for DefinitionError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<LimitsError> for DefinitionError {
    fn from(error: LimitsError) -> Self {
        Self::Limits(error)
    }
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::Limits(error) => error.fmt(f),
            Self::WrongKind => f.write_str("block is not a Definition"),
            Self::UnsupportedVersion => {
                f.write_str("unsupported Definition, memory, projection or normalizer version")
            }
            Self::InvalidSchema => f.write_str("invalid Definition or tool descriptor schema"),
            Self::DuplicateToolId => f.write_str("tool IDs must be unique"),
            Self::InvalidAuthorityBinding => {
                f.write_str("authority must be Id32; only local tools may use null")
            }
        }
    }
}

impl core::error::Error for DefinitionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Limits(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ToolInfo {
    id: Range<usize>,
    version: u64,
    control: ToolControl,
    description: Range<usize>,
    arguments_schema: Range<usize>,
    authority_binding: Option<[u8; 32]>,
}

/// Borrowed descriptor data in the exact order pinned by its Definition.
/// `arguments_schema` is one canonical model Map, not executable validation code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolDescriptorView<'a> {
    pub id: &'a str,
    pub version: u64,
    pub control: ToolControl,
    pub description: &'a str,
    pub arguments_schema: &'a [u8],
    pub authority_binding: Option<[u8; 32]>,
}

/// Borrowed input to the version-1 Definition encoder. Encoded profile/schema fields
/// must be canonical model Maps; they are validated before being copied into the record.
/// Tools retain this exact order. No input field can override the supported wire versions.
#[derive(Clone, Copy, Debug)]
pub struct DefinitionSpec<'a> {
    pub execution_domain: [u8; 32],
    pub policy: &'a str,
    pub mandate: &'a str,
    pub memory_profile: MemoryProfile,
    pub tools: &'a [ToolDescriptorView<'a>],
    pub inference_profile: &'a [u8],
    pub limits: Limits,
}

/// An owned, schema-checked Definition with bounded borrowed-field indexes.
///
/// Decoding proves canonical bytes and data shape only, not an authorized program,
/// matching planner semantics, absence of semantic secrets, or acceptable deployment
/// policy. The host must select an approved Definition and bind its digest to authority.
/// Embedded Limits never replace the caller's bootstrap bounds during decoding.
#[derive(Debug, Eq, PartialEq)]
pub struct Definition {
    block: Vec<u8>,
    execution_domain: [u8; 32],
    policy: Range<usize>,
    mandate: Range<usize>,
    memory_profile: MemoryProfile,
    inference_profile: Range<usize>,
    tools: Vec<ToolInfo>,
    limits: Limits,
}

impl Definition {
    /// Encode the exact supported schema without re-encoding embedded canonical maps.
    /// The complete output is inspected before returning a valid Definition.
    ///
    /// ```
    /// use raiv::definition::{Definition, DefinitionSpec, MemoryProfile};
    /// use raiv::format::DecodeLimits;
    /// use raiv::limits::Limits;
    /// let spec = DefinitionSpec {
    ///     execution_domain: [1; 32], policy: "fixture policy", mandate: "fixture mandate",
    ///     memory_profile: MemoryProfile::None, tools: &[], inference_profile: &[0xa0],
    ///     limits: Limits::default(),
    /// };
    /// let definition = Definition::encode(&spec, &DecodeLimits::default())?;
    /// assert_eq!(definition.execution_domain(), [1; 32]);
    /// // A valid record is not a runnable agent or authorized host policy.
    /// # Ok::<(), raiv::definition::DefinitionError>(())
    /// ```
    pub fn encode(
        spec: &DefinitionSpec<'_>,
        bounds: &DecodeLimits,
    ) -> Result<Self, DefinitionError> {
        if bounds.max_depth < 3 || (!spec.tools.is_empty() && bounds.max_depth < 4) {
            return Err(CodecError::DepthLimit.into());
        }
        if bounds.max_collection_entries < 18
            || spec.tools.len() as u64 > bounds.max_collection_entries
        {
            return Err(CodecError::CollectionLimit.into());
        }
        check_model_map(spec.inference_profile, bounds, 2)?;
        for (index, tool) in spec.tools.iter().enumerate() {
            check_model_map(tool.arguments_schema, bounds, 4)?;
            if tool.control == ToolControl::Yield && tool.authority_binding.is_none() {
                return Err(DefinitionError::InvalidAuthorityBinding);
            }
            if spec.tools[..index]
                .iter()
                .any(|previous| previous.id == tool.id)
            {
                return Err(DefinitionError::DuplicateToolId);
            }
        }
        let nested_bounds = DecodeLimits {
            max_depth: bounds.max_depth - 1,
            ..*bounds
        };
        let policy_limits = spec.limits.encode(&nested_bounds)?;
        let mut encoder = RecordEncoder::new(bounds)?;
        // Structural keys use encoded-key order, not Rust/string insertion order.
        encoder.map(11)?;
        encoder.text("kind")?;
        encoder.text("definition")?;
        encoder.text("tools")?;
        encoder.list(spec.tools.len() as u64)?;
        for tool in spec.tools {
            encoder.map(6)?;
            encoder.text("id")?;
            encoder.text(tool.id)?;
            encoder.text("control")?;
            encoder.text(match tool.control {
                ToolControl::Continue => "continue",
                ToolControl::Yield => "yield",
                ToolControl::Finish => "finish",
            })?;
            encoder.text("version")?;
            encoder.unsigned(tool.version)?;
            encoder.text("description")?;
            encoder.text(tool.description)?;
            encoder.text("arguments_schema")?;
            encoder.encoded(tool.arguments_schema)?;
            encoder.text("authority_binding")?;
            if let Some(authority) = tool.authority_binding {
                encoder.bytes(&authority)?;
            } else {
                encoder.null()?;
            }
        }
        encoder.text("limits")?;
        encoder.encoded(&policy_limits)?;
        encoder.text("policy")?;
        encoder.text(spec.policy)?;
        encoder.text("mandate")?;
        encoder.text(spec.mandate)?;
        encoder.text("version")?;
        encoder.unsigned(1)?;
        encoder.text("memory_profile")?;
        encoder.text(match spec.memory_profile {
            MemoryProfile::None => "none",
            MemoryProfile::IntentsStrategyTactics => "intents-strategy-tactics/1",
        })?;
        encoder.text("execution_domain")?;
        encoder.bytes(&spec.execution_domain)?;
        encoder.text("inference_profile")?;
        encoder.encoded(spec.inference_profile)?;
        encoder.text("normalizer_version")?;
        encoder.unsigned(1)?;
        encoder.text("projection_version")?;
        encoder.unsigned(1)?;
        let block = encoder.finish();
        let mut definition = Self::inspect(&block, bounds)?;
        definition.block = block;
        Ok(definition)
    }

    pub fn decode(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, DefinitionError> {
        let mut definition = Self::inspect(bytes, bounds)?;
        definition.block = crate::format::copy_validated(bytes)?;
        Ok(definition)
    }

    fn inspect(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, DefinitionError> {
        validate_drisl(bytes, bounds)?;
        let fields = exact_fields(
            ValueView::preflighted(bytes),
            [
                "kind",
                "version",
                "execution_domain",
                "policy",
                "mandate",
                "memory_profile",
                "tools",
                "inference_profile",
                "projection_version",
                "normalizer_version",
                "limits",
            ],
        )?;
        if text(fields[0])? != "definition" {
            return Err(DefinitionError::WrongKind);
        }
        for index in [1, 8, 9] {
            if unsigned(fields[index])? != 1 {
                return Err(DefinitionError::UnsupportedVersion);
            }
        }
        let execution_domain = fields[2]
            .bytes()
            .map_err(|_| DefinitionError::InvalidSchema)?
            .try_into()
            .map_err(|_| DefinitionError::InvalidSchema)?;
        let policy = text_span(fields[3])?;
        let mandate = text_span(fields[4])?;
        let memory_profile = match text(fields[5])? {
            "none" => MemoryProfile::None,
            "intents-strategy-tactics/1" => MemoryProfile::IntentsStrategyTactics,
            _ => return Err(DefinitionError::UnsupportedVersion),
        };
        let inference_profile = model_map(fields[7], bounds)?;
        let limits = Limits::from_view(fields[10])?;
        let list = fields[6]
            .list()
            .map_err(|_| DefinitionError::InvalidSchema)?;
        let count = usize::try_from(list.len()).map_err(|_| CodecError::CollectionLimit)?;
        let mut tools = Vec::new();
        let mut names = Vec::new();
        tools
            .try_reserve_exact(count)
            .map_err(|_| CodecError::AllocationFailure)?;
        names
            .try_reserve_exact(count)
            .map_err(|_| CodecError::AllocationFailure)?;
        for value in list {
            let fields = exact_fields(
                value?,
                [
                    "id",
                    "version",
                    "control",
                    "description",
                    "arguments_schema",
                    "authority_binding",
                ],
            )?;
            let name = text(fields[0])?;
            let id = text_span(fields[0])?;
            let version = unsigned(fields[1])?;
            let control = match text(fields[2])? {
                "continue" => ToolControl::Continue,
                "yield" => ToolControl::Yield,
                "finish" => ToolControl::Finish,
                _ => return Err(DefinitionError::InvalidSchema),
            };
            let description = text_span(fields[3])?;
            let arguments_schema = model_map(fields[4], bounds)?;
            let authority_binding = if fields[5].is_null() {
                if control == ToolControl::Yield {
                    return Err(DefinitionError::InvalidAuthorityBinding);
                }
                None
            } else {
                Some(
                    fields[5]
                        .bytes()
                        .map_err(|_| DefinitionError::InvalidAuthorityBinding)?
                        .try_into()
                        .map_err(|_| DefinitionError::InvalidAuthorityBinding)?,
                )
            };
            names.push(name);
            tools.push(ToolInfo {
                id,
                version,
                control,
                description,
                arguments_schema,
                authority_binding,
            });
        }
        // Sort only the borrowed names for uniqueness; descriptor order is unchanged.
        names.sort_unstable();
        if names.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(DefinitionError::DuplicateToolId);
        }
        Ok(Self {
            block: Vec::new(),
            execution_domain,
            policy,
            mandate,
            memory_profile,
            inference_profile,
            tools,
            limits,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.block
    }

    pub fn cid(&self) -> DaslCid {
        DaslCid::for_block(DaslCodec::Drisl, self.as_bytes())
    }

    pub fn execution_domain(&self) -> [u8; 32] {
        self.execution_domain
    }

    pub fn policy(&self) -> &str {
        self.text(&self.policy)
    }

    pub fn mandate(&self) -> &str {
        self.text(&self.mandate)
    }

    pub fn memory_profile(&self) -> MemoryProfile {
        self.memory_profile
    }

    pub fn inference_profile_bytes(&self) -> &[u8] {
        &self.block[self.inference_profile.clone()]
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn tools(&self) -> impl ExactSizeIterator<Item = ToolDescriptorView<'_>> {
        self.tools.iter().map(|tool| ToolDescriptorView {
            id: self.text(&tool.id),
            version: tool.version,
            control: tool.control,
            description: self.text(&tool.description),
            arguments_schema: &self.block[tool.arguments_schema.clone()],
            authority_binding: tool.authority_binding,
        })
    }

    fn text(&self, span: &Range<usize>) -> &str {
        core::str::from_utf8(&self.block[span.clone()]).expect("schema-validated immutable UTF-8")
    }
}

fn exact_fields<'a, const N: usize>(
    value: ValueView<'a>,
    names: [&str; N],
) -> Result<[ValueView<'a>; N], DefinitionError> {
    value
        .fields(names)
        .map_err(|_| DefinitionError::InvalidSchema)
}

fn text(value: ValueView<'_>) -> Result<&str, DefinitionError> {
    value.text().map_err(|_| DefinitionError::InvalidSchema)
}

fn text_span(value: ValueView<'_>) -> Result<Range<usize>, DefinitionError> {
    value
        .text_span()
        .map_err(|_| DefinitionError::InvalidSchema)
}

fn unsigned(value: ValueView<'_>) -> Result<u64, DefinitionError> {
    value.unsigned().map_err(|_| DefinitionError::InvalidSchema)
}

fn check_model_map(bytes: &[u8], bounds: &DecodeLimits, depth: u64) -> Result<(), DefinitionError> {
    let max_depth = bounds
        .max_depth
        .checked_sub(depth - 1)
        .ok_or(CodecError::DepthLimit)?;
    let nested = DecodeLimits {
        max_depth,
        ..*bounds
    };
    validate_model_value(bytes, &nested)?;
    ValueView::preflighted(bytes)
        .map()
        .map_err(|_| DefinitionError::InvalidSchema)?;
    Ok(())
}

fn model_map(value: ValueView<'_>, bounds: &DecodeLimits) -> Result<Range<usize>, DefinitionError> {
    value.map().map_err(|_| DefinitionError::InvalidSchema)?;
    validate_model_value(value.encoded(), bounds)?;
    Ok(value.span())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::oracle::{encode as bytes, fixture_value as node, map_mut as map};
    use alloc::{string::ToString, vec};
    use ciborium::value::Value as Cbor;
    use serde_json::Value;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../fixtures/format/definition.json")).unwrap()
    }

    fn record() -> Cbor {
        node(&fixtures()[0]["record"])
    }

    fn tool_list(value: &mut Cbor) -> &mut Vec<Cbor> {
        map(value)
            .get_mut("tools")
            .unwrap()
            .as_array_mut()
            .expect("fixture tools are not a list")
    }

    fn encoder_spec<'a>(
        definition: &'a Definition,
        tools: &'a [ToolDescriptorView<'a>],
    ) -> DefinitionSpec<'a> {
        DefinitionSpec {
            execution_domain: definition.execution_domain(),
            policy: definition.policy(),
            mandate: definition.mandate(),
            memory_profile: definition.memory_profile(),
            tools,
            inference_profile: definition.inference_profile_bytes(),
            limits: *definition.limits(),
        }
    }

    #[test]
    fn complete_definition_golden_records_match_independent_bytes_and_cids() {
        let fixtures = fixtures();
        for fixture in fixtures.as_array().unwrap() {
            let expected = hex(fixture["hex"].as_str().unwrap());
            assert_eq!(bytes(&node(&fixture["record"])), expected);
            let definition = Definition::decode(&expected, &DecodeLimits::default()).unwrap();
            assert_eq!(definition.as_bytes(), expected);
            assert_eq!(definition.cid().to_string(), fixture["cid"]);
            assert_eq!(definition.execution_domain(), [1; 32]);
            assert_eq!(
                definition.policy(),
                fixture["record"]["policy"].as_str().unwrap()
            );
            assert_eq!(
                definition.mandate(),
                fixture["record"]["mandate"].as_str().unwrap()
            );
            let profile = if fixture["record"]["memory_profile"] == "none" {
                MemoryProfile::None
            } else {
                MemoryProfile::IntentsStrategyTactics
            };
            assert_eq!(definition.memory_profile(), profile);
            assert_eq!(*definition.limits(), Limits::default());
            assert_eq!(
                definition.inference_profile_bytes(),
                bytes(&node(&fixture["record"]["inference_profile"]))
            );
            let expected_tools = fixture["record"]["tools"].as_array().unwrap();
            assert_eq!(definition.tools().len(), expected_tools.len());
            for (tool, expected) in definition.tools().zip(expected_tools) {
                assert_eq!(tool.id, expected["id"].as_str().unwrap());
                assert_eq!(tool.description, expected["description"].as_str().unwrap());
                assert_eq!(tool.version, 1);
                assert_eq!(
                    tool.arguments_schema,
                    bytes(&node(&expected["arguments_schema"]))
                );
                let control = match expected["control"].as_str().unwrap() {
                    "continue" => ToolControl::Continue,
                    "yield" => ToolControl::Yield,
                    _ => ToolControl::Finish,
                };
                assert_eq!(tool.control, control);
                let authority = if expected["authority_binding"].is_null() {
                    None
                } else {
                    Some(
                        hex(expected["authority_binding"]["$bytes"].as_str().unwrap())
                            .try_into()
                            .unwrap(),
                    )
                };
                assert_eq!(tool.authority_binding, authority);
            }
            let tools = definition.tools().collect::<Vec<_>>();
            let encoded =
                Definition::encode(&encoder_spec(&definition, &tools), &DecodeLimits::default())
                    .unwrap();
            assert_eq!(encoded, definition);
            assert_eq!(encoded.cid().to_string(), fixture["cid"]);
        }
    }

    #[test]
    fn encoded_definition_owns_output_independently_of_all_borrowed_inputs() {
        let encoded = {
            let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
            let tools = decoded.tools().collect::<Vec<_>>();
            Definition::encode(&encoder_spec(&decoded, &tools), &DecodeLimits::default()).unwrap()
        };
        assert_eq!(encoded.as_bytes(), bytes(&record()));
        assert_eq!(
            encoded.tools().map(|tool| tool.id).collect::<Vec<_>>(),
            ["veto", "pass"]
        );
    }

    #[test]
    fn encoding_preserves_tool_order_and_full_width_policy_and_tool_versions() {
        let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
        let mut tools = decoded.tools().collect::<Vec<_>>();
        tools.reverse();
        tools[0].version = u64::MAX;
        let mut spec = encoder_spec(&decoded, &tools);
        spec.limits.max_model_calls_per_episode = u64::MAX;
        spec.limits.max_memory_bytes = u64::MAX;
        let encoded = Definition::encode(&spec, &DecodeLimits::default()).unwrap();
        let mut oracle = record();
        tool_list(&mut oracle).reverse();
        map(&mut tool_list(&mut oracle)[0])
            .insert("version".into(), Cbor::Integer(u64::MAX.into()));
        let limits = map(&mut oracle).get_mut("limits").unwrap();
        map(limits).insert("max_memory_bytes".into(), Cbor::Integer(u64::MAX.into()));
        map(limits).insert(
            "max_model_calls_per_episode".into(),
            Cbor::Integer(u64::MAX.into()),
        );
        assert_eq!(encoded.as_bytes(), bytes(&oracle));
        assert_eq!(
            encoded.tools().map(|tool| tool.id).collect::<Vec<_>>(),
            ["pass", "veto"]
        );
        assert_eq!(encoded.limits().max_memory_bytes, u64::MAX);
        assert_eq!(encoded.tools().next().unwrap().version, u64::MAX);
    }

    #[test]
    fn borrowed_map_input_must_be_one_canonical_model_map() {
        let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
        let tools = decoded.tools().collect::<Vec<_>>();
        let invalid = [
            "f6",
            "80",
            "40",
            "b801616100",
            "a0ff",
            "a2616100616101",
            "a161611b0020000000000000",
            "a16161fb0000000000000000",
        ];
        for data in invalid {
            let raw = hex(data);
            let mut spec = encoder_spec(&decoded, &tools);
            spec.inference_profile = &raw;
            assert!(
                Definition::encode(&spec, &DecodeLimits::default()).is_err(),
                "profile {data}"
            );
            let mut invalid_tools = tools.clone();
            invalid_tools[0].arguments_schema = &raw;
            let spec = encoder_spec(&decoded, &invalid_tools);
            assert!(
                Definition::encode(&spec, &DecodeLimits::default()).is_err(),
                "schema {data}"
            );
        }
    }

    #[test]
    fn encoder_checks_combined_nesting_of_embedded_model_maps_before_copying() {
        let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
        let tools = decoded.tools().collect::<Vec<_>>();
        let spec = encoder_spec(&decoded, &tools);
        let exact = DecodeLimits {
            max_depth: 5,
            ..DecodeLimits::default()
        };
        assert_eq!(Definition::encode(&spec, &exact).unwrap(), decoded);
        let shallow = DecodeLimits {
            max_depth: 4,
            ..exact
        };
        assert_eq!(
            Definition::encode(&spec, &shallow),
            Err(DefinitionError::Codec(CodecError::DepthLimit))
        );
        let nested = hex("a1616181f6");
        let mut changed = tools.clone();
        changed[0].arguments_schema = &nested;
        assert_eq!(
            Definition::encode(&encoder_spec(&decoded, &changed), &exact),
            Err(DefinitionError::Codec(CodecError::DepthLimit))
        );
        let empty = [0xa0];
        for tool in &mut changed {
            tool.arguments_schema = &empty;
        }
        assert!(Definition::encode(&encoder_spec(&decoded, &changed), &shallow).is_ok());
    }

    #[test]
    fn encoder_byte_count_and_depth_bounds_do_not_come_from_embedded_policy() {
        let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
        let tools = decoded.tools().collect::<Vec<_>>();
        let mut spec = encoder_spec(&decoded, &tools);
        spec.limits.max_block_bytes = u64::MAX;
        spec.limits.max_depth = u64::MAX;
        let encoded = Definition::encode(&spec, &DecodeLimits::default()).unwrap();
        let exact = DecodeLimits {
            max_block_bytes: encoded.as_bytes().len() as u64,
            ..DecodeLimits::default()
        };
        assert_eq!(Definition::encode(&spec, &exact).unwrap(), encoded);
        let short = DecodeLimits {
            max_block_bytes: exact.max_block_bytes - 1,
            ..exact
        };
        assert_eq!(
            Definition::encode(&spec, &short),
            Err(DefinitionError::Codec(CodecError::ByteLimit))
        );
        let narrow = DecodeLimits {
            max_collection_entries: 17,
            ..exact
        };
        assert_eq!(
            Definition::encode(&spec, &narrow),
            Err(DefinitionError::Codec(CodecError::CollectionLimit))
        );
        let shallow = DecodeLimits {
            max_depth: 0,
            ..exact
        };
        assert_eq!(
            Definition::encode(&spec, &shallow),
            Err(DefinitionError::Codec(CodecError::DepthLimit))
        );
    }

    #[test]
    fn encoder_rejects_duplicate_ids_and_unbound_yields_without_granting_authority() {
        let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
        let mut tools = decoded.tools().collect::<Vec<_>>();
        tools[1].id = tools[0].id;
        assert_eq!(
            Definition::encode(&encoder_spec(&decoded, &tools), &DecodeLimits::default()),
            Err(DefinitionError::DuplicateToolId)
        );
        tools[1].id = "pass";
        tools[0].authority_binding = None;
        assert_eq!(
            Definition::encode(&encoder_spec(&decoded, &tools), &DecodeLimits::default()),
            Err(DefinitionError::InvalidAuthorityBinding)
        );
        tools[0].control = ToolControl::Continue;
        assert!(
            Definition::encode(&encoder_spec(&decoded, &tools), &DecodeLimits::default()).is_ok()
        );
        tools[0].authority_binding = Some([0; 32]);
        assert!(
            Definition::encode(&encoder_spec(&decoded, &tools), &DecodeLimits::default()).is_ok()
        );
    }

    #[test]
    fn minimal_definition_still_accounts_for_its_nonempty_limits_envelope() {
        let decoded = Definition::decode(&bytes(&record()), &DecodeLimits::default()).unwrap();
        let mut spec = encoder_spec(&decoded, &[]);
        spec.inference_profile = &[0xa0];
        let exact = DecodeLimits {
            max_depth: 3,
            ..DecodeLimits::default()
        };
        let encoded = Definition::encode(&spec, &exact).unwrap();
        assert_eq!(encoded.tools().len(), 0);
        let shallow = DecodeLimits {
            max_depth: 2,
            ..exact
        };
        assert_eq!(
            Definition::encode(&spec, &shallow),
            Err(DefinitionError::Codec(CodecError::DepthLimit))
        );
    }

    #[test]
    fn every_definition_field_is_required_and_the_root_whitelist_is_closed() {
        let mut source = record();
        let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut changed = source.clone();
            map(&mut changed).remove(&key);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema),
                "missing {key}"
            );
            map(&mut changed).insert("unknown".into(), Cbor::Null);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema),
                "replaced {key}"
            );
        }
        for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(1.into())] {
            assert_eq!(
                Definition::decode(&bytes(&value), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema)
            );
        }
    }

    #[test]
    fn definition_versions_profiles_and_scalar_types_are_not_coerced() {
        for field in ["version", "projection_version", "normalizer_version"] {
            for version in [0, 2, u64::MAX] {
                let mut changed = record();
                map(&mut changed).insert(field.into(), Cbor::Integer(version.into()));
                assert_eq!(
                    Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                    Err(DefinitionError::UnsupportedVersion)
                );
            }
            let mut changed = record();
            map(&mut changed).insert(field.into(), Cbor::Text("1".into()));
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema)
            );
        }
        let mut changed = record();
        map(&mut changed).insert("kind".into(), Cbor::Text("memory".into()));
        assert_eq!(
            Definition::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(DefinitionError::WrongKind)
        );
        map(&mut changed).insert("kind".into(), Cbor::Text("definition".into()));
        for incompatible in [
            "strategy-intent/1",
            "strategy-intent/2",
            "intents-strategy-tactics/2",
            "intent",
        ] {
            map(&mut changed).insert("memory_profile".into(), Cbor::Text(incompatible.into()));
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::UnsupportedVersion)
            );
        }
        for field in [
            "kind",
            "policy",
            "mandate",
            "memory_profile",
            "tools",
            "inference_profile",
        ] {
            let mut changed = record();
            map(&mut changed).insert(field.into(), Cbor::Null);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema)
            );
        }
    }

    #[test]
    fn execution_domain_requires_exactly_32_raw_bytes_not_a_text_id() {
        for value in [
            Cbor::Bytes(vec![0; 31]),
            Cbor::Bytes(vec![0; 33]),
            Cbor::Text("domain".into()),
            Cbor::Array(vec![]),
        ] {
            let mut changed = record();
            map(&mut changed).insert("execution_domain".into(), value);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema)
            );
        }
    }

    #[test]
    fn descriptor_field_membership_and_types_are_closed() {
        let keys = [
            "id",
            "version",
            "control",
            "description",
            "arguments_schema",
            "authority_binding",
        ];
        for key in keys {
            let mut changed = record();
            map(&mut tool_list(&mut changed)[0]).remove(key);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema),
                "missing tool {key}"
            );
            map(&mut tool_list(&mut changed)[0]).insert("unknown".into(), Cbor::Null);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema),
                "replaced tool {key}"
            );
        }
        for (key, value) in [
            ("id", Cbor::Null),
            ("version", Cbor::Integer((-1).into())),
            ("description", Cbor::Integer(1.into())),
            ("control", Cbor::Text("parallel".into())),
            ("arguments_schema", Cbor::Array(vec![])),
        ] {
            let mut changed = record();
            map(&mut tool_list(&mut changed)[0]).insert(key.into(), value);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidSchema)
            );
        }
    }

    #[test]
    fn duplicate_tool_ids_reject_without_reordering_valid_descriptors() {
        let mut changed = record();
        let tools = tool_list(&mut changed);
        tools.push(tools[0].clone());
        assert_eq!(
            Definition::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(DefinitionError::DuplicateToolId)
        );
        let mut changed = record();
        tool_list(&mut changed).reverse();
        let definition = Definition::decode(&bytes(&changed), &DecodeLimits::default()).unwrap();
        assert_eq!(
            definition.tools().map(|tool| tool.id).collect::<Vec<_>>(),
            ["pass", "veto"]
        );
    }

    #[test]
    fn authority_bindings_follow_control_and_require_raw_id32_or_null() {
        for authority in [
            Cbor::Null,
            Cbor::Bytes(vec![1; 31]),
            Cbor::Bytes(vec![1; 33]),
            Cbor::Text("binding".into()),
        ] {
            let mut changed = record();
            map(&mut tool_list(&mut changed)[0]).insert("authority_binding".into(), authority);
            assert_eq!(
                Definition::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(DefinitionError::InvalidAuthorityBinding)
            );
        }
        for control in ["continue", "finish"] {
            let mut changed = record();
            map(&mut tool_list(&mut changed)[0])
                .insert("control".into(), Cbor::Text(control.into()));
            let definition =
                Definition::decode(&bytes(&changed), &DecodeLimits::default()).unwrap();
            assert_eq!(
                definition.tools().next().unwrap().authority_binding,
                Some([0x11; 32])
            );
            map(&mut tool_list(&mut changed)[0]).insert("authority_binding".into(), Cbor::Null);
            let definition =
                Definition::decode(&bytes(&changed), &DecodeLimits::default()).unwrap();
            assert_eq!(definition.tools().next().unwrap().authority_binding, None);
        }
    }

    #[test]
    fn model_map_values_are_narrower_than_runtime_scalars() {
        for field in ["inference_profile", "arguments_schema"] {
            for invalid in [
                Cbor::Bytes(vec![0]),
                Cbor::Integer((1u64 << 53).into()),
                Cbor::Float(1.5),
            ] {
                let mut changed = record();
                let target = if field == "inference_profile" {
                    map(&mut changed).get_mut(field).unwrap()
                } else {
                    map(&mut tool_list(&mut changed)[0]).get_mut(field).unwrap()
                };
                map(target).insert("bad".into(), invalid);
                assert!(
                    Definition::decode(&bytes(&changed), &DecodeLimits::default()).is_err(),
                    "{field}"
                );
            }
        }
        let mut changed = record();
        map(&mut tool_list(&mut changed)[0])
            .insert("version".into(), Cbor::Integer(u64::MAX.into()));
        let definition = Definition::decode(&bytes(&changed), &DecodeLimits::default()).unwrap();
        assert_eq!(definition.tools().next().unwrap().version, u64::MAX);
    }

    #[test]
    fn embedded_policy_never_overrides_bootstrap_byte_depth_or_count_bounds() {
        let mut changed = record();
        let limit_map = map(&mut changed)
            .get_mut("limits")
            .unwrap()
            .as_map_mut()
            .unwrap();
        for (_, value) in limit_map {
            *value = Cbor::Integer(u64::MAX.into());
        }
        let encoded = bytes(&changed);
        let exact = DecodeLimits {
            max_block_bytes: encoded.len() as u64,
            ..DecodeLimits::default()
        };
        let definition = Definition::decode(&encoded, &exact).unwrap();
        assert_eq!(definition.limits().max_block_bytes, u64::MAX);
        assert_eq!(definition.limits().max_model_calls_per_episode, u64::MAX);
        let short = DecodeLimits {
            max_block_bytes: exact.max_block_bytes - 1,
            ..exact
        };
        assert_eq!(
            Definition::decode(&encoded, &short),
            Err(DefinitionError::Codec(CodecError::ByteLimit))
        );
        let shallow = DecodeLimits {
            max_depth: 1,
            ..exact
        };
        assert_eq!(
            Definition::decode(&encoded, &shallow),
            Err(DefinitionError::Codec(CodecError::DepthLimit))
        );
        let narrow = DecodeLimits {
            max_collection_entries: 17,
            ..exact
        };
        assert_eq!(
            Definition::decode(&encoded, &narrow),
            Err(DefinitionError::Codec(CodecError::CollectionLimit))
        );
        let mut changed = record();
        map(map(&mut changed).get_mut("limits").unwrap()).remove("max_depth");
        assert_eq!(
            Definition::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(DefinitionError::Limits(LimitsError::InvalidSchema))
        );
    }

    #[test]
    fn incomplete_noncanonical_or_trailing_bytes_never_produce_a_definition() {
        let encoded = bytes(&record());
        for length in 0..encoded.len() {
            assert!(
                Definition::decode(&encoded[..length], &DecodeLimits::default()).is_err(),
                "prefix {length}"
            );
        }
        let mut nonminimal = vec![0xb8, 11];
        nonminimal.extend_from_slice(&encoded[1..]);
        assert_eq!(
            Definition::decode(&nonminimal, &DecodeLimits::default()),
            Err(DefinitionError::Codec(CodecError::NonCanonical))
        );
        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(
            Definition::decode(&trailing, &DecodeLimits::default()),
            Err(DefinitionError::Codec(CodecError::TrailingBytes))
        );
    }
}
