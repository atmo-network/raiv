//! Lossy finite diagnostic observations, not provider assertions or raw error disclosure.
use super::{json, json_has_secret};
use raiv::format::DecodeLimits;
use serde_json::Value;
use std::io::Read;
const CAP: usize = 8192;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    ModelUnavailable,
    Authentication,
    Quota,
    UnsupportedParameter,
    InvalidRequest,
    Unknown,
    Withheld,
    Malformed,
    Incomplete,
    Oversize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticParameter {
    Model,
    Thinking,
    ReasoningEffort,
    MaxTokens,
    Stream,
    StreamOptions,
    Tools,
    ToolChoice,
    ParallelToolCalls,
    Messages,
    N,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub status: u16,
    pub kind: DiagnosticKind,
    pub parameter: Option<DiagnosticParameter>,
}
fn parameter(text: &str) -> Option<DiagnosticParameter> {
    use DiagnosticParameter::*;
    match text {
        "model" => Some(Model),
        "thinking" => Some(Thinking),
        "reasoning_effort" => Some(ReasoningEffort),
        "max_tokens" => Some(MaxTokens),
        "stream" => Some(Stream),
        "stream_options" => Some(StreamOptions),
        "tools" => Some(Tools),
        "tool_choice" => Some(ToolChoice),
        "parallel_tool_calls" => Some(ParallelToolCalls),
        "messages" => Some(Messages),
        "n" => Some(N),
        _ => None,
    }
}
pub(super) fn inspect(status: u16, reader: impl Read, key: &[u8]) -> Diagnostic {
    let mut report = Diagnostic {
        status,
        kind: DiagnosticKind::Unknown,
        parameter: None,
    };
    let mut body = Vec::new();
    if body.try_reserve_exact(CAP + 1).is_err() {
        report.kind = DiagnosticKind::Incomplete;
        return report;
    }
    if reader
        .take((CAP + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        report.kind = DiagnosticKind::Incomplete;
        return report;
    }
    if body.len() > CAP {
        report.kind = DiagnosticKind::Oversize;
        return report;
    }
    let bounds = DecodeLimits {
        max_block_bytes: CAP as u64,
        max_depth: 12,
        max_collection_entries: 256,
    };
    let Ok(value) = json::parse(&body, &bounds) else {
        report.kind = DiagnosticKind::Malformed;
        return report;
    };
    if json_has_secret(&value, key) {
        report.kind = DiagnosticKind::Withheld;
        return report;
    }
    let error = value.get("error").unwrap_or(&value);
    report.parameter = error
        .get("param")
        .and_then(Value::as_str)
        .and_then(parameter);
    // Only fixed labels escape this module. Strings are compared, never returned/formatted.
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .unwrap_or("");
    report.kind = match code {
        "model_not_found" | "model_not_available" | "unsupported_model" => {
            DiagnosticKind::ModelUnavailable
        }
        "invalid_api_key" | "authentication_error" | "unauthorized" => {
            DiagnosticKind::Authentication
        }
        "insufficient_quota" | "rate_limit_exceeded" | "rate_limit_error" => DiagnosticKind::Quota,
        "unsupported_parameter" | "unknown_parameter" => DiagnosticKind::UnsupportedParameter,
        "invalid_request_error" | "invalid_request" | "validation_error" => {
            DiagnosticKind::InvalidRequest
        }
        _ => DiagnosticKind::Unknown,
    };
    if report.kind == DiagnosticKind::Unknown || report.kind == DiagnosticKind::InvalidRequest {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        report.kind = if message.contains("model not found")
            || message.contains("model is not supported")
            || message.contains("model not supported")
            || message.contains("model is not available")
        {
            DiagnosticKind::ModelUnavailable
        } else if message.contains("invalid api key")
            || message.contains("invalid api-key")
            || message.contains("api key is invalid")
        {
            DiagnosticKind::Authentication
        } else if message.contains("quota exceeded")
            || message.contains("rate limit exceeded")
            || message.contains("insufficient balance")
        {
            DiagnosticKind::Quota
        } else if message.contains("unsupported parameter")
            || message.contains("unknown parameter")
            || message.contains("unrecognized parameter")
        {
            DiagnosticKind::UnsupportedParameter
        } else {
            report.kind
        };
        if report.parameter.is_none() {
            // Require quoted token, not substring matches inside provider/user-controlled prose.
            for name in [
                "model",
                "thinking",
                "reasoning_effort",
                "max_tokens",
                "stream",
                "stream_options",
                "tools",
                "tool_choice",
                "parallel_tool_calls",
                "messages",
                "n",
            ] {
                if ['\'', '"', '`']
                    .iter()
                    .any(|quote| message.contains(&format!("{quote}{name}{quote}")))
                {
                    if report.parameter.is_some() {
                        report.parameter = None;
                        break;
                    }
                    report.parameter = parameter(name);
                }
            }
        }
    }
    report
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_labels_only_for_escaped_messages_and_arbitrary_fields_never_disclose_provider_data() {
        let secret = b"SECRET_SENTINEL";
        for (raw, kind, param) in [
            (
                r#"{"error":{"code":"model_not_found","message":"private-account-id"}}"#,
                DiagnosticKind::ModelUnavailable,
                None,
            ),
            (
                r#"{"error":{"message":"Unsupported parameter: 'thinking'. Private request ID abc"}}"#,
                DiagnosticKind::UnsupportedParameter,
                Some(DiagnosticParameter::Thinking),
            ),
            (
                r#"{"error":{"type":"invalid_request_error","param":"tools","message":"private schema details"}}"#,
                DiagnosticKind::InvalidRequest,
                Some(DiagnosticParameter::Tools),
            ),
            (
                r#"{"error":{"code":"unknown-secret-code","param":"private-secret-field","message":"private-account-id"}}"#,
                DiagnosticKind::Unknown,
                None,
            ),
            (
                r#"{"error":{"message":"Invalid API key: SECRET\u005fSENTINEL"}}"#,
                DiagnosticKind::Withheld,
                None,
            ),
        ] {
            let report = inspect(400, raw.as_bytes(), secret);
            assert_eq!(report.kind, kind);
            assert_eq!(report.parameter, param);
            let output = format!("{report:?}");
            for forbidden in ["SECRET", "private", "abc", "schema details"] {
                assert!(!output.contains(forbidden));
            }
        }
    }
    #[test]
    fn duplicates_bounds_truncation_and_stream_read_failure_never_salvage_a_diagnostic() {
        for raw in [
            br#"{"error":{"code":"model_not_found","code":"invalid_api_key"}}"#.as_slice(),
            br#"{"error":"unfinished"#,
            b"not-json",
        ] {
            assert_eq!(inspect(400, raw, b"secret").kind, DiagnosticKind::Malformed);
        }
        assert_eq!(
            inspect(400, vec![b'x'; CAP + 2].as_slice(), b"secret").kind,
            DiagnosticKind::Oversize
        );
        struct Fault;
        impl Read for Fault {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("private body secret"))
            }
        }
        assert_eq!(
            inspect(400, Fault, b"secret").kind,
            DiagnosticKind::Incomplete
        );
        let raw = serde_json::json!({"error":{"message":"bad 'thinking' and 'tools' fields"}});
        let report = inspect(400, serde_json::to_vec(&raw).unwrap().as_slice(), b"secret");
        assert_eq!(report.parameter, None);
    }
}
