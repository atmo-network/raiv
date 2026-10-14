use super::*;
use serde_json::json;
use std::{
    cell::Cell,
    io::{self, Cursor},
};
const KEY: &str = "OFFLINE_SECRET_TOKEN_SENTINEL";
fn config(stream: bool) -> ProviderConfig {
    ProviderConfig {
        protocol: "openai-chat-completions".into(),
        base_url: "https://opencode.ai/zen/go/v1".into(),
        model: "m".into(),
        api_key_env: "TEST_INFERENCE_KEY".into(),
        stream,
        max_tokens: 1024,
        max_request_bytes: 65536,
        max_response_bytes: 65536,
        max_header_bytes: 8192,
        timeout: Duration::from_secs(3),
        stream_limits: StreamLimits {
            max_wire_bytes: 65536,
            max_event_bytes: 4096,
            max_events: 32,
        },
    }
}
fn client(stream: bool) -> ChatCompletionsClient {
    ChatCompletionsClient::with_lookup(config(stream), |name| {
        assert_eq!(name, "TEST_INFERENCE_KEY");
        Some(KEY.into())
    })
    .unwrap()
}
#[test]
fn session_routing_is_native_bounded_once_and_never_secret_or_header_injection() {
    let routed = client(false).with_session_id("raiv-event-0123").unwrap();
    assert_eq!(
        routed.session.as_ref().unwrap().to_str().unwrap(),
        "raiv-event-0123"
    );
    assert!(routed.with_session_id("replacement").is_err());
    for id in [
        "",
        "x\r\nAuthorization: injected",
        "bad space",
        KEY,
        &"a".repeat(129),
    ] {
        assert!(client(false).with_session_id(id).is_err());
    }
    assert!(client(false).session.is_none());
}
fn request() -> Value {
    json!({"model":"m","stream":false,"max_tokens":128,"messages":[{"role":"system","content":"guest policy"},{"role":"user","content":"guest context"}],"n":1})
}
fn reply() -> Vec<u8> {
    serde_json::to_vec(&json!({"model":"m","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]})).unwrap()
}
fn headers(stream: bool) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(if stream {
            "text/event-stream"
        } else {
            "application/json"
        }),
    );
    h
}
#[test]
fn exact_https_resource_aliases_and_bad_configuration_precede_environment_lookup() {
    for (base, want) in [
        ("https://x", "https://x/chat/completions"),
        ("https://x/v1/", "https://x/v1/chat/completions"),
        (
            "https://x/v1/chat/completions/",
            "https://x/v1/chat/completions",
        ),
    ] {
        assert_eq!(endpoint(base).unwrap(), want);
    }
    for base in [
        "http://x",
        "https://user:secret@x/v1",
        "https://x/v1?key=secret",
        "https://x/#fragment",
        "https://x/a/../v1",
        "https://x/\\v1",
        "https://x/\nv1",
        "/v1",
    ] {
        assert!(endpoint(base).is_err());
    }
    for protocol in ["openai-chat-completions", "openai-responses"] {
        let mut cfg = config(false);
        cfg.protocol = protocol.into();
        let client = ChatCompletionsClient::with_lookup(cfg, |_| Some(KEY.into())).unwrap();
        assert_eq!(
            client.endpoint(),
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
    }
    for selector in ["openai-completions", "scripted", "anthropic-messages"] {
        let mut cfg = config(false);
        cfg.protocol = selector.into();
        let called = Cell::new(false);
        assert!(ChatCompletionsClient::with_lookup(cfg, |_| {
            called.set(true);
            Some(KEY.into())
        })
        .is_err());
        assert!(!called.get());
    }
    let mut cfg = config(false);
    cfg.api_key_env = "secret-value-with-dashes".into();
    assert!(ChatCompletionsClient::with_lookup(cfg, |_| panic!("no key lookup")).is_err());
}
#[test]
fn credential_port_is_name_only_missing_invalid_and_debug_never_expose_value() {
    for key in [
        None,
        Some("".into()),
        Some("token\nheader".into()),
        Some(" token ".into()),
        Some("ключ".into()),
        Some("x".repeat(8192)),
    ] {
        assert_eq!(
            ChatCompletionsClient::with_lookup(config(false), |_| key).err(),
            Some(ClientError::Credential)
        );
    }
    let mut cfg = config(false);
    cfg.base_url = format!("https://x/{KEY}");
    assert_eq!(
        ChatCompletionsClient::with_lookup(cfg, |_| Some(KEY.into())).err(),
        Some(ClientError::CredentialEcho)
    );
    let client = client(false);
    assert!(!format!("{:?}", client.authorization).contains(KEY));
    let cfg = client.agent.config();
    assert!(cfg.https_only());
    assert!(cfg.proxy().is_none());
    assert_eq!(cfg.max_redirects(), 0);
    assert_eq!(cfg.max_idle_connections(), 0);
    assert_eq!(cfg.timeouts().global, Some(Duration::from_secs(3)));
    for error in [
        ClientError::Credential,
        ClientError::Transport,
        ClientError::Response,
    ] {
        assert!(!format!("{error:?}: {error}").contains(KEY));
    }
}
#[test]
fn request_model_stream_max_tokens_single_choice_roles_duplicates_and_secret_echo_fail_before_post()
{
    let client = client(false);
    let good = request();
    assert!(client
        .validate_request(&serde_json::to_vec(&good).unwrap(), &Limits::default())
        .is_ok());
    for (key, value) in [
        ("model", json!("foreign")),
        ("stream", json!(true)),
        ("max_tokens", json!(1025)),
        ("max_tokens", json!(0)),
        ("n", json!(2)),
        ("messages", json!([])),
    ] {
        let mut v = good.clone();
        v[key] = value;
        assert!(client
            .validate_request(&serde_json::to_vec(&v).unwrap(), &Limits::default())
            .is_err());
    }
    let mut v = good.clone();
    v["messages"][0]["role"] = json!("developer");
    assert!(client
        .validate_request(&serde_json::to_vec(&v).unwrap(), &Limits::default())
        .is_err());
    v = good;
    v["messages"][0]["content"] = json!(KEY);
    assert_eq!(
        client.validate_request(&serde_json::to_vec(&v).unwrap(), &Limits::default()),
        Err(ClientError::CredentialEcho)
    );
    let duplicate = b"{\"model\":\"m\",\"model\":\"m\"}";
    assert!(client
        .validate_request(duplicate, &Limits::default())
        .is_err());
}
struct NeverRead;
impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("error body must not be consumed")
    }
}
#[test]
fn status_redirect_type_encoding_and_declared_limit_fail_without_error_body_echo() {
    let client = client(false);
    for status in [301, 302, 401, 429, 500] {
        assert_eq!(
            client
                .read_response(1, status, &headers(false), NeverRead, &Limits::default())
                .err(),
            Some(ClientError::Status(status))
        );
        let message = ClientError::Status(status).to_string();
        assert!(message.contains(&status.to_string()));
        assert!(!message.contains(KEY));
    }
    let mut h = headers(false);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
    assert_eq!(
        client
            .read_response(1, 200, &h, NeverRead, &Limits::default())
            .err(),
        Some(ClientError::ContentType)
    );
    h = headers(false);
    h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    assert_eq!(
        client
            .read_response(1, 200, &h, NeverRead, &Limits::default())
            .err(),
        Some(ClientError::ContentType)
    );
    h = headers(false);
    h.append(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    assert_eq!(
        client
            .read_response(1, 200, &h, NeverRead, &Limits::default())
            .err(),
        Some(ClientError::ContentType)
    );
    h = headers(false);
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_static("18446744073709551615"),
    );
    assert_eq!(
        client
            .read_response(1, 200, &h, NeverRead, &Limits::default())
            .err(),
        Some(ClientError::Limit)
    );
    h = headers(false);
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=latin1"),
    );
    assert!(client
        .read_response(1, 200, &h, NeverRead, &Limits::default())
        .is_err());
}
#[test]
fn json_success_full_u64_correlation_and_actual_length_caps_do_not_trust_header_or_model() {
    let client = client(false);
    let raw = reply();
    let mut h = headers(false);
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=\"UTF-8\""),
    );
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&raw.len().to_string()).unwrap(),
    );
    let result = client
        .read_response(u64::MAX, 200, &h, Cursor::new(&raw), &Limits::default())
        .unwrap();
    assert_eq!(
        raiv::activation::ModelReply::decode(result.as_bytes(), &Limits::default())
            .unwrap()
            .request_id(),
        u64::MAX
    );
    h.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&(raw.len() + 1).to_string()).unwrap(),
    );
    assert_eq!(
        client
            .read_response(1, 200, &h, Cursor::new(&raw), &Limits::default())
            .err(),
        Some(ClientError::Transport)
    );
    let mut cfg = config(false);
    cfg.max_response_bytes = raw.len() as u64 - 1;
    cfg.stream_limits.max_wire_bytes = cfg.max_response_bytes;
    cfg.stream_limits.max_event_bytes = 1;
    let small = ChatCompletionsClient::with_lookup(cfg, |_| Some(KEY.into())).unwrap();
    assert_eq!(
        small
            .read_response(
                1,
                200,
                &headers(false),
                Cursor::new(raw),
                &Limits::default()
            )
            .err(),
        Some(ClientError::Limit)
    );
    let raw = b"{\"model\":\"other\",\"error\":{\"message\":\"OFFLINE_SECRET_TOKEN_SENTINEL\"}}";
    let error = client
        .read_response(
            1,
            200,
            &headers(false),
            Cursor::new(raw),
            &Limits::default(),
        )
        .err()
        .unwrap();
    assert_eq!(error, ClientError::Response);
    assert!(!format!("{error:?}: {error}").contains(KEY));
}
fn sse() -> Vec<u8> {
    format!("data: {}\n\ndata: [DONE]\n\n",serde_json::to_string(&json!({"id":"response","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]})).unwrap()).into_bytes()
}
struct FailureAfter {
    body: Cursor<Vec<u8>>,
    kind: io::ErrorKind,
}
impl Read for FailureAfter {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.body.read(out)?;
        if n == 0 {
            Err(io::Error::new(self.kind, KEY))
        } else {
            Ok(n)
        }
    }
}
#[test]
fn sse_done_plus_reader_error_timeout_or_truncated_eof_cannot_advance_partial_reply() {
    let client = client(true);
    let raw = sse();
    let result = client
        .read_response(
            1,
            200,
            &headers(true),
            Cursor::new(raw.clone()),
            &Limits::default(),
        )
        .unwrap();
    assert_eq!(
        raiv::activation::ModelReply::decode(result.as_bytes(), &Limits::default())
            .unwrap()
            .text(),
        "ok"
    );
    for (kind, want) in [
        (io::ErrorKind::TimedOut, ClientError::Timeout),
        (io::ErrorKind::UnexpectedEof, ClientError::Transport),
    ] {
        let error = client
            .read_response(
                1,
                200,
                &headers(true),
                FailureAfter {
                    body: Cursor::new(raw.clone()),
                    kind,
                },
                &Limits::default(),
            )
            .err()
            .unwrap();
        assert_eq!(error, want);
        assert!(!format!("{error:?}: {error}").contains(KEY));
    }
    let partial = raw.split(|b| *b == b'\n').next().unwrap();
    assert_eq!(
        client
            .read_response(
                1,
                200,
                &headers(true),
                Cursor::new(partial),
                &Limits::default()
            )
            .err(),
        Some(ClientError::Stream(StreamError::Incomplete))
    );
}
#[test]
fn sse_failure_categories_survive_transport_without_disclosing_provider_data() {
    use crate::inference::NormalizeError;
    let client = client(true);
    let termination = |reason: &str| {
        String::from_utf8(sse())
            .unwrap()
            .replace(
                "\"finish_reason\":\"stop\"",
                &format!("\"finish_reason\":\"{reason}\""),
            )
            .into_bytes()
    };
    let wrong_model = String::from_utf8(sse())
        .unwrap()
        .replace("\"model\":\"m\"", "\"model\":\"other\"")
        .into_bytes();
    for (raw, expected) in [
        (
            b"data: not-json OFFLINE_SECRET_TOKEN_SENTINEL\n\n".to_vec(),
            StreamError::Normalize(NormalizeError::Json),
        ),
        (b"data: [DONE]\n\n".to_vec(), StreamError::Incomplete),
        (wrong_model, StreamError::Normalize(NormalizeError::Model)),
        (termination("length"), StreamError::Truncated),
        (termination("content_filter"), StreamError::Refused),
    ] {
        let error = client
            .read_response(1, 200, &headers(true), Cursor::new(raw), &Limits::default())
            .err()
            .unwrap();
        assert_eq!(error, ClientError::Stream(expected));
        assert!(!format!("{error:?}: {error}").contains(KEY));
    }
}
#[test]
fn normalized_text_tool_or_reasoning_credential_echo_is_withheld_even_when_json_escaped() {
    let selected = client(false);
    assert!(selected
        .guard_credential_echo(b"safe input/definition DATA")
        .is_ok());
    assert_eq!(
        selected.guard_credential_echo(KEY.as_bytes()),
        Err(ClientError::CredentialEcho)
    );
    for target in ["content", "reasoning_content"] {
        let client = client(false);
        let mut body: Value = serde_json::from_slice(&reply()).unwrap();
        body["choices"][0]["message"][target] = json!(KEY);
        let raw = serde_json::to_string(&body)
            .unwrap()
            .replace("OFFLINE_", "OFFLINE\\u005f");
        assert_eq!(
            client
                .read_response(
                    1,
                    200,
                    &headers(false),
                    Cursor::new(raw),
                    &Limits::default()
                )
                .err(),
            Some(ClientError::CredentialEcho)
        );
    }
}
