//! Host-only Chat Completions HTTPS/JSON/SSE ports. No guest decisions or acceptance.
pub mod http;
mod json;
#[cfg(target_os = "linux")]
pub mod request;
pub mod sse;
use raiv::{activation::ModelReply, format::DecodeLimits, limits::Limits, usage::Usage};
use serde_json::Value;
use std::fmt;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizeError {
    Json,
    Schema,
    Model,
    Incomplete,
    Limit,
    Allocation,
    Wire,
}
impl fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Json => "provider JSON invalid or bounded duplicate preflight failed",
            Self::Schema => "provider completion schema invalid",
            Self::Model => "provider model differs from selected exact model",
            Self::Incomplete => "provider reply is incomplete, refused or filtered",
            Self::Limit => "provider/normalized reply resource bound",
            Self::Allocation => "provider normalization allocation failed",
            Self::Wire => "normalized reply cannot encode required DRISL DATA",
        })
    }
}
impl std::error::Error for NormalizeError {}
/// Ephemeral native normalization result; not admission, checkpoint or successful stream evidence.
/// Reasoning belongs only to the live native provider continuation, never guest text/state.
pub struct NormalizedReply {
    bytes: Vec<u8>,
    reasoning: Option<String>,
}
impl NormalizedReply {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn reasoning_content(&self) -> Option<&str> {
        self.reasoning.as_deref()
    }
}
struct Writer {
    bytes: Vec<u8>,
    cap: u64,
}
impl minicbor::encode::Write for Writer {
    type Error = NormalizeError;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(NormalizeError::Limit)?;
        if next as u64 > self.cap {
            return Err(NormalizeError::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| NormalizeError::Allocation)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
type Encoder = minicbor::Encoder<Writer>;
fn encoded(error: minicbor::encode::Error<NormalizeError>) -> NormalizeError {
    error.into_write().unwrap_or(NormalizeError::Wire)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, NormalizeError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(NormalizeError::Schema)
}
fn optional_count(value: Option<&Value>) -> Result<Option<u64>, NormalizeError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or(NormalizeError::Schema),
    }
}
// DRISL arguments remain opaque here: valid scalars/arrays/full-u64 reach Core's recoverable
// single-call model-Map selection. Floats cannot enter DRISL; no numeric/string coercion.
fn argument(value: &Value, e: &mut Encoder) -> Result<(), NormalizeError> {
    match value {
        Value::Null => {
            e.null().map_err(encoded)?;
        }
        Value::Bool(v) => {
            e.bool(*v).map_err(encoded)?;
        }
        Value::Number(v) => {
            if let Some(v) = v.as_u64() {
                e.u64(v).map_err(encoded)?;
            } else if let Some(v) = v.as_i64() {
                e.i64(v).map_err(encoded)?;
            } else {
                return Err(NormalizeError::Wire);
            }
        }
        Value::String(v) => {
            e.str(v).map_err(encoded)?;
        }
        Value::Array(values) => {
            e.array(values.len() as u64).map_err(encoded)?;
            for value in values {
                argument(value, e)?;
            }
        }
        Value::Object(values) => {
            let mut fields = Vec::new();
            fields
                .try_reserve_exact(values.len())
                .map_err(|_| NormalizeError::Allocation)?;
            fields.extend(values.iter());
            fields.sort_unstable_by(|a, b| {
                a.0.len()
                    .cmp(&b.0.len())
                    .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
            });
            e.map(fields.len() as u64).map_err(encoded)?;
            for (key, value) in fields {
                e.str(key).map_err(encoded)?;
                argument(value, e)?;
            }
        }
    }
    Ok(())
}
/// Normalize one complete successful non-streamed response (or externally completed assembly).
/// Caller binds request_id and exact model to its native pending request; response IDs are DATA.
/// Every raw member, including ignored provider extensions, and every argument JSON string
/// preflights before typed conversion. This function neither assembles nor approves partial SSE.
pub fn normalize_chat_completion(
    bytes: &[u8],
    request_id: u64,
    model: &str,
    limits: &Limits,
) -> Result<NormalizedReply, NormalizeError> {
    let bounds = DecodeLimits {
        max_block_bytes: limits.max_model_reply_bytes,
        max_depth: limits.max_depth,
        max_collection_entries: limits.max_collection_entries,
    };
    let body = json::parse(bytes, &bounds)?;
    if string(&body, "model")? != model {
        return Err(NormalizeError::Model);
    }
    if !body.get("error").is_none_or(Value::is_null) {
        return Err(NormalizeError::Incomplete);
    }
    let choices = body
        .get("choices")
        .and_then(Value::as_array)
        .ok_or(NormalizeError::Schema)?;
    let [choice] = choices.as_slice() else {
        return Err(NormalizeError::Schema);
    };
    if choice.get("index").and_then(Value::as_u64) != Some(0) {
        return Err(NormalizeError::Schema);
    }
    let finish = string(choice, "finish_reason")?;
    if !matches!(finish, "stop" | "tool_calls") {
        return Err(NormalizeError::Incomplete);
    }
    let message = choice.get("message").ok_or(NormalizeError::Schema)?;
    if string(message, "role")? != "assistant"
        || !message.get("refusal").is_none_or(Value::is_null)
        || !message.get("function_call").is_none_or(Value::is_null)
    {
        return Err(NormalizeError::Incomplete);
    }
    let calls = match message.get("tool_calls") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(v)) => v.as_slice(),
        _ => return Err(NormalizeError::Schema),
    };
    // Never strip inconsistent calls or turn missing fragments into a final reply.
    if (finish == "tool_calls") == calls.is_empty() {
        return Err(NormalizeError::Incomplete);
    }
    let text = match message.get("content") {
        Some(Value::String(v)) => v.as_str(),
        Some(Value::Null) => "",
        _ => return Err(NormalizeError::Schema),
    };
    let reasoning = match message.get("reasoning_content") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v),
        _ => return Err(NormalizeError::Schema),
    };
    let mut arguments = Vec::new();
    arguments
        .try_reserve_exact(calls.len())
        .map_err(|_| NormalizeError::Allocation)?;
    for call in calls {
        let id = string(call, "id")?;
        if id.is_empty()
            || string(call, "type")? != "function"
            || arguments.iter().any(|(prior, _, _)| *prior == id)
        {
            return Err(NormalizeError::Schema);
        }
        let function = call.get("function").ok_or(NormalizeError::Schema)?;
        let tool = string(function, "name")?;
        if tool.is_empty() {
            return Err(NormalizeError::Schema);
        }
        arguments.push((
            id,
            tool,
            json::parse(string(function, "arguments")?.as_bytes(), &bounds)?,
        ));
    }
    let usage = match body.get("usage") {
        None | Some(Value::Null) => None,
        Some(v) if v.is_object() => Some(v),
        _ => return Err(NormalizeError::Schema),
    };
    let usage = Usage {
        model_calls: 0,
        local_tool_calls: 0,
        input_tokens: optional_count(usage.and_then(|v| v.get("prompt_tokens")))?,
        output_tokens: optional_count(usage.and_then(|v| v.get("completion_tokens")))?,
        billed_units: None,
    };
    let mut e = Encoder::new(Writer {
        bytes: Vec::new(),
        cap: limits.max_model_reply_bytes,
    });
    e.map(4)
        .and_then(|e| e.str("text"))
        .and_then(|e| e.str(text))
        .and_then(|e| e.str("calls"))
        .and_then(|e| e.array(arguments.len() as u64))
        .map_err(encoded)?;
    for (id, tool, args) in arguments {
        e.map(3)
            .and_then(|e| e.str("call_id"))
            .and_then(|e| e.str(id))
            .and_then(|e| e.str("tool_id"))
            .and_then(|e| e.str(tool))
            .and_then(|e| e.str("arguments"))
            .map_err(encoded)?;
        argument(&args, &mut e)?;
    }
    e.str("usage").map_err(encoded)?;
    use minicbor::encode::Write as _;
    e.writer_mut()
        .write_all(&usage.encode(&bounds).map_err(|_| NormalizeError::Wire)?)?;
    e.str("request_id")
        .and_then(|e| e.u64(request_id))
        .map_err(encoded)?;
    let bytes = e.into_writer().bytes;
    ModelReply::decode(&bytes, limits).map_err(|_| NormalizeError::Wire)?;
    let reasoning = reasoning
        .map(|v| {
            let mut owned = String::new();
            owned
                .try_reserve_exact(v.len())
                .map_err(|_| NormalizeError::Allocation)?;
            owned.push_str(v);
            Ok(owned)
        })
        .transpose()?;
    Ok(NormalizedReply { bytes, reasoning })
}
#[cfg(test)]
mod tests;
