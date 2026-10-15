//! Owns the transient model-compatible history fragment, not provider messages or checkpoints.
//! Fixed section order and schema live here; canonical codec primitives stay in format.

use super::{EpisodeProjection, HistoryProjection};
use crate::{
    command::CommandView,
    format::{validate_model_value, CodecError, DecodeLimits, Id32, ModelValue, RecordEncoder},
    input::InputError,
    limits::Limits,
    memory::Memory,
    position::Position,
    receipt::ReceiptView,
};
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    Codec(CodecError),
    ContextLimit,
    Input(InputError),
    WrongPhase,
    ReceiptMismatch,
    TrajectoryNotList,
}

impl From<CodecError> for ProjectionError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::ByteLimit => Self::ContextLimit,
            other => Self::Codec(other),
        }
    }
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::ContextLimit => f.write_str("projection exceeds context byte limit"),
            Self::Input(error) => error.fmt(f),
            Self::WrongPhase => f.write_str("input cause does not match checkpoint continuation"),
            Self::ReceiptMismatch => f.write_str("input Receipt does not match pending Command"),
            Self::TrajectoryNotList => f.write_str("RAM trajectory must be a model List"),
        }
    }
}

impl core::error::Error for ProjectionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Input(error) => Some(error),
            _ => None,
        }
    }
}

impl HistoryProjection<'_> {
    /// Encode the complete retained-history/current-chain fragment as model-compatible CBOR.
    /// The two ordered sections are `{kind: "completed_history", episodes: [...]}` then
    /// `{kind: "current_chain", episode: ... | null}`. Episode fields are id, closed,
    /// actions, entry_memory and latest_memory (final for a closed Episode).
    /// Actions retain all Command fields plus receipt. Receipt/Position retain all fields.
    /// Runtime Id32 values become lowercase hex text; full-u64 metadata becomes canonical
    /// decimal text, never lossy model integers. Model Maps are copied as validated spans;
    /// Memory is its compact two-plane value, never its runtime record or a made-up map.
    ///
    /// Caller and Definition context/depth/count limits only tighten. No Episode truncation
    /// or fallback is allowed. max_context_bytes bounds this whole transient fragment,
    /// not max_block_bytes: it is not a structural block or checkpoint. Full ModelRequest
    /// accounting must ALSO include policy, tools, new prepared input, live working memory
    /// and RAM trajectory. This method neither admits that input nor performs inference.
    pub fn encode(self, caller: &Limits) -> Result<ModelValue, ProjectionError> {
        let bounds = context_bounds(self.restored.definition().limits(), caller);
        let mut writer = RecordEncoder::new(&bounds)?;
        list(&mut writer, 2, 1, &bounds)?;
        history_sections(&mut writer, self, &bounds)?;
        Ok(ModelValue::from_encoded(writer.finish(), &bounds)?)
    }
}

pub(super) fn context_bounds(definition: &Limits, caller: &Limits) -> DecodeLimits {
    DecodeLimits {
        max_block_bytes: caller.max_context_bytes.min(definition.max_context_bytes),
        max_depth: caller.max_depth.min(definition.max_depth),
        max_collection_entries: caller
            .max_collection_entries
            .min(definition.max_collection_entries),
    }
}

pub(super) fn history_sections(
    writer: &mut RecordEncoder,
    history: HistoryProjection<'_>,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    map(writer, 2, 2, bounds)?;
    writer.text("kind")?;
    writer.text("completed_history")?;
    writer.text("episodes")?;
    list(writer, history.completed().count() as u64, 3, bounds)?;
    for episode in history.completed() {
        encode_episode(writer, episode, 4, bounds)?;
    }
    map(writer, 2, 2, bounds)?;
    writer.text("kind")?;
    writer.text("current_chain")?;
    writer.text("episode")?;
    if let Some(episode) = history.current() {
        encode_episode(writer, episode, 3, bounds)?;
    } else {
        writer.null()?;
    }
    Ok(())
}

fn container(count: u64, depth: u64, bounds: &DecodeLimits) -> Result<(), CodecError> {
    if depth > bounds.max_depth || (count != 0 && depth >= bounds.max_depth) {
        return Err(CodecError::DepthLimit);
    }
    if count > bounds.max_collection_entries {
        return Err(CodecError::CollectionLimit);
    }
    Ok(())
}

pub(super) fn map(
    writer: &mut RecordEncoder,
    count: u64,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    container(count, depth, bounds)?;
    writer.map(count)
}

pub(super) fn list(
    writer: &mut RecordEncoder,
    count: u64,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    container(count, depth, bounds)?;
    writer.list(count)
}

