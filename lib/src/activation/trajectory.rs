//! Owns transient turn/result trajectory assembly, not checkpoints or provider JSON.

use super::{ActivationError, ModelReply};
use crate::{
    format::{validate_model_value, DecodeLimits, ModelValue, RecordEncoder, ValueView},
    limits::Limits,
};

pub(super) fn append(
    previous: &ModelValue,
    reply: &ModelReply<'_>,
    arguments: Option<&[u8]>,
    result: &[u8],
    error: Option<&str>,
    limits: &Limits,
) -> Result<ModelValue, ActivationError> {
    let bounds = DecodeLimits {
        max_block_bytes: limits.max_context_bytes,
        max_depth: limits.max_depth.saturating_sub(2),
        max_collection_entries: limits.max_collection_entries,
    };
    // Trajectory is embedded at depth three; call/result values add four more levels.
    let value_bounds = DecodeLimits {
        max_depth: bounds.max_depth.saturating_sub(4),
        ..bounds
    };
    validate_model_value(result, &value_bounds)?;
    if let Some(arguments) = arguments {
        validate_model_value(arguments, &value_bounds)?;
    }
    let old = ValueView::preflighted(previous.as_bytes())
        .list()
        .expect("validated trajectory List");
    let count = old
        .len()
        .checked_add(1)
        .ok_or(ActivationError::UsageOverflow)?;
    let mut writer = RecordEncoder::new(&bounds)?;
    writer.list(count)?;
    for turn in old {
        writer.encoded(turn.expect("validated previous turn").encoded())?;
    }
    writer.map(5)?;
    writer.text("kind")?;
    writer.text("turn")?;
    writer.text("text")?;
    writer.text(reply.text())?;
    writer.text("calls")?;
    writer.list(reply.call_count())?;
    for meta in reply.calls() {
        writer.map(3)?;
        writer.text("call_id")?;
        writer.text(meta.call_id)?;
        writer.text("tool_id")?;
        writer.text(meta.tool_id)?;
        writer.text("arguments")?;
        if let Some(args) = arguments {
            writer.encoded(args)?;
        } else {
            writer.null()?;
        }
    }
    writer.text("error")?;
    if let Some(error) = error {
        writer.text(error)?;
    } else {
        writer.null()?;
    }
    writer.text("results")?;
    writer.list(reply.call_count())?;
    for meta in reply.calls() {
        writer.map(4)?;
        writer.text("ok")?;
        writer.boolean(error.is_none())?;
        writer.text("error")?;
        if let Some(error) = error {
            writer.text(error)?;
        } else {
            writer.null()?;
        }
        writer.text("result")?;
        writer.encoded(result)?;
        writer.text("call_id")?;
        writer.text(meta.call_id)?;
    }
    Ok(ModelValue::from_encoded(writer.finish(), &bounds)?)
}
