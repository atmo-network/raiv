//! Owns the immutable Command data contract, not tool planning, dispatch, or host authority.

use crate::format::{
    validate_drisl, validate_model_value, CodecError, Id32, RecordEncoder, ValueView,
};
use crate::limits::Limits;
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    Codec(CodecError),
    InvalidSchema,
}

impl From<CodecError> for CommandError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::InvalidSchema => {
                f.write_str("Command requires exact Id32/tool/version/authority/model-map fields")
            }
        }
    }
}

impl core::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// Borrowed planner data. Arguments must be one canonical model Map.
/// The core/host still binds this command to its actual logical action slot and tool policy.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec<'a> {
    pub action_id: Id32,
    pub tool_id: &'a str,
    pub tool_version: u64,
    pub authority_binding: Id32,
    pub arguments: &'a [u8],
}

/// A borrowed, schema-validated Command, with no owned payload or execution authority.
/// Runtime IDs/version stay typed metadata; arguments remain a bounded model Map.
#[derive(Clone, Copy)]
pub struct CommandView<'a> {
    bytes: &'a [u8],
    fields: [ValueView<'a>; 5],
}
impl<'a> CommandView<'a> {
    /// Validate caller bytes under whole-record/contextual limits, then borrow them.
    /// Preflight may allocate bounded traversal metadata, never another payload set.
    pub fn decode(bytes: &'a [u8], limits: &Limits) -> Result<Self, CommandError> {
        Command::validate_encoded(bytes, limits)?;
        Ok(Self::from_validated(bytes))
    }
    pub(crate) fn from_validated(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            fields: validated_fields(bytes),
        }
    }
    pub fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }
    pub fn action_id(self) -> Id32 {
        self.fields[0]
            .bytes()
            .expect("validated action ID")
            .try_into()
            .expect("validated Id32")
    }
    pub fn tool_id(self) -> &'a str {
        self.fields[1].text().expect("validated tool ID")
    }
    pub fn tool_version(self) -> u64 {
        self.fields[2].unsigned().expect("validated tool version")
    }
    pub fn authority_binding(self) -> Id32 {
        self.fields[3]
            .bytes()
            .expect("validated authority")
            .try_into()
            .expect("validated Id32")
    }
    pub fn arguments_bytes(self) -> &'a [u8] {
        self.fields[4].encoded()
    }
}

/// One validated canonical Command subrecord, embedded in a Boundary, not a state block.
/// All byte limits cover this entire record. No method plans or executes effects, matches
/// a whitelist, authenticates an issuer, or treats this record as accepted authority.
#[derive(Debug, Eq, PartialEq)]
pub struct Command {
    encoded: Vec<u8>,
    action_id: Id32,
    tool_id: Range<usize>,
    tool_version: u64,
    authority_binding: Id32,
    arguments: Range<usize>,
}

const COMMAND_FIELDS: [&str; 5] = [
    "action_id",
    "tool_id",
    "tool_version",
    "authority_binding",
    "arguments",
];

impl Command {
    /// Encode one immutable planner record, without dispatching or authorizing it.
    ///
    /// ```
    /// use raiv::command::{Command, CommandSpec};
    /// use raiv::limits::Limits;
    /// let spec = CommandSpec { action_id: [1; 32], tool_id: "trade", tool_version: 1,
    ///     authority_binding: [2; 32], arguments: &[0xa0] };
    /// let command = Command::encode(&spec, &Limits::default())?;
    /// assert_eq!(command.tool_id(), "trade");
    /// # Ok::<(), raiv::command::CommandError>(())
    /// ```
    pub fn encode(spec: &CommandSpec<'_>, limits: &Limits) -> Result<Self, CommandError> {
        let bounds = command_bounds(limits);
        if bounds.max_depth < 2 {
            return Err(CodecError::DepthLimit.into());
        }
        let argument_bounds = crate::format::DecodeLimits {
            max_depth: bounds.max_depth - 1,
            ..bounds
        };
        validate_model_value(spec.arguments, &argument_bounds)?;
        ValueView::preflighted(spec.arguments)
            .map()
            .map_err(|_| CommandError::InvalidSchema)?;
        let mut encoder = RecordEncoder::new(&bounds)?;
        encoder.map(5)?;
        // Preferred CBOR string keys: length first, then encoded payload bytes.
        encoder.text("tool_id")?;
        encoder.text(spec.tool_id)?;
        encoder.text("action_id")?;
        encoder.bytes(&spec.action_id)?;
        encoder.text("arguments")?;
        encoder.encoded(spec.arguments)?;
        encoder.text("tool_version")?;
        encoder.unsigned(spec.tool_version)?;
        encoder.text("authority_binding")?;
        encoder.bytes(&spec.authority_binding)?;
        let encoded = encoder.finish();
        let mut command = Self::inspect(&encoded, limits)?;
        command.encoded = encoded;
        Ok(command)
    }

    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Self, CommandError> {
        let mut command = Self::inspect(bytes, limits)?;
        command.encoded = crate::format::copy_validated(bytes)?;
        Ok(command)
    }

