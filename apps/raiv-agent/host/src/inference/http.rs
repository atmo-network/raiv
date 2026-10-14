//! Native bounded HTTPS Chat Completions transport. No context construction, guest decisions or admission.
mod diagnostic;
use super::{
    json, normalize_chat_completion,
    sse::{ChatCompletionStream, StreamError, StreamLimits},
    NormalizedReply,
};
pub use diagnostic::{Diagnostic, DiagnosticKind, DiagnosticParameter};
use raiv::{format::DecodeLimits, limits::Limits};
use serde_json::Value;
use std::{fmt, io::Read, time::Duration};
use ureq::http::{header, HeaderMap, HeaderValue, Uri};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientError {
    Configuration,
    Credential,
    Request,
    Transport,
    Timeout,
    Status(u16),
    ContentType,
    Limit,
    Response,
    Stream(StreamError),
    CredentialEcho,
    Allocation,
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Configuration => "inference protocol/HTTPS endpoint/model/bounds invalid",
            Self::Credential => "selected inference environment credential missing or invalid",
            Self::Request => "inference request differs from selected model/stream/output policy",
            Self::Transport => "inference transport failed; no retry or successful partial reply",
            Self::Timeout => "inference transport timeout; no retry or successful partial reply",
            Self::Status(code) => {
                return write!(
                    f,
                    "inference HTTP status {code} refused; provider error body withheld"
                )
            }
            Self::ContentType => "inference HTTP content type/encoding refused",
            Self::Limit => "inference HTTP header/request/response bound",
            Self::Response => "inference complete response normalization failed",
            Self::Stream(StreamError::Normalize(error)) => {
                return write!(f, "inference SSE normalization refused: {error}")
            }
            Self::Stream(error) => return write!(f, "inference SSE refused: {error}"),
            Self::CredentialEcho => "inference credential echo withheld",
            Self::Allocation => "inference transport allocation failed",
        };
        f.write_str(message)
    }
}
impl std::error::Error for ClientError {}
/// Explicit native policy, not model-provided endpoint/credential discovery. No credential value here.
#[derive(Clone)]
pub struct ProviderConfig {
    pub protocol: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    pub stream: bool,
    pub max_tokens: u64,
    pub max_request_bytes: u64,
    pub max_response_bytes: u64,
    pub max_header_bytes: usize,
    pub timeout: Duration,
    pub stream_limits: StreamLimits,
}
impl ProviderConfig {
    /// Validates non-secret configuration DATA only; no environment/TLS/request access.
    /// The returned endpoint is selection DATA, never guest/run/admission authority.
    pub fn validate(&self) -> Result<String, ClientError> {
        if !matches!(
            self.protocol.as_str(),
            "openai-chat-completions" | "openai-responses"
        ) || self.model.is_empty()
            || self.model.chars().any(char::is_control)
            || !env_name(&self.api_key_env)
            || self.max_tokens == 0
            || self.max_request_bytes == 0
            || self.max_response_bytes == 0
            || self.max_header_bytes == 0
            || self.timeout.is_zero()
            || self.stream_limits.max_events == 0
            || self.stream_limits.max_event_bytes == 0
            || self.stream_limits.max_event_bytes > self.stream_limits.max_wire_bytes
            || self.stream_limits.max_wire_bytes > self.max_response_bytes
        {
            return Err(ClientError::Configuration);
        }
        endpoint(&self.base_url)
    }
}
pub struct ChatCompletionsClient {
    agent: ureq::Agent,
    endpoint: String,
    config: ProviderConfig,
    authorization: HeaderValue,
    session: Option<HeaderValue>,
}
impl ChatCompletionsClient {
    pub fn new(config: ProviderConfig) -> Result<Self, ClientError> {
        Self::with_lookup(config, |name| std::env::var(name).ok())
    }
    /// Testable native environment-name lookup port. Not provider/user authority or a raw-key config.
    pub fn with_lookup(
        config: ProviderConfig,
        lookup: impl FnOnce(&str) -> Option<String>,
    ) -> Result<Self, ClientError> {
        let endpoint = config.validate()?;
        let key = lookup(&config.api_key_env).ok_or(ClientError::Credential)?;
        if key.is_empty()
            || !key.bytes().all(|b| (33..=126).contains(&b))
            || key
                .len()
                .checked_add(7)
                .is_none_or(|len| len > config.max_header_bytes)
        {
            return Err(ClientError::Credential);
        }
        if contains(endpoint.as_bytes(), key.as_bytes())
            || contains(config.model.as_bytes(), key.as_bytes())
        {
            return Err(ClientError::CredentialEcho);
        }
        let mut value = String::new();
        value
            .try_reserve_exact(key.len() + 7)
            .map_err(|_| ClientError::Allocation)?;
        value.push_str("Bearer ");
        value.push_str(&key);
        let mut authorization =
            HeaderValue::from_str(&value).map_err(|_| ClientError::Credential)?;
        authorization.set_sensitive(true);
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            .max_response_header_size(config.max_header_bytes)
            .timeout_global(Some(config.timeout))
            .timeout_resolve(Some(config.timeout))
            .timeout_connect(Some(config.timeout))
            .timeout_recv_body(Some(config.timeout))
            .build()
            .new_agent();
        Ok(Self {
            agent,
            endpoint,
            config,
            authorization,
            session: None,
        })
    }
    /// Selected non-secret transport policy only; no credential value/capability is exposed.
    pub fn configuration(&self) -> &ProviderConfig {
        &self.config
    }
    /// Refuses copying known selected credential bytes into native input/Definition/guest DATA.
    /// No credential value is exposed and success does not grant any authority.
    pub fn guard_credential_echo(&self, data: &[u8]) -> Result<(), ClientError> {
        if contains(data, self.key()) {
            Err(ClientError::CredentialEcho)
        } else {
            Ok(())
        }
    }
    /// Native per-Activation routing label, not provider authority or RequestTicket replacement.
    /// Bound once before exposure; never sourced from a model response.
    pub fn with_session_id(mut self, id: &str) -> Result<Self, ClientError> {
        if self.session.is_some()
            || id.is_empty()
            || id.len() > 128
            || id.len() > self.config.max_header_bytes
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ClientError::Configuration);
        }
        self.guard_credential_echo(id.as_bytes())?;
        self.session = Some(HeaderValue::from_str(id).map_err(|_| ClientError::Configuration)?);
        Ok(self)
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    /// Caller owns authorized Definition/Run/RequestTicket binding and must persist attempt before exposure.
    /// Payload is already guest-owned request content mapped by the native adapter, never rebuilt here.
    /// This function issues one POST; redirects, ambient proxies, pooling/replay and retries are disabled.
    pub fn complete(
        &self,
        request_id: u64,
        payload: &[u8],
        limits: &Limits,
    ) -> Result<NormalizedReply, ClientError> {
        self.complete_inner(request_id, payload, limits, None)
    }
    /// Explicit native diagnostic opt-in. Refused bodies are bounded RAM-only DATA;
    /// callback receives finite static classifications, never provider strings or authority.
    /// One POST, same no-retry/credential/normalization gates as complete().
    pub fn complete_diagnostic(
        &self,
        request_id: u64,
        payload: &[u8],
        limits: &Limits,
        report: &mut dyn FnMut(Diagnostic),
    ) -> Result<NormalizedReply, ClientError> {
        self.complete_inner(request_id, payload, limits, Some(report))
    }
    fn complete_inner(
        &self,
        request_id: u64,
        payload: &[u8],
        limits: &Limits,
        report: Option<&mut dyn FnMut(Diagnostic)>,
    ) -> Result<NormalizedReply, ClientError> {
        self.validate_request(payload, limits)?;
        let mut request = self
            .agent
            .post(&self.endpoint)
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::ACCEPT,
                if self.config.stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .header(header::ACCEPT_ENCODING, "identity");
        if let Some(session) = &self.session {
            request = request.header("x-opencode-session", session.clone());
        }
        let response = request.send(payload).map_err(|error| match error {
            ureq::Error::Timeout(_) => ClientError::Timeout,
            _ => ClientError::Transport,
        })?;
        let (parts, body) = response.into_parts();
        if let Some(report) = report {
            if !parts.status.is_success() {
                report(diagnostic::inspect(
                    parts.status.as_u16(),
                    body.into_reader(),
                    self.key(),
                ));
                return Err(ClientError::Status(parts.status.as_u16()));
            }
        }
        self.read_response(
            request_id,
            parts.status.as_u16(),
            &parts.headers,
            body.into_reader(),
            limits,
        )
    }
    fn validate_request(&self, payload: &[u8], limits: &Limits) -> Result<(), ClientError> {
        let value = json::parse(
            payload,
            &DecodeLimits {
                max_block_bytes: self.config.max_request_bytes,
                max_depth: limits.max_depth,
                max_collection_entries: limits.max_collection_entries,
            },
        )
        .map_err(|_| ClientError::Request)?;
        if value.get("model").and_then(Value::as_str) != Some(&self.config.model)
            || value.get("stream").and_then(Value::as_bool) != Some(self.config.stream)
            || value
                .get("max_tokens")
                .and_then(Value::as_u64)
                .is_none_or(|n| n == 0 || n > self.config.max_tokens)
            || value.get("n").is_some_and(|n| n.as_u64() != Some(1))
        {
            return Err(ClientError::Request);
        }
        let messages = value
            .get("messages")
            .and_then(Value::as_array)
            .filter(|m| !m.is_empty())
            .ok_or(ClientError::Request)?;
        for message in messages {
            if !matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "user" | "assistant" | "tool")
            ) {
                return Err(ClientError::Request);
            }
            if message
                .get("content")
                .is_some_and(|v| !v.is_string() && !v.is_null())
            {
                return Err(ClientError::Request);
            }
        }
        if json_has_secret(&value, self.key()) {
            return Err(ClientError::CredentialEcho);
        }
        Ok(())
    }
    fn key(&self) -> &[u8] {
        &self.authorization.as_bytes()[7..]
    }
    // Shared with the offline transport port. Successful EOF is observed by Read, not guessed at DONE.
    fn read_response(
        &self,
        request_id: u64,
        status: u16,
        headers: &HeaderMap,
        mut reader: impl Read,
        limits: &Limits,
    ) -> Result<NormalizedReply, ClientError> {
        if !(200..300).contains(&status) {
            return Err(ClientError::Status(status));
        }
        if headers.get_all(header::CONTENT_TYPE).iter().count() != 1 {
            return Err(ClientError::ContentType);
        }
        let content = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .ok_or(ClientError::ContentType)?;
        let mut parts = content.split(';');
        let media = parts.next().unwrap().trim();
        if !media.eq_ignore_ascii_case(if self.config.stream {
            "text/event-stream"
        } else {
            "application/json"
        }) {
            return Err(ClientError::ContentType);
        }
        for part in parts {
            if let Some((name, value)) = part.trim().split_once('=') {
                if name.trim().eq_ignore_ascii_case("charset")
                    && !value.trim().trim_matches('"').eq_ignore_ascii_case("utf-8")
                {
                    return Err(ClientError::ContentType);
                }
            }
        }
        if headers.get_all(header::CONTENT_ENCODING).iter().count() > 1
            || headers
                .get(header::CONTENT_ENCODING)
                .is_some_and(|v| v.as_bytes() != b"identity")
        {
            return Err(ClientError::ContentType);
        }
        if headers.get_all(header::CONTENT_LENGTH).iter().count() > 1 {
            return Err(ClientError::Limit);
        }
        let expected = match headers.get(header::CONTENT_LENGTH) {
            Some(v) => Some(
                v.to_str()
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or(ClientError::Limit)?,
            ),
            None => None,
        };
        if expected.is_some_and(|n| n > self.config.max_response_bytes) {
            return Err(ClientError::Limit);
        }
        let mut total = 0u64;
        let mut bytes = Vec::new();
        let mut stream = if self.config.stream {
            Some(
                ChatCompletionStream::new(
                    request_id,
                    &self.config.model,
                    *limits,
                    self.config.stream_limits,
                )
                .map_err(ClientError::Stream)?,
            )
        } else {
            None
        };
        let mut buffer = [0u8; 4096];
        loop {
            let n = reader.read(&mut buffer).map_err(|error| {
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) {
                    ClientError::Timeout
                } else {
                    ClientError::Transport
                }
            })?;
            if n == 0 {
                break;
            }
            total = total.checked_add(n as u64).ok_or(ClientError::Limit)?;
            if total > self.config.max_response_bytes {
                return Err(ClientError::Limit);
            }
            if let Some(stream) = &mut stream {
                stream.push(&buffer[..n]).map_err(ClientError::Stream)?;
            } else {
                bytes.try_reserve(n).map_err(|_| ClientError::Allocation)?;
                bytes.extend_from_slice(&buffer[..n]);
            }
        }
        if expected.is_some_and(|len| len != total) {
            return Err(ClientError::Transport);
        }
        let reply = if let Some(stream) = stream {
            stream.finish().map_err(ClientError::Stream)?
        } else {
            normalize_chat_completion(&bytes, request_id, &self.config.model, limits)
                .map_err(|_| ClientError::Response)?
        };
        if contains(reply.as_bytes(), self.key())
            || reply
                .reasoning_content()
                .is_some_and(|text| contains(text.as_bytes(), self.key()))
        {
            return Err(ClientError::CredentialEcho);
        }
        Ok(reply)
    }
}
fn env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn endpoint(base: &str) -> Result<String, ClientError> {
    if base
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b < 32 || matches!(b, b'?' | b'#' | b'\\'))
    {
        return Err(ClientError::Configuration);
    }
    let base = base.trim_end_matches('/');
    let uri: Uri = base.parse().map_err(|_| ClientError::Configuration)?;
    if uri.scheme_str() != Some("https")
        || uri.host().is_none_or(str::is_empty)
        || uri.authority().is_none_or(|a| a.as_str().contains('@'))
        || uri.path().split('/').any(|p| p == "." || p == "..")
    {
        return Err(ClientError::Configuration);
    }
    let mut value = String::new();
    value
        .try_reserve_exact(
            base.len()
                .checked_add(17)
                .ok_or(ClientError::Configuration)?,
        )
        .map_err(|_| ClientError::Allocation)?;
    value.push_str(base);
    if !base.ends_with("/chat/completions") {
        value.push_str("/chat/completions");
    }
    Ok(value)
}
fn contains(bytes: &[u8], key: &[u8]) -> bool {
    bytes.windows(key.len()).any(|part| part == key)
}
fn json_has_secret(value: &Value, key: &[u8]) -> bool {
    match value {
        Value::String(text) => contains(text.as_bytes(), key),
        Value::Array(values) => values.iter().any(|v| json_has_secret(v, key)),
        Value::Object(values) => values
            .iter()
            .any(|(name, v)| contains(name.as_bytes(), key) || json_has_secret(v, key)),
        _ => false,
    }
}
#[cfg(test)]
mod tests;
