//! Owns bounded normalized reply frames and single-call argument selection, not provider
//! JSON/SSE parsing, Usage trust, application schemas, tool execution or checkpoint data.

use super::{InferenceError, InferenceSession, RequestBinding};
use crate::{
    definition::Definition,
    format::{validate_drisl, validate_model_value, CodecError, DecodeLimits, ValueView},
    limits::Limits,
    usage::{Usage, UsageError},
};
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyError {
    Codec(CodecError),
    Usage(UsageError),
    InvalidSchema,
    MultipleToolCalls,
    InvalidArguments,
    DefinitionMismatch,
    Inference(InferenceError),
}
impl From<CodecError> for ReplyError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}
impl From<UsageError> for ReplyError {
    fn from(error: UsageError) -> Self {
        Self::Usage(error)
    }
}
impl From<InferenceError> for ReplyError {
    fn from(error: InferenceError) -> Self {
        Self::Inference(error)
    }
}
impl fmt::Display for ReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Codec(error) => return error.fmt(f),
            Self::Usage(error) => return error.fmt(f),
            Self::Inference(error) => return error.fmt(f),
            Self::InvalidSchema => "normalized reply/call requires its exact closed schema",
            Self::MultipleToolCalls => {
                "all multiple tool calls are rejected before argument selection or effects"
            }
            Self::InvalidArguments => "single tool arguments must be a model Map",
            Self::DefinitionMismatch => "reply limits belong to another Definition",
        })
    }
}
impl core::error::Error for ReplyError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Usage(error) => Some(error),
            Self::Inference(error) => Some(error),
            _ => None,
        }
    }
}

/// Native correlation text, never a persistent identity or tool/version authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallMeta<'a> {
    pub call_id: &'a str,
    pub tool_id: &'a str,
}
/// A selected single call with a canonical model Map. Application validation, registered
/// control/version/authority, budget/units and execution remain required above this view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolCall<'a> {
    meta: CallMeta<'a>,
    arguments: &'a [u8],
}
impl<'a> ToolCall<'a> {
    pub fn meta(self) -> CallMeta<'a> {
        self.meta
    }
    pub fn arguments_bytes(self) -> &'a [u8] {
        self.arguments
    }
}