    /// Check contextual embedding bounds without allocating another command payload.
    pub(crate) fn validate_encoded(bytes: &[u8], limits: &Limits) -> Result<(), CommandError> {
        Self::inspect(bytes, limits).map(|_| ())
    }

    /// Boundary owns immutable bytes already checked as a complete Command.
    pub(crate) fn action_id_in_validated(bytes: &[u8]) -> Id32 {
        CommandView::from_validated(bytes).action_id()
    }

    /// Borrow only the dispatch descriptor; Boundary already validated the entire Command.
    pub(crate) fn descriptor_in_validated(bytes: &[u8]) -> (&str, u64, Id32) {
        let view = CommandView::from_validated(bytes);
        (
            view.tool_id(),
            view.tool_version(),
            view.authority_binding(),
        )
    }

    fn inspect(bytes: &[u8], limits: &Limits) -> Result<Self, CommandError> {
        let bounds = command_bounds(limits);
        validate_drisl(bytes, &bounds)?;
        let fields = ValueView::preflighted(bytes)
            .fields(COMMAND_FIELDS)
            .map_err(|_| CommandError::InvalidSchema)?;
        let action_id = fields[0]
            .bytes()
            .map_err(|_| CommandError::InvalidSchema)?
            .try_into()
            .map_err(|_| CommandError::InvalidSchema)?;
        let tool_id = fields[1]
            .text_span()
            .map_err(|_| CommandError::InvalidSchema)?;
        let tool_version = fields[2]
            .unsigned()
            .map_err(|_| CommandError::InvalidSchema)?;
        let authority_binding = fields[3]
            .bytes()
            .map_err(|_| CommandError::InvalidSchema)?
            .try_into()
            .map_err(|_| CommandError::InvalidSchema)?;
        fields[4].map().map_err(|_| CommandError::InvalidSchema)?;
        validate_model_value(fields[4].encoded(), &bounds)?;
        Ok(Self {
            encoded: Vec::new(),
            action_id,
            tool_id,
            tool_version,
            authority_binding,
            arguments: fields[4].span(),
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    /// Borrow the already validated record without preflight or another payload copy.
    pub fn view(&self) -> CommandView<'_> {
        CommandView::from_validated(self.as_bytes())
    }

    pub fn action_id(&self) -> Id32 {
        self.action_id
    }

    pub fn tool_id(&self) -> &str {
        core::str::from_utf8(&self.encoded[self.tool_id.clone()])
            .expect("validated immutable Command text")
    }

    pub fn tool_version(&self) -> u64 {
        self.tool_version
    }

    pub fn authority_binding(&self) -> Id32 {
        self.authority_binding
    }

    pub fn arguments_bytes(&self) -> &[u8] {
        &self.encoded[self.arguments.clone()]
    }
}

fn validated_fields(bytes: &[u8]) -> [ValueView<'_>; 5] {
    ValueView::preflighted(bytes)
        .fields(COMMAND_FIELDS)
        .expect("validated immutable Command fields")
}

fn command_bounds(limits: &Limits) -> crate::format::DecodeLimits {
    let mut bounds = limits.decode_limits();
    bounds.max_block_bytes = bounds.max_block_bytes.min(limits.max_command_bytes);
    bounds
}

#[cfg(test)]
mod tests {
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
        serde_json::from_str(include_str!("../../fixtures/format/command.json")).unwrap()
    }

    fn record() -> Cbor {
        node(&fixtures()[0]["record"])
    }
    fn spec(command: &Command) -> CommandSpec<'_> {
        CommandSpec {
            action_id: command.action_id(),
            tool_id: command.tool_id(),
            tool_version: command.tool_version(),
            authority_binding: command.authority_binding(),
            arguments: command.arguments_bytes(),
        }
    }

