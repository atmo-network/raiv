//! Maps exact validated guest ModelRequest DATA into the selected Chat Completions provider format.
//! Does not build/truncate context, choose tools, approve a Definition/run or perform HTTP.

mod continuation;
mod turns;

use super::{http::ProviderConfig, Writer};
use crate::output::Request;
pub use continuation::ReasoningSession;
use raiv::{
    definition::Definition,
    format::{validate_model_value, DecodeLimits},
    limits::Limits,
};
use serde_json::{json, Value};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    Binding,
    Context,
    Codec,
    Limit,
    Allocation,
    ReplyMode,
    Continuation,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Continuation => {
                "native reasoning continuation missing/stale/foreign/failed; no replay"
            }
            Self::Binding => "provider request differs from selected Definition/model/source",
            Self::ReplyMode => {
                "provider reasoning presence conflicts with selected thinking profile"
            }
            Self::Context => "guest context structure/policy differs from Definition",
            Self::Codec => "guest model DATA cannot convert to provider JSON",
            Self::Limit => "provider request bound; no context truncation",
            Self::Allocation => "provider request allocation failed",
        })
    }
}

impl std::error::Error for RequestError {}

fn value(bytes: &[u8], bounds: &DecodeLimits) -> Result<Value, RequestError> {
    if bounds.max_depth > 64 {
        return Err(RequestError::Limit);
    }
    validate_model_value(bytes, bounds).map_err(|_| RequestError::Codec)?;
    fn item(d: &mut minicbor::Decoder<'_>) -> Result<Value, RequestError> {
        use minicbor::data::Type;
        Ok(match d.datatype().map_err(|_| RequestError::Codec)? {
            Type::Null => {
                d.null().map_err(|_| RequestError::Codec)?;
                Value::Null
            }
            Type::Bool => Value::Bool(d.bool().map_err(|_| RequestError::Codec)?),
            Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::I8
            | Type::I16
            | Type::I32
            | Type::I64 => json!(d.i64().map_err(|_| RequestError::Codec)?),
            Type::String => Value::String(d.str().map_err(|_| RequestError::Codec)?.into()),
            Type::Array => {
                let n = d
                    .array()
                    .map_err(|_| RequestError::Codec)?
                    .ok_or(RequestError::Codec)?;
                let mut values = Vec::new();
                values
                    .try_reserve_exact(usize::try_from(n).map_err(|_| RequestError::Limit)?)
                    .map_err(|_| RequestError::Allocation)?;
                for _ in 0..n {
                    values.push(item(d)?);
                }
                Value::Array(values)
            }
            Type::Map => {
                let n = d
                    .map()
                    .map_err(|_| RequestError::Codec)?
                    .ok_or(RequestError::Codec)?;
                let mut map = serde_json::Map::new();
                for _ in 0..n {
                    let key = d.str().map_err(|_| RequestError::Codec)?.into();
                    map.insert(key, item(d)?);
                }
                Value::Object(map)
            }
            _ => return Err(RequestError::Codec),
        })
    }
    item(&mut minicbor::Decoder::new(bytes))
}

/// Native caller still owns approved module/Definition/Run/RequestTicket and credential binding.
/// Explicit compiled thinking:false selects a disabled-thinking request, not implicit reasoning loss.
pub fn encode_chat_request(
    definition: &Definition,
    request: Request<'_>,
    config: &ProviderConfig,
    caller: &Limits,
) -> Result<Vec<u8>, RequestError> {
    encode_inner(definition, request, config, caller, None)
}