/// Borrowed normalizer-v1 reply data. It is not one of the six checkpoint block kinds.
/// The closed canonical CBOR schema is {text:Text, calls:list<Call>, usage:Usage,
/// request_id:u64}; Call is {call_id:Text, tool_id:Text, arguments:opaque DRISL value}.
/// Opaque arguments are NOT accepted model data until control_call succeeds. This permits
/// rejecting all multi-call IDs without selecting even a valid first call or materializing
/// an invalid later argument. The complete wire/metadata/Usage schema always preflights.
/// A malformed single argument is a recoverable selection error, not intrinsic Finish.
/// Raw provider JSON failures/truncation are inference failures outside this decoder.
pub struct ModelReply<'a> {
    bytes: &'a [u8],
    request_id: u64,
    text: &'a str,
    usage: Usage,
    calls: ValueView<'a>,
    count: u64,
    bounds: DecodeLimits,
}
impl<'a> ModelReply<'a> {
    /// Caller supplies authorized reply/depth/count limits. max_model_reply_bytes covers
    /// the whole ephemeral frame; max_block_bytes does not cap this non-block object.
    /// Validation may allocate bounded traversal metadata, never an owned payload/call set.
    pub fn decode(bytes: &'a [u8], limits: &Limits) -> Result<Self, ReplyError> {
        let bounds = DecodeLimits {
            max_block_bytes: limits.max_model_reply_bytes,
            max_depth: limits.max_depth,
            max_collection_entries: limits.max_collection_entries,
        };
        validate_drisl(bytes, &bounds)?;
        let fields = ValueView::preflighted(bytes)
            .fields(["request_id", "text", "calls", "usage"])
            .map_err(|_| ReplyError::InvalidSchema)?;
        let request_id = fields[0]
            .unsigned()
            .map_err(|_| ReplyError::InvalidSchema)?;
        let text = fields[1].text().map_err(|_| ReplyError::InvalidSchema)?;
        let calls = fields[2].list().map_err(|_| ReplyError::InvalidSchema)?;
        let count = calls.len();
        let usage = Usage::from_view(fields[3])?;
        for call in calls {
            let call = call_fields(call?);
            let fields = call?;
            call_meta(&fields)?;
        }
        Ok(Self {
            bytes,
            request_id,
            text,
            usage,
            calls: fields[2],
            count,
            bounds,
        })
    }
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn request_id(&self) -> u64 {
        self.request_id
    }
    pub fn text(&self) -> &'a str {
        self.text
    }
    /// Report data only. It never overrides Core call counts or estimates null costs as zero.
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn call_count(&self) -> u64 {
        self.count
    }
    /// All native call metadata, including rejected multi-calls, in provider order.
    /// No argument/handler selection or payload ownership occurs in this iterator.
    pub fn calls(&self) -> impl Iterator<Item = CallMeta<'a>> + 'a {
        self.calls
            .list()
            .expect("validated calls List")
            .map(|call| {
                call_meta(
                    &call_fields(call.expect("validated call span"))
                        .expect("validated call fields"),
                )
                .expect("validated call metadata")
            })
    }
    /// Cardinality precedes argument processing. None is the complete reply's intrinsic
    /// Finish decision; MultipleToolCalls rejects ALL, never a selected first call.
    /// Limits are the immutable decoder-captured bounds, with no later widening/truncation.
    pub fn control_call(&self) -> Result<Option<ToolCall<'a>>, ReplyError> {
        if self.count > 1 {
            return Err(ReplyError::MultipleToolCalls);
        }
        if self.count == 0 {
            return Ok(None);
        }
        let fields = call_fields(
            self.calls
                .list()
                .expect("validated calls List")
                .next()
                .expect("one call")
                .expect("validated call span"),
        )
        .expect("validated call fields");
        validate_model_value(fields[2].encoded(), &self.bounds)?;
        fields[2].map().map_err(|_| ReplyError::InvalidArguments)?;
        Ok(Some(ToolCall {
            meta: call_meta(&fields).expect("validated call metadata"),
            arguments: fields[2].encoded(),
        }))
    }
}
impl InferenceSession {
    /// Correlate before parsing and consume only a complete bounded normalizer-v1 frame.
    /// The body's request ID must also match. Definition/caller reply/depth/count ceilings
    /// intersect; malformed/wrong-body frames preserve pending state. Multiple/invalid
    /// arguments are selected only afterward via control_call, before any handler effect.
    /// Caller is the authorized policy retained by the enclosing runtime, not model input.
    pub fn accept_normalized_reply<'a>(
        &mut self,
        key: RequestBinding,
        bytes: &'a [u8],
        definition: &Definition,
        caller: &Limits,
    ) -> Result<ModelReply<'a>, ReplyError> {
        self.check_response(key)?;
        if self.budget().definition != definition.cid() {
            return Err(ReplyError::DefinitionMismatch);
        }
        let policy = definition.limits();
        let limits = Limits {
            max_model_reply_bytes: caller
                .max_model_reply_bytes
                .min(policy.max_model_reply_bytes),
            max_depth: caller.max_depth.min(policy.max_depth),
            max_collection_entries: caller
                .max_collection_entries
                .min(policy.max_collection_entries),
            ..*policy
        };
        let reply = ModelReply::decode(bytes, &limits)?;
        if reply.request_id() != key.request_id {
            return Err(InferenceError::WrongRequest.into());
        }
        self.accept_reply(key)?;
        Ok(reply)
    }
}

fn call_fields(call: ValueView<'_>) -> Result<[ValueView<'_>; 3], ReplyError> {
    call.fields(["call_id", "tool_id", "arguments"])
        .map_err(|_| ReplyError::InvalidSchema)
}
fn call_meta<'a>(fields: &[ValueView<'a>; 3]) -> Result<CallMeta<'a>, ReplyError> {
    Ok(CallMeta {
        call_id: fields[0].text().map_err(|_| ReplyError::InvalidSchema)?,
        tool_id: fields[1].text().map_err(|_| ReplyError::InvalidSchema)?,
    })
}

#[cfg(test)]
mod tests;
