use super::*;
use raiv::activation::{ModelReply, ReplyError};
fn caps() -> StreamLimits {
    StreamLimits {
        max_wire_bytes: 65536,
        max_event_bytes: 4096,
        max_events: 64,
    }
}
fn assembler() -> ChatCompletionStream {
    ChatCompletionStream::new(u64::MAX, "m", Limits::default(), caps()).unwrap()
}
fn chunk(delta: Value, finish: Value) -> Value {
    json!({"id":"response-1","model":"m","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}
fn event(value: &Value) -> Vec<u8> {
    format!("data: {}\r\n\r\n", serde_json::to_string(value).unwrap()).into_bytes()
}
fn trace() -> Vec<Vec<u8>> {
    vec![
        event(&chunk(
            json!({"role":"assistant","content":"Привет ","reasoning_content":"r1","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"patch_","arguments":"{"}}]}),
            Value::Null,
        )),
        event(&chunk(
            json!({"tool_calls":[{"index":0,"function":{"name":"memory","arguments":"\"goals\":[null],\"intents\":{\"keep\":false}}"}}]}),
            Value::Null,
        )),
        event(&chunk(
            json!({"content":"мир🌍","reasoning_content":"r2"}),
            json!("tool_calls"),
        )),
        event(
            &json!({"id":"response-1","model":"m","choices":[],"usage":{"prompt_tokens":u64::MAX,"completion_tokens":0}}),
        ),
        b"data: [DONE]\r\n\r\n".to_vec(),
    ]
}
fn complete() -> NormalizedReply {
    normalize_chat_completion(&serde_json::to_vec(&json!({"model":"m","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":"Привет мир🌍","reasoning_content":"r1r2","tool_calls":[{"id":"call-1","type":"function","function":{"name":"patch_memory","arguments":"{\"goals\":[null],\"intents\":{\"keep\":false}}"}}]}}],"usage":{"prompt_tokens":u64::MAX,"completion_tokens":0}})).unwrap(),u64::MAX,"m",&Limits::default()).unwrap()
}
#[test]
fn all_byte_splits_utf8_crlf_tool_fragments_and_usage_tail_equal_complete_normalization() {
    let wire = trace().concat();
    let expected = complete();
    for split in 0..=wire.len() {
        let mut stream = assembler();
        stream.push(&wire[..split]).unwrap();
        stream.push(&wire[split..]).unwrap();
        let result = stream.finish().unwrap();
        assert_eq!(result.as_bytes(), expected.as_bytes());
        assert_eq!(result.reasoning_content(), Some("r1r2"));
    }
    let mut stream = assembler();
    for byte in wire {
        stream.push(&[byte]).unwrap();
    }
    let result = stream.finish().unwrap();
    assert_eq!(result.as_bytes(), expected.as_bytes());
    let reply = ModelReply::decode(result.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(reply.request_id(), u64::MAX);
    assert_eq!(reply.usage().input_tokens, Some(u64::MAX));
    assert_eq!(reply.usage().output_tokens, Some(0));
    assert!(reply.control_call().unwrap().is_some());
}
#[test]
fn lf_cr_bom_comments_metadata_and_multiline_data_frame_without_selecting_remote_ids() {
    for ending in ["\n", "\r", "\r\n"] {
        let value = serde_json::to_string(&chunk(
            json!({"role":"assistant","content":"ok"}),
            json!("stop"),
        ))
        .unwrap();
        let value = value.replacen(",", &format!(",{ending}data: "), 1);
        let wire=format!("\u{feff}: heartbeat{ending}id: foreign{ending}retry: 1{ending}unknown: ignored{ending}event: message{ending}data: {value}{ending}{ending}data: [DONE]{ending}{ending}");
        let mut stream = assembler();
        for byte in wire.as_bytes() {
            stream.push(&[*byte]).unwrap();
        }
        let result = stream.finish().unwrap();
        let reply = ModelReply::decode(result.as_bytes(), &Limits::default()).unwrap();
        assert_eq!(reply.text(), "ok");
        assert_eq!(reply.request_id(), u64::MAX);
    }
}
#[test]
fn complete_empty_stop_normalizes_to_zero_calls_only_after_done_and_successful_end() {
    let terminal = event(&chunk(
        json!({"role":"assistant","content":null}),
        json!("stop"),
    ));
    let mut incomplete = assembler();
    incomplete.push(&terminal).unwrap();
    assert!(incomplete.finish().is_err());
    let wire = [terminal, b"data: [DONE]\r\n\r\n".to_vec()].concat();
    for split in 0..=wire.len() {
        let mut stream = assembler();
        stream.push(&wire[..split]).unwrap();
        stream.push(&wire[split..]).unwrap();
        let reply = stream.finish().unwrap();
        let parsed = ModelReply::decode(reply.as_bytes(), &Limits::default()).unwrap();
        assert_eq!(parsed.call_count(), 0);
        assert_eq!(parsed.text(), "");
    }
    let mut stream = assembler();
    stream.push(&wire).unwrap();
    assert!(stream.push(b"data: {}\n\n").is_err());
    assert!(stream.finish().is_err());
}
#[test]
fn provisional_text_and_terminal_chunk_without_done_or_unterminated_event_never_finish() {
    let first = event(&chunk(
        json!({"role":"assistant","content":"partial"}),
        Value::Null,
    ));
    let mut stream = assembler();
    stream.push(&first).unwrap();
    assert_eq!(stream.provisional_text(), "partial");
    assert!(matches!(stream.finish(), Err(StreamError::Incomplete)));
    let terminal = event(&chunk(
        json!({"role":"assistant","content":"complete"}),
        json!("stop"),
    ));
    for suffix in [
        b"".as_slice(),
        b"data: [DONE]",
        b"data: [DONE]\n",
        b"data: [DON\n\n",
    ] {
        let mut stream = assembler();
        stream.push(&terminal).unwrap();
        let _ = stream.push(suffix);
        assert!(stream.finish().is_err());
    }
    let mut stream = assembler();
    assert!(stream.push(b"data: [DONE]\n\n").is_err());
    assert!(matches!(stream.finish(), Err(StreamError::Failed)));
}
#[test]
fn malformed_duplicate_model_id_index_role_refusal_and_terminal_conflicts_poison_stream() {
    let good = chunk(json!({"role":"assistant","content":"ok"}), Value::Null);
    let mut cases = Vec::new();
    for (key, value) in [
        ("model", json!("wrong")),
        ("choices", json!([])),
        ("error", json!({"message":"SECRET_MARKER"})),
    ] {
        let mut v = good.clone();
        v[key] = value;
        cases.push(event(&v));
    }
    for (key, value) in [("index", json!(1)), ("finish_reason", json!("length"))] {
        let mut v = good.clone();
        v["choices"][0][key] = value;
        cases.push(event(&v));
    }
    for (key, value) in [
        ("role", json!("user")),
        ("content", json!(1)),
        ("refusal", json!("SECRET_MARKER")),
        ("function_call", json!({})),
    ] {
        let mut v = good.clone();
        v["choices"][0]["delta"][key] = value;
        cases.push(event(&v));
    }
    cases.push(b"data: {\"model\":\"m\",\"model\":\"m\"}\n\n".to_vec());
    cases.push(b"data: {\"ignored\":{\"a\":0,\"\\u0061\":1}}\n\n".to_vec());
    cases.push(b"data: \xff\n\n".to_vec());
    cases.push(b"event: error\ndata: {}\n\n".to_vec());
    for bad in cases {
        let mut stream = assembler();
        let error = stream.push(&bad).err().unwrap();
        assert!(!format!("{error:?}: {error}").contains("SECRET_MARKER"));
        assert_eq!(stream.push(&event(&good)), Err(StreamError::Failed));
        assert!(matches!(stream.finish(), Err(StreamError::Failed)));
    }
    let first = trace()[0].clone();
    let mut conflict = good.clone();
    conflict["id"] = json!("different");
    let mut stream = assembler();
    stream.push(&first).unwrap();
    assert!(stream.push(&event(&conflict)).is_err());
    let terminal = event(&chunk(
        json!({"role":"assistant","content":"ok"}),
        json!("stop"),
    ));
    let mut stream = assembler();
    stream.push(&terminal).unwrap();
    assert!(stream.push(&event(&good)).is_err());
}
#[test]
fn whole_wire_event_crlf_and_event_count_limits_include_ignored_metadata() {
    let frames = trace();
    let wire = frames.concat();
    let event_max = frames.iter().map(Vec::len).max().unwrap() as u64;
    for (bounds, passes) in [
        (
            StreamLimits {
                max_wire_bytes: wire.len() as u64,
                max_event_bytes: event_max,
                max_events: 5,
            },
            true,
        ),
        (
            StreamLimits {
                max_wire_bytes: wire.len() as u64 - 1,
                max_event_bytes: event_max,
                max_events: 5,
            },
            false,
        ),
        (
            StreamLimits {
                max_wire_bytes: wire.len() as u64,
                max_event_bytes: event_max - 1,
                max_events: 5,
            },
            false,
        ),
        (
            StreamLimits {
                max_wire_bytes: wire.len() as u64,
                max_event_bytes: event_max,
                max_events: 4,
            },
            false,
        ),
    ] {
        let mut stream = ChatCompletionStream::new(1, "m", Limits::default(), bounds).unwrap();
        let pushed = stream.push(&wire);
        if passes {
            pushed.unwrap();
            stream.finish().unwrap();
        } else {
            assert!(pushed.is_err());
            assert!(matches!(stream.finish(), Err(StreamError::Failed)));
        }
    }
    let mut stream = ChatCompletionStream::new(
        1,
        "m",
        Limits::default(),
        StreamLimits {
            max_wire_bytes: 1024,
            max_event_bytes: 8,
            max_events: 4,
        },
    )
    .unwrap();
    assert!(stream.push(b":12345678\n\n").is_err());
    let mut stream = assembler();
    let giant = format!("data: {}\n\n", "x".repeat(4096));
    assert!(stream.push(giant.as_bytes()).is_err());
}
#[test]
fn tool_call_index_identity_cardinality_and_late_argument_failure_never_salvage_first() {
    let first = trace()[0].clone();
    for delta in [
        json!({"tool_calls":[{"index":2,"id":"new","type":"function"}]}),
        json!({"tool_calls":[{"index":0,"id":"different"}]}),
        json!({"tool_calls":[{"index":0,"type":"other"}]}),
        json!({"tool_calls":[{"index":0,"function":{"arguments":0}}]}),
    ] {
        let mut stream = assembler();
        stream.push(&first).unwrap();
        assert!(stream.push(&event(&chunk(delta, Value::Null))).is_err());
    }
    let call = |index: u64, id: &str, args: &str| json!({"index":index,"id":id,"type":"function","function":{"name":"no_op","arguments":args}});
    let value = chunk(
        json!({"role":"assistant","content":null,"tool_calls":[call(0,"first","{}"),call(1,"second","{}")] }),
        json!("tool_calls"),
    );
    let mut stream = assembler();
    stream.push(&event(&value)).unwrap();
    stream.push(b"data: [DONE]\n\n").unwrap();
    let result = stream.finish().unwrap();
    let reply = ModelReply::decode(result.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(
        reply.calls().map(|c| c.call_id).collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert!(matches!(
        reply.control_call(),
        Err(ReplyError::MultipleToolCalls)
    ));
    for args in ["{\"x\":0,\"x\":1}", "{", "1.5"] {
        let value = chunk(
            json!({"role":"assistant","content":null,"tool_calls":[call(0,"first","{}"),call(1,"second",args)]}),
            json!("tool_calls"),
        );
        let mut stream = assembler();
        stream.push(&event(&value)).unwrap();
        stream.push(b"data: [DONE]\n\n").unwrap();
        assert!(stream.finish().is_err());
    }
}
#[test]
fn done_is_not_permission_to_ignore_trailing_data_or_conflicting_token_reports() {
    let mut stream = assembler();
    stream.push(&trace().concat()).unwrap();
    assert!(stream.push(b"data: [DONE]\n\n").is_err());
    assert!(matches!(stream.finish(), Err(StreamError::Failed)));
    let frames = trace();
    let mut stream = assembler();
    stream.push(&frames[..4].concat()).unwrap();
    let changed = json!({"id":"response-1","model":"m","choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0}});
    assert!(stream.push(&event(&changed)).is_err());
    let mut stream = assembler();
    stream.push(&frames[..4].concat()).unwrap();
    stream.push(&frames[3]).unwrap();
    stream.push(&frames[4]).unwrap();
    assert_eq!(stream.finish().unwrap().as_bytes(), complete().as_bytes());
    let mut stream = assembler();
    stream.push(&frames.concat()).unwrap();
    stream.push(b"data: partial").unwrap();
    assert!(stream.finish().is_err());
}
#[test]
fn assembled_and_final_normalized_bounds_remain_separate_and_reasoning_is_native_only() {
    let limits = Limits {
        max_model_reply_bytes: 64,
        ..Limits::default()
    };
    let mut stream = ChatCompletionStream::new(1, "m", limits, caps()).unwrap();
    let raw = chunk(
        json!({"role":"assistant","content":"x".repeat(64)}),
        Value::Null,
    );
    assert!(stream.push(&event(&raw)).is_err());
    let mut stream = ChatCompletionStream::new(1, "m", limits, caps()).unwrap();
    stream
        .push(&event(&chunk(
            json!({"role":"assistant","content":""}),
            json!("stop"),
        )))
        .unwrap();
    stream.push(b"data: [DONE]\n\n").unwrap();
    assert!(stream.finish().is_err());
    let result = complete();
    assert_eq!(result.reasoning_content(), Some("r1r2"));
    assert!(!result.as_bytes().windows(4).any(|v| v == b"r1r2"));
}