fn encode_inner(
    definition: &Definition,
    request: Request<'_>,
    config: &ProviderConfig,
    caller: &Limits,
    records: Option<&[super::NormalizedReply]>,
) -> Result<Vec<u8>, RequestError> {
    let profile = guest::ChatProfile::decode(definition.inference_profile_bytes())
        .map_err(|_| RequestError::Binding)?;
    if profile.base_url != config.base_url
        || profile.model != config.model
        || !matches!(
            config.protocol.as_str(),
            "openai-chat-completions" | "openai-responses"
        )
        || request.model_parameters_bytes() != definition.inference_profile_bytes()
        || config.max_tokens == 0
    {
        return Err(RequestError::Binding);
    }
    if profile.reasoning_effort.is_some() != records.is_some() {
        return Err(RequestError::Continuation);
    }
    let bounds = DecodeLimits {
        max_block_bytes: caller
            .max_context_bytes
            .min(request.limits().max_context_bytes),
        max_depth: caller.max_depth.min(request.limits().max_depth),
        max_collection_entries: caller
            .max_collection_entries
            .min(request.limits().max_collection_entries),
    };
    let context = value(request.context_bytes(), &bounds)?;
    let sections = context
        .as_array()
        .filter(|v| v.len() == 7)
        .ok_or(RequestError::Context)?;
    if sections[0].get("kind").and_then(Value::as_str) != Some("policy")
        || sections[0].get("policy").and_then(Value::as_str) != Some(definition.policy())
        || sections[0].get("mandate").and_then(Value::as_str) != Some(definition.mandate())
        || sections[6].get("kind").and_then(Value::as_str) != Some("ram_trajectory")
    {
        return Err(RequestError::Context);
    }
    let mut tools = Vec::new();
    let mut last = None;
    for tool in request.tools() {
        let index = definition
            .tools()
            .position(|d| d == tool)
            .ok_or(RequestError::Binding)?;
        if last.is_some_and(|prior| index <= prior) {
            return Err(RequestError::Binding);
        }
        last = Some(index);
        let schema = value(
            tool.arguments_schema,
            &DecodeLimits {
                max_block_bytes: caller.max_block_bytes.min(request.limits().max_block_bytes),
                ..bounds
            },
        )?;
        tools.try_reserve(1).map_err(|_| RequestError::Allocation)?;
        // Preserve schemas verbatim; natural completion is not a provider function.
        tools.push(json!({"type":"function","function":{"name":tool.id,"description":tool.description,"parameters":schema}}));
    }
    // Roles are transport encoding of the guest's trusted first section and complete remaining DATA.
    // No fields, history, initial Memory or RAM results are reconstructed/removed/rewritten.
    let system = serde_json::to_string(&sections[0]).map_err(|_| RequestError::Codec)?;
    let data = serde_json::to_string(&sections[1..]).map_err(|_| RequestError::Codec)?;
    let mut messages = vec![
        json!({"role":"system","content":system}),
        json!({"role":"user","content":data}),
    ];
    turns::append(&mut messages, &sections[6])?;
    if let Some(records) = records {
        continuation::attach(records, &mut messages, caller)?;
    }
    let mut body = json!({"model":profile.model,"messages":messages,"max_tokens":config.max_tokens,"stream":config.stream,"n":1});
    // `thinking` is a DeepSeek extension; generic OpenAI-compatible requests carry no extensions.
    if profile.provider == guest::Provider::OpencodeGo {
        body["thinking"] = json!({"type":"disabled"});
    }
    // Tool-free inference is valid; omit API fields that require a nonempty tool list.
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
        body["parallel_tool_calls"] = json!(false);
    }
    if let Some(effort) = profile.reasoning_effort {
        body["thinking"] = json!({"type":"enabled"});
        body["reasoning_effort"] = json!(effort.as_str());
    }
    if config.stream {
        body["stream_options"] = json!({"include_usage":true});
    }
    let mut writer = Writer {
        bytes: Vec::new(),
        cap: config.max_request_bytes,
    };
    serde_json::to_writer(&mut writer, &body).map_err(|_| RequestError::Limit)?;
    Ok(writer.bytes)
}

/// Enabled tool turns need replayable reasoning; a complete no-call final has no continuation.
/// Disabled mode still rejects nonempty reasoning. Completeness is proven by normalization/EOF.
/// A generic OpenAI-compatible profile requested no reasoning mode, so provider reasoning text
/// is transient DATA that is never replayed, stored or shown to the guest.
pub fn check_chat_reply_mode(
    definition: &Definition,
    reply: &super::NormalizedReply,
) -> Result<(), RequestError> {
    let profile = guest::ChatProfile::decode(definition.inference_profile_bytes())
        .map_err(|_| RequestError::Binding)?;
    if profile.provider == guest::Provider::OpenAiCompatible {
        return Ok(());
    }
    if profile.reasoning_effort.is_some() {
        if reply.reasoning_content().is_some() {
            return Ok(());
        }
        // Inspect complete normalized DATA only, never a provisional stream or missing-call fallback.
        let parsed = raiv::activation::ModelReply::decode(reply.as_bytes(), definition.limits())
            .map_err(|_| RequestError::ReplyMode)?;
        return if parsed.call_count() == 0 {
            Ok(())
        } else {
            Err(RequestError::ReplyMode)
        };
    }
    if reply
        .reasoning_content()
        .is_some_and(|value| !value.is_empty())
    {
        return Err(RequestError::ReplyMode);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