    #[test]
    fn borrowed_command_views_match_ciborium_owned_fields_and_original_payload_pointers() {
        for fixture in fixtures().as_array().unwrap() {
            let source = bytes(&node(&fixture["record"]));
            assert_eq!(source, hex(fixture["hex"].as_str().unwrap()));
            let view = CommandView::decode(&source, &Limits::default()).unwrap();
            let owned = Command::decode(&source, &Limits::default()).unwrap();
            assert_eq!(view.as_bytes().as_ptr(), source.as_ptr());
            assert_eq!(view.action_id(), owned.action_id());
            assert_eq!(view.tool_id(), owned.tool_id());
            assert_eq!(view.tool_version(), owned.tool_version());
            assert_eq!(view.authority_binding(), owned.authority_binding());
            let arguments = bytes(&node(&fixture["record"]["arguments"]));
            assert_eq!(view.arguments_bytes(), arguments);
            assert_eq!(
                view.arguments_bytes().as_ptr(),
                source
                    .windows(arguments.len())
                    .find(|slice| *slice == arguments)
                    .unwrap()
                    .as_ptr()
            );
            assert_eq!(owned.view().as_bytes().as_ptr(), owned.as_bytes().as_ptr());
            assert_eq!(
                owned.view().arguments_bytes().as_ptr(),
                owned.arguments_bytes().as_ptr()
            );
            assert_eq!(
                view.tool_version(),
                fixture["record"]["tool_version"].as_u64().unwrap()
            );
        }
    }

    #[test]
    fn borrowed_command_validation_matches_owned_errors_for_bounds_prefixes_and_schema_mutations() {
        let valid = bytes(&record());
        for limit in [
            Limits::default(),
            Limits {
                max_command_bytes: valid.len() as u64 - 1,
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
                max_collection_entries: 4,
                ..Limits::default()
            },
        ] {
            for end in 0..=valid.len() {
                let data = &valid[..end];
                assert_eq!(
                    CommandView::decode(data, &limit).err(),
                    Command::decode(data, &limit).err()
                );
            }
        }
        for key in COMMAND_FIELDS {
            let mut value = record();
            map(&mut value).remove(key);
            let data = bytes(&value);
            assert_eq!(
                CommandView::decode(&data, &Limits::default()).err(),
                Some(CommandError::InvalidSchema)
            );
        }
        for arguments in [
            Cbor::Bytes(vec![]),
            Cbor::Array(vec![]),
            Cbor::Map(vec![(
                Cbor::Text("x".into()),
                Cbor::Integer(u64::MAX.into()),
            )]),
        ] {
            let mut value = record();
            map(&mut value).insert("arguments".into(), arguments);
            let data = bytes(&value);
            assert!(CommandView::decode(&data, &Limits::default()).is_err());
            assert_eq!(
                CommandView::decode(&data, &Limits::default()).err(),
                Command::decode(&data, &Limits::default()).err()
            );
        }
    }

    #[test]
    fn complete_command_vectors_match_independent_codec_hash_and_owned_encoding() {
        let fixtures = fixtures();
        for fixture in fixtures.as_array().unwrap() {
            let expected = hex(fixture["hex"].as_str().unwrap());
            assert_eq!(bytes(&node(&fixture["record"])), expected);
            assert_eq!(
                &Sha256::digest(&expected)[..],
                &hex(fixture["sha256"].as_str().unwrap())[..]
            );
            let command = Command::decode(&expected, &Limits::default()).unwrap();
            assert_eq!(command.as_bytes(), expected);
            assert_eq!(command.action_id(), [0x10; 32]);
            assert_eq!(command.authority_binding(), [0x20; 32]);
            assert_eq!(command.tool_id(), "trade");
            assert_eq!(
                command.tool_version(),
                fixture["record"]["tool_version"].as_u64().unwrap()
            );
            assert_eq!(
                command.arguments_bytes(),
                bytes(&node(&fixture["record"]["arguments"]))
            );
            assert_eq!(
                Command::encode(&spec(&command), &Limits::default()),
                Ok(command)
            );
        }
    }

