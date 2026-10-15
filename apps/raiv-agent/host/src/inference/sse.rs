//! Owns bounded incremental Chat Completions SSE DATA, not HTTP success, credentials or guest advancement.

use super::{json, normalize_chat_completion, NormalizeError, NormalizedReply, Writer};
use raiv::{format::DecodeLimits, limits::Limits};
use serde_json::{json, Value};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamError {
    Frame,
    Limit,
    Incomplete,
    Truncated,
    Refused,
    Failed,
    Allocation,
    Normalize(NormalizeError),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Frame => "provider SSE framing/delta schema invalid",
            Self::Limit => "provider SSE wire/event/assembly bound",
            Self::Incomplete => "provider SSE terminal/DONE/EOF incomplete",
            Self::Truncated => "provider SSE declares finish_reason=length; no complete reply",
            Self::Refused => "provider SSE refusal/filter marker; no complete reply",
            Self::Failed => "provider SSE already failed; no partial reply/reuse",
            Self::Allocation => "provider SSE allocation failed",
            Self::Normalize(_) => "provider SSE complete normalization failed",
        })
    }
}

impl std::error::Error for StreamError {}

impl From<NormalizeError> for StreamError {
    fn from(v: NormalizeError) -> Self {
        Self::Normalize(v)
    }
}

/// Explicit native transport bounds; bytes include ignored metadata/comments and every chunk.
#[derive(Clone, Copy)]
pub struct StreamLimits {
    pub max_wire_bytes: u64,
    pub max_event_bytes: u64,
    pub max_events: u64,
}

struct Call {
    id: String,
    name: String,
    arguments: String,
}

/// Incremental DATA assembler for one native pending request. No normalized reply exists until
/// consuming finish after successful HTTP EOF. Errors poison it; preview text is provisional only.
pub struct ChatCompletionStream {
    request_id: u64,
    model: String,
    limits: Limits,
    bounds: StreamLimits,
    wire: u64,
    event_bytes: u64,
    events: u64,
    stored: u64,
    line: Vec<u8>,
    data: Vec<u8>,
    has_data: bool,
    event_name: String,
    pending_cr: bool,
    first_line: bool,
    failed: bool,
    done: bool,
    response_id: Option<String>,
    role: bool,
    text: String,
    reasoning: String,
    has_reasoning: bool,
    calls: Vec<Call>,
    terminal: Option<String>,
    usage: Option<Value>,
}

impl ChatCompletionStream {
    pub fn new(
        request_id: u64,
        model: &str,
        limits: Limits,
        bounds: StreamLimits,
    ) -> Result<Self, StreamError> {
        if model.is_empty()
            || limits.max_depth == 0
            || limits.max_depth > 64
            || bounds.max_wire_bytes == 0
            || bounds.max_event_bytes == 0
            || bounds.max_event_bytes > bounds.max_wire_bytes
            || bounds.max_events == 0
        {
            return Err(StreamError::Limit);
        }
        let mut stream = Self {
            request_id,
            model: String::new(),
            limits,
            bounds,
            wire: 0,
            event_bytes: 0,
            events: 0,
            stored: 0,
            line: Vec::new(),
            data: Vec::new(),
            has_data: false,
            event_name: String::new(),
            pending_cr: false,
            first_line: true,
            failed: false,
            done: false,
            response_id: None,
            role: false,
            text: String::new(),
            reasoning: String::new(),
            has_reasoning: false,
            calls: Vec::new(),
            terminal: None,
            usage: None,
        };
        append(
            &mut stream.model,
            model,
            &mut stream.stored,
            limits.max_model_reply_bytes,
        )?;
        Ok(stream)
    }

    pub fn provisional_text(&self) -> &str {
        &self.text
    }