pub(super) fn model(
    writer: &mut RecordEncoder,
    bytes: &[u8],
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    let nested = DecodeLimits {
        max_depth: bounds.max_depth.saturating_sub(depth.saturating_sub(1)),
        ..*bounds
    };
    validate_model_value(bytes, &nested)?;
    writer.encoded(bytes)
}

pub(super) fn memory(
    writer: &mut RecordEncoder,
    value: Option<&Memory>,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    if let Some(memory) = value {
        model(writer, memory.value_bytes(), depth, bounds)
    } else {
        writer.null()
    }
}

pub(super) fn id(writer: &mut RecordEncoder, value: Id32) -> Result<(), CodecError> {
    let mut text = [0u8; 64];
    data_encoding::HEXLOWER.encode_mut(&value, &mut text);
    writer.text(core::str::from_utf8(&text).expect("hex encoder emits ASCII"))
}

pub(super) fn number(writer: &mut RecordEncoder, value: u64) -> Result<(), CodecError> {
    struct Decimal {
        bytes: [u8; 20],
        len: usize,
    }
    impl fmt::Write for Decimal {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            let end = self
                .len
                .checked_add(text.len())
                .filter(|&end| end <= self.bytes.len())
                .ok_or(fmt::Error)?;
            self.bytes[self.len..end].copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    let mut output = Decimal {
        bytes: [0; 20],
        len: 0,
    };
    fmt::write(&mut output, format_args!("{value}")).expect("u64 decimal fits twenty ASCII bytes");
    writer.text(
        core::str::from_utf8(&output.bytes[..output.len]).expect("integer formatter emits ASCII"),
    )
}

pub(super) fn position(
    writer: &mut RecordEncoder,
    value: Position,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    map(writer, 3, depth, bounds)?;
    writer.text("domain")?;
    id(writer, value.domain)?;
    writer.text("number")?;
    number(writer, value.number)?;
    writer.text("reference")?;
    if let Some(reference) = value.reference {
        id(writer, reference)?;
    } else {
        writer.null()?;
    }
    Ok(())
}

pub(super) fn receipt(
    writer: &mut RecordEncoder,
    value: ReceiptView<'_>,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    map(writer, 7, depth, bounds)?;
    writer.text("costs")?;
    model(writer, value.costs_bytes(), depth + 1, bounds)?;
    writer.text("result")?;
    model(writer, value.result_bytes(), depth + 1, bounds)?;
    writer.text("outcome")?;
    writer.text(value.outcome().as_str())?;
    writer.text("action_id")?;
    id(writer, value.action_id())?;
    writer.text("finalized_at")?;
    position(writer, value.finalized_at(), depth + 1, bounds)?;
    writer.text("command_digest")?;
    id(writer, value.command_digest())?;
    writer.text("execution_reference")?;
    if let Some(reference) = value.execution_reference() {
        writer.text(reference)?;
    } else {
        writer.null()?;
    }
    Ok(())
}

fn command(
    writer: &mut RecordEncoder,
    value: CommandView<'_>,
    result: Option<ReceiptView<'_>>,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    map(writer, 6, depth, bounds)?;
    writer.text("receipt")?;
    if let Some(result) = result {
        receipt(writer, result, depth + 1, bounds)?;
    } else {
        writer.null()?;
    }
    writer.text("tool_id")?;
    writer.text(value.tool_id())?;
    writer.text("action_id")?;
    id(writer, value.action_id())?;
    writer.text("arguments")?;
    model(writer, value.arguments_bytes(), depth + 1, bounds)?;
    writer.text("tool_version")?;
    number(writer, value.tool_version())?;
    writer.text("authority_binding")?;
    id(writer, value.authority_binding())?;
    Ok(())
}

fn encode_episode(
    writer: &mut RecordEncoder,
    value: EpisodeProjection<'_>,
    depth: u64,
    bounds: &DecodeLimits,
) -> Result<(), CodecError> {
    map(writer, 5, depth, bounds)?;
    writer.text("id")?;
    id(writer, value.id())?;
    writer.text("closed")?;
    writer.boolean(value.closed())?;
    writer.text("actions")?;
    list(writer, value.actions().count() as u64, depth + 1, bounds)?;
    for action in value.actions() {
        command(
            writer,
            action.command(),
            action.receipt(),
            depth + 2,
            bounds,
        )?;
    }
    writer.text("entry_memory")?;
    memory(writer, value.entry_memory(), depth + 1, bounds)?;
    writer.text("latest_memory")?;
    memory(writer, value.latest_memory(), depth + 1, bounds)?;
    Ok(())
}