    #[test]
    fn command_whitelist_is_exact_and_no_field_is_defaulted() {
        let mut source = record();
        let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut changed = source.clone();
            map(&mut changed).remove(&key);
            assert_eq!(
                Command::decode(&bytes(&changed), &Limits::default()),
                Err(CommandError::InvalidSchema)
            );
            map(&mut changed).insert("unknown".into(), Cbor::Null);
            assert_eq!(
                Command::decode(&bytes(&changed), &Limits::default()),
                Err(CommandError::InvalidSchema)
            );
        }
        for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(0.into())] {
            assert_eq!(
                Command::decode(&bytes(&value), &Limits::default()),
                Err(CommandError::InvalidSchema)
            );
        }
    }

    #[test]
    fn command_ids_are_exact_raw_scalars_and_other_fields_are_not_coerced() {
        for field in ["action_id", "authority_binding"] {
            for value in [
                Cbor::Null,
                Cbor::Bytes(vec![0; 31]),
                Cbor::Bytes(vec![0; 33]),
                Cbor::Text("cid-or-provider-id".into()),
            ] {
                let mut changed = record();
                map(&mut changed).insert(field.into(), value);
                assert_eq!(
                    Command::decode(&bytes(&changed), &Limits::default()),
                    Err(CommandError::InvalidSchema)
                );
            }
        }
        for (field, value) in [
            ("tool_id", Cbor::Null),
            ("tool_version", Cbor::Integer((-1).into())),
            ("tool_version", Cbor::Text("1".into())),
            ("arguments", Cbor::Array(vec![])),
        ] {
            let mut changed = record();
            map(&mut changed).insert(field.into(), value);
            assert_eq!(
                Command::decode(&bytes(&changed), &Limits::default()),
                Err(CommandError::InvalidSchema)
            );
        }
    }

    #[test]
    fn argument_values_do_not_inherit_runtime_bytes_links_floats_or_integer_widths() {
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
            let mut changed = record();
            map(map(&mut changed).get_mut("arguments").unwrap()).insert("bad".into(), value);
            assert!(Command::decode(&bytes(&changed), &Limits::default()).is_err());
        }
        let source = Command::decode(&bytes(&record()), &Limits::default()).unwrap();
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
            input.arguments = &raw;
            assert!(
                Command::encode(&input, &Limits::default()).is_err(),
                "{data}"
            );
        }
    }

    #[test]
    fn command_limit_covers_the_whole_record_not_only_arguments() {
        let source = Command::decode(&bytes(&record()), &Limits::default()).unwrap();
        let exact = Limits {
            max_command_bytes: source.as_bytes().len() as u64,
            ..Limits::default()
        };
        assert_eq!(Command::encode(&spec(&source), &exact).unwrap(), source);
        let short = Limits {
            max_command_bytes: exact.max_command_bytes - 1,
            ..exact
        };
        assert_eq!(
            Command::decode(source.as_bytes(), &short),
            Err(CommandError::Codec(CodecError::ByteLimit))
        );
        assert_eq!(
            Command::encode(&spec(&source), &short),
            Err(CommandError::Codec(CodecError::ByteLimit))
        );
        let block_short = Limits {
            max_block_bytes: exact.max_command_bytes - 1,
            max_command_bytes: u64::MAX,
            ..exact
        };
        assert_eq!(
            Command::decode(source.as_bytes(), &block_short),
            Err(CommandError::Codec(CodecError::ByteLimit))
        );
        let count_short = Limits {
            max_collection_entries: 4,
            ..exact
        };
        assert_eq!(
            Command::encode(&spec(&source), &count_short),
            Err(CommandError::Codec(CodecError::CollectionLimit))
        );
        assert_eq!(
            Command::decode(source.as_bytes(), &count_short),
            Err(CommandError::Codec(CodecError::CollectionLimit))
        );
    }

    #[test]
    fn embedded_argument_depth_and_owned_output_are_bounded_independently() {
        let source = Command::decode(&bytes(&record()), &Limits::default()).unwrap();
        let exact = Limits {
            max_depth: 3,
            ..Limits::default()
        };
        assert_eq!(Command::encode(&spec(&source), &exact).unwrap(), source);
        let shallow = Limits {
            max_depth: 2,
            ..exact
        };
        assert_eq!(
            Command::encode(&spec(&source), &shallow),
            Err(CommandError::Codec(CodecError::DepthLimit))
        );
        let nested = hex("a1616181f6");
        let mut input = spec(&source);
        input.arguments = &nested;
        assert_eq!(
            Command::encode(&input, &exact),
            Err(CommandError::Codec(CodecError::DepthLimit))
        );
        input.arguments = &[0xa0];
        assert!(Command::encode(&input, &shallow).is_ok());
        let owned = Command::encode(&spec(&source), &Limits::default()).unwrap();
        drop(source);
        assert_eq!(owned.tool_id(), "trade");
    }

    #[test]
    fn malformed_truncated_duplicate_and_trailing_commands_never_decode() {
        let record = bytes(&record());
        for length in 0..record.len() {
            assert!(Command::decode(&record[..length], &Limits::default()).is_err());
        }
        let duplicate = hex("a267746f6f6c5f6964616167746f6f6c5f69646162");
        assert_eq!(
            Command::decode(&duplicate, &Limits::default()),
            Err(CommandError::Codec(CodecError::DuplicateKey))
        );
        let mut nonminimal = vec![0xb8, 5];
        nonminimal.extend_from_slice(&record[1..]);
        assert_eq!(
            Command::decode(&nonminimal, &Limits::default()),
            Err(CommandError::Codec(CodecError::NonCanonical))
        );
        let mut trailing = record;
        trailing.push(0);
        assert_eq!(
            Command::decode(&trailing, &Limits::default()),
            Err(CommandError::Codec(CodecError::TrailingBytes))
        );
    }
}