    /// Feed arbitrary byte splits, including within CRLF, UTF-8 and JSON strings.
    /// Neither return value nor provisional_text is successful inference DATA.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if self.failed {
            return Err(StreamError::Failed);
        }
        let result = self.feed(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        self.wire = self
            .wire
            .checked_add(bytes.len() as u64)
            .ok_or(StreamError::Limit)?;
        if self.wire > self.bounds.max_wire_bytes {
            return Err(StreamError::Limit);
        }
        for &byte in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    self.event_byte()?;
                    self.line_end()?;
                    continue;
                }
                self.line_end()?;
            }
            self.event_byte()?;
            if byte == b'\r' {
                self.pending_cr = true;
            } else if byte == b'\n' {
                self.line_end()?;
            } else {
                self.line
                    .try_reserve(1)
                    .map_err(|_| StreamError::Allocation)?;
                self.line.push(byte);
            }
        }
        Ok(())
    }

    fn event_byte(&mut self) -> Result<(), StreamError> {
        self.event_bytes = self.event_bytes.checked_add(1).ok_or(StreamError::Limit)?;
        if self.event_bytes > self.bounds.max_event_bytes {
            return Err(StreamError::Limit);
        }
        Ok(())
    }

    fn line_end(&mut self) -> Result<(), StreamError> {
        let raw = std::mem::take(&mut self.line);
        let raw = if self.first_line {
            self.first_line = false;
            raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&raw)
        } else {
            &raw
        };
        let line = std::str::from_utf8(raw).map_err(|_| StreamError::Frame)?;
        if line.is_empty() {
            if self.has_data {
                if !self.event_name.is_empty() && self.event_name != "message" {
                    return Err(StreamError::Frame);
                }
                let data = std::mem::take(&mut self.data);
                self.event(&data)?;
            }
            self.event_name.clear();
            self.has_data = false;
            self.event_bytes = 0;
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self.done {
                    return Err(StreamError::Frame);
                }
                let needed = self
                    .data
                    .len()
                    .checked_add(value.len())
                    .and_then(|n| n.checked_add(usize::from(self.has_data)))
                    .ok_or(StreamError::Limit)?;
                if needed as u64 > self.bounds.max_event_bytes {
                    return Err(StreamError::Limit);
                }
                self.data
                    .try_reserve(value.len() + usize::from(self.has_data))
                    .map_err(|_| StreamError::Allocation)?;
                if self.has_data {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value.as_bytes());
                self.has_data = true;
            }
            "event" => {
                self.event_name.clear();
                self.event_name
                    .try_reserve_exact(value.len())
                    .map_err(|_| StreamError::Allocation)?;
                self.event_name.push_str(value);
            }
            _ => {} // id/retry/unknown fields are bounded DATA, never native correlation.
        }
        Ok(())
    }

    fn event(&mut self, data: &[u8]) -> Result<(), StreamError> {
        self.events = self.events.checked_add(1).ok_or(StreamError::Limit)?;
        if self.events > self.bounds.max_events {
            return Err(StreamError::Limit);
        }
        if data == b"[DONE]" {
            if self.done || self.terminal.is_none() {
                return Err(StreamError::Incomplete);
            }
            self.done = true;
            return Ok(());
        }
        if self.done {
            return Err(StreamError::Frame);
        }
        let value = json::parse(
            data,
            &DecodeLimits {
                max_block_bytes: self.bounds.max_event_bytes,
                max_depth: self.limits.max_depth,
                max_collection_entries: self.limits.max_collection_entries,
            },
        )?;
        if !value
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|reported| super::model_matches(&self.model, reported))
        {
            return Err(StreamError::Normalize(NormalizeError::Model));
        }
        if !value.get("error").is_none_or(Value::is_null) {
            return Err(StreamError::Incomplete);
        }
        if let Some(id) = part(&value, "id")? {
            if id.is_empty() {
                return Err(StreamError::Frame);
            }
            if let Some(prior) = &self.response_id {
                if prior != id {
                    return Err(StreamError::Frame);
                }
            } else {
                let mut owned = String::new();
                append(
                    &mut owned,
                    id,
                    &mut self.stored,
                    self.limits.max_model_reply_bytes,
                )?;
                self.response_id = Some(owned);
            }
        }
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(StreamError::Frame)?;
        if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
            if !usage.is_object() {
                return Err(StreamError::Frame);
            }
            if let Some(prior) = &self.usage {
                if prior != usage {
                    return Err(StreamError::Frame);
                }
            } else {
                let bytes = bounded_json(usage, self.limits.max_model_reply_bytes)?;
                charge(
                    &mut self.stored,
                    bytes.len(),
                    self.limits.max_model_reply_bytes,
                )?;
                self.usage = Some(usage.clone());
            }
        }
        if choices.is_empty() {
            if self.terminal.is_none() || value.get("usage").is_none_or(Value::is_null) {
                return Err(StreamError::Frame);
            }
            return Ok(());
        }
        let [choice] = choices.as_slice() else {
            return Err(StreamError::Frame);
        };
        if self.terminal.is_some() || choice.get("index").and_then(Value::as_u64) != Some(0) {
            return Err(StreamError::Frame);
        }
        let delta = choice
            .get("delta")
            .filter(|v| v.is_object())
            .ok_or(StreamError::Frame)?;
        if !delta.get("refusal").is_none_or(Value::is_null) {
            return Err(StreamError::Refused);
        }
        if !delta.get("function_call").is_none_or(Value::is_null) {
            return Err(StreamError::Incomplete);
        }
        if let Some(role) = part(delta, "role")? {
            if role != "assistant" {
                return Err(StreamError::Frame);
            }
            self.role = true;
        }
        if let Some(content) = part(delta, "content")? {
            append(
                &mut self.text,
                content,
                &mut self.stored,
                self.limits.max_model_reply_bytes,
            )?;
        }
        if let Some(content) = part(delta, "reasoning_content")? {
            self.has_reasoning = true;
            append(
                &mut self.reasoning,
                content,
                &mut self.stored,
                self.limits.max_model_reply_bytes,
            )?;
        }
        if let Some(calls) = delta.get("tool_calls").filter(|v| !v.is_null()) {
            for value in calls.as_array().ok_or(StreamError::Frame)? {
                let index = value
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or(StreamError::Frame)?;
                if index > self.calls.len() as u64 || index >= self.limits.max_collection_entries {
                    return Err(StreamError::Frame);
                }
                let index = usize::try_from(index).map_err(|_| StreamError::Limit)?;
                if index == self.calls.len() {
                    if part(value, "type")? != Some("function") {
                        return Err(StreamError::Frame);
                    }
                    let id = part(value, "id")?
                        .filter(|id| !id.is_empty())
                        .ok_or(StreamError::Frame)?;
                    let mut call = Call {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    };
                    append(
                        &mut call.id,
                        id,
                        &mut self.stored,
                        self.limits.max_model_reply_bytes,
                    )?;
                    self.calls
                        .try_reserve(1)
                        .map_err(|_| StreamError::Allocation)?;
                    self.calls.push(call);
                }
                let call = &mut self.calls[index];
                // Continuation chunks may repeat an empty id (common in OpenAI-compatible servers);
                // only a nonempty different id would change the call identity.
                if part(value, "id")?.is_some_and(|id| !id.is_empty() && id != call.id)
                    || part(value, "type")?.is_some_and(|v| v != "function")
                {
                    return Err(StreamError::Frame);
                }
                if let Some(function) = value.get("function").filter(|v| !v.is_null()) {
                    if !function.is_object() {
                        return Err(StreamError::Frame);
                    }
                    if let Some(name) = part(function, "name")? {
                        append(
                            &mut call.name,
                            name,
                            &mut self.stored,
                            self.limits.max_model_reply_bytes,
                        )?;
                    }
                    if let Some(args) = part(function, "arguments")? {
                        append(
                            &mut call.arguments,
                            args,
                            &mut self.stored,
                            self.limits.max_model_reply_bytes,
                        )?;
                    }
                }
            }
        }
        match choice.get("finish_reason") {
            None | Some(Value::Null) => {}
            Some(Value::String(reason)) if matches!(reason.as_str(), "stop" | "tool_calls") => {
                if !self.role {
                    return Err(StreamError::Incomplete);
                }
                self.terminal = Some(reason.clone());
            }
            Some(Value::String(reason)) if reason == "length" => {
                return Err(StreamError::Truncated)
            }
            Some(Value::String(reason)) if reason == "content_filter" => {
                return Err(StreamError::Refused)
            }
            _ => return Err(StreamError::Incomplete),
        }
        Ok(())
    }

    /// Call only after successful HTTP EOF, never upon terminal chunk/DONE alone.
    /// An unfinished trailing event, interruption or poisoned stream cannot return a reply.
    pub fn finish(mut self) -> Result<NormalizedReply, StreamError> {
        if self.failed {
            return Err(StreamError::Failed);
        }
        if self.pending_cr {
            self.line_end()?;
        }
        if !self.done || !self.line.is_empty() || self.has_data || self.event_bytes != 0 {
            return Err(StreamError::Incomplete);
        }
        let calls:Vec<_>=self.calls.into_iter().map(|call|json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments}})).collect();
        let mut message = json!({"role":"assistant","content":self.text,"tool_calls":calls});
        if self.has_reasoning {
            message["reasoning_content"] = Value::String(self.reasoning);
        }
        let body = json!({"model":self.model,"choices":[{"index":0,"finish_reason":self.terminal,"message":message}],"usage":self.usage});
        let bytes = bounded_json(&body, self.limits.max_model_reply_bytes)?;
        normalize_chat_completion(
            &bytes,
            self.request_id,
            body["model"].as_str().ok_or(StreamError::Frame)?,
            &self.limits,
        )
        .map_err(Into::into)
    }
}

fn part<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, StreamError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(StreamError::Frame),
    }
}

fn charge(stored: &mut u64, len: usize, cap: u64) -> Result<(), StreamError> {
    *stored = stored.checked_add(len as u64).ok_or(StreamError::Limit)?;
    if *stored > cap {
        return Err(StreamError::Limit);
    }
    Ok(())
}

fn append(target: &mut String, value: &str, stored: &mut u64, cap: u64) -> Result<(), StreamError> {
    charge(stored, value.len(), cap)?;
    target
        .try_reserve(value.len())
        .map_err(|_| StreamError::Allocation)?;
    target.push_str(value);
    Ok(())
}

impl std::io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        minicbor::encode::Write::write_all(self, bytes)
            .map_err(|_| std::io::Error::other("bounded provider JSON"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bounded_json(value: &Value, cap: u64) -> Result<Vec<u8>, StreamError> {
    let mut writer = Writer {
        bytes: Vec::new(),
        cap,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| StreamError::Limit)?;
    Ok(writer.bytes)
}

#[cfg(test)]
mod tests;
