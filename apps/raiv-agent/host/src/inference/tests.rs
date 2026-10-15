use super::*;
use raiv::activation::ReplyError;
use serde_json::json;

fn completion(arguments: &str) -> Value {
    json!({"model":"deepseek-v4.1-flash","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"patch_memory","arguments":arguments}}]}}]})
}

fn normalize(value: &Value) -> Result<NormalizedReply, NormalizeError> {
    normalize_chat_completion(
        &serde_json::to_vec(value).unwrap(),
        u64::MAX,
        "deepseek-v4.1-flash",
        &Limits::default(),
    )
}

#[test]
fn noop_wrapped_arguments_are_preserved_exactly_not_created_or_stripped_by_normalization() {
    for (arguments, expected) in [
        ("{}", b"\xa0".as_slice()),
        (
            r#"{"arguments":"{}"}"#,
            b"\xa1\x69arguments\x62{}".as_slice(),
        ),
        (r#"{"arguments":{}}"#, b"\xa1\x69arguments\xa0".as_slice()),
    ] {
        let mut body = completion(arguments);
        body["choices"][0]["message"]["tool_calls"][0]["function"]["name"] = json!("no_op");
        let reply = normalize(&body).unwrap();
        let parsed = ModelReply::decode(reply.as_bytes(), &Limits::default()).unwrap();
        assert_eq!(
            parsed.control_call().unwrap().unwrap().arguments_bytes(),
            expected
        );
    }
}

#[test]
fn final_no_calls_requires_normal_stop_but_allows_empty_or_null_text_without_invented_tools() {
    for content in [json!("answer"), json!(""), Value::Null] {
        let good = json!({"model":"deepseek-v4.1-flash","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":content,"tool_calls":[]}}]});
        let normalized = normalize(&good).unwrap();
        let parsed = ModelReply::decode(normalized.as_bytes(), &Limits::default()).unwrap();
        assert_eq!(parsed.call_count(), 0);
        assert!(parsed.control_call().unwrap().is_none());
        assert_eq!(parsed.text(), content.as_str().unwrap_or(""));
        for reason in ["tool_calls", "length", "content_filter", "error"] {
            let mut bad = good.clone();
            bad["choices"][0]["finish_reason"] = json!(reason);
            assert!(normalize(&bad).is_err());
        }
        let mut bad = good.clone();
        bad["choices"][0]["message"]["refusal"] = json!("refusal");
        assert!(normalize(&bad).is_err());
        let mut bad = good;
        bad["choices"][0]["message"]["tool_calls"] = json!([{"id":"broken"}]);
        assert!(normalize(&bad).is_err());
    }
    let mut bad = completion("{}");
    bad["choices"][0]["finish_reason"] = json!("stop");
    assert!(normalize(&bad).is_err());
}

#[test]
fn raw_duplicates_include_ignored_payloads_escaped_unicode_and_nested_arguments_before_map_ownership(
) {
    let limits = DecodeLimits::default();
    for raw in [
        r#"{"a":1,"a":2}"#,
        r#"{"extension":{"a":0,"\u0061":1}}"#,
        r#"{"😀":0,"\ud83d\ude00":1}"#,
        r#"[{}, {"a":[{"x":false,"x":true}]}]"#,
    ] {
        assert!(json::parse(raw.as_bytes(), &limits).is_err());
    }
    assert!(json::parse("{\"é\":0,\"é\":1}".as_bytes(), &limits).is_ok());
    assert!(normalize(&completion(r#"{"goals":{"n":0,"\u006e":1}}"#)).is_err());
    let body = serde_json::to_string(&completion("{}")).unwrap();
    let duplicate = body.replacen("{", "{\"ignored\":0,\"ignored\":1,", 1);
    assert!(normalize_chat_completion(
        duplicate.as_bytes(),
        1,
        "deepseek-v4.1-flash",
        &Limits::default()
    )
    .is_err());
}

#[test]
fn complete_syntax_utf8_depth_byte_and_aggregate_entry_preflight_has_exact_boundaries() {
    for raw in [
        &b""[..],
        b"{",
        b"[1,]",
        b"0 1",
        b"{\"a\":\"\xff\"}",
        b"\"\\ud800\"",
        b"1e9999",
    ] {
        assert!(json::parse(raw, &DecodeLimits::default()).is_err());
    }
    let mut limits = DecodeLimits {
        max_block_bytes: 2,
        max_depth: 1,
        max_collection_entries: 1,
    };
    assert!(json::parse(b"{}", &limits).is_ok());
    limits.max_block_bytes = 1;
    assert!(json::parse(b"{}", &limits).is_err());
    limits = DecodeLimits {
        max_block_bytes: 64,
        max_depth: 3,
        max_collection_entries: 4,
    };
    assert!(json::parse(b"{\"a\":[0]}", &limits).is_ok());
    limits.max_depth = 2;
    assert!(json::parse(b"{\"a\":[0]}", &limits).is_err());
    limits.max_depth = 3;
    limits.max_collection_entries = 3;
    assert!(json::parse(b"{\"a\":[0]}", &limits).is_err());
    let deep = "[".repeat(10000) + "0" + &"]".repeat(10000);
    assert!(json::parse(deep.as_bytes(), &DecodeLimits::default()).is_err());
}

#[test]
fn canonical_reply_matches_independent_ciborium_with_native_correlation_unknown_usage_and_separate_reasoning(
) {
    let mut response =
        completion(r#"{"strategy":{"z":0,"aa":false},"goals":[null],"$ref":"literal"}"#);
    response["id"] = json!("foreign-response-id");
    response["request_id"] = json!(1);
    response["choices"][0]["message"]["reasoning_content"] = json!("NATIVE_RAM_REASONING_ONLY");
    let normalized = normalize(&response).unwrap();
    let reply = ModelReply::decode(normalized.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(reply.request_id(), u64::MAX);
    assert_eq!(reply.text(), "");
    assert_eq!(reply.call_count(), 1);
    assert_eq!(reply.usage().model_calls, 0);
    assert_eq!(reply.usage().local_tool_calls, 0);
    assert_eq!(reply.usage().input_tokens, None);
    assert_eq!(reply.usage().output_tokens, None);
    assert_eq!(reply.usage().billed_units, None);
    assert_eq!(
        normalized.reasoning_content(),
        Some("NATIVE_RAM_REASONING_ONLY")
    );
    assert!(!normalized
        .as_bytes()
        .windows(b"NATIVE_RAM_REASONING_ONLY".len())
        .any(|s| s == b"NATIVE_RAM_REASONING_ONLY"));
    use ciborium::value::Value as C;
    let map = |entries: Vec<(&str, C)>| {
        C::Map(
            entries
                .into_iter()
                .map(|(k, v)| (C::Text(k.into()), v))
                .collect(),
        )
    };
    let expected = map(vec![
        ("text", C::Text("".into())),
        (
            "calls",
            C::Array(vec![map(vec![
                ("call_id", C::Text("call-1".into())),
                ("tool_id", C::Text("patch_memory".into())),
                (
                    "arguments",
                    map(vec![
                        ("$ref", C::Text("literal".into())),
                        ("goals", C::Array(vec![C::Null])),
                        (
                            "strategy",
                            map(vec![("z", C::Integer(0.into())), ("aa", C::Bool(false))]),
                        ),
                    ]),
                ),
            ])]),
        ),
        (
            "usage",
            map(vec![
                ("model_calls", C::Integer(0.into())),
                ("billed_units", C::Null),
                ("input_tokens", C::Null),
                ("output_tokens", C::Null),
                ("local_tool_calls", C::Integer(0.into())),
            ]),
        ),
        ("request_id", C::Integer(u64::MAX.into())),
    ]);
    let mut oracle = Vec::new();
    ciborium::ser::into_writer(&expected, &mut oracle).unwrap();
    assert_eq!(normalized.as_bytes(), oracle);
    assert_eq!(
        reply.control_call().unwrap().unwrap().meta().tool_id,
        "patch_memory"
    );
}

#[test]
fn full_width_counters_are_exact_and_scalar_array_or_nonmodel_integer_selection_stays_core_owned() {
    let mut response = completion("{}");
    response["usage"] = json!({"prompt_tokens":u64::MAX,"completion_tokens":0});
    let normalized = normalize(&response).unwrap();
    let reply = ModelReply::decode(normalized.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(reply.usage().input_tokens, Some(u64::MAX));
    assert_eq!(reply.usage().output_tokens, Some(0));
    for args in [
        "null",
        "[]",
        "true",
        "18446744073709551615",
        r#"{"n":18446744073709551615}"#,
    ] {
        let data = normalize(&completion(args)).unwrap();
        let reply = ModelReply::decode(data.as_bytes(), &Limits::default()).unwrap();
        assert!(reply.control_call().is_err());
    }
    let data = normalize(&completion(r#"{"n":18446744073709551615}"#)).unwrap();
    assert!(data
        .as_bytes()
        .windows(9)
        .any(|b| b == b"\x1b\xff\xff\xff\xff\xff\xff\xff\xff"));
    for args in ["1.0", "1e2", "18446744073709551616"] {
        assert!(normalize(&completion(args)).is_err());
    }
    for usage in [
        json!({"prompt_tokens":-1}),
        json!({"completion_tokens":0.5}),
        json!(7),
    ] {
        response["usage"] = usage;
        assert!(normalize(&response).is_err());
    }
}

#[test]
fn complete_multi_call_metadata_is_preserved_without_selecting_first_and_bad_later_argument_cannot_be_salvaged(
) {
    let mut value = completion("{}");
    let mut second = value["choices"][0]["message"]["tool_calls"][0].clone();
    second["id"] = json!("call-2");
    second["function"]["name"] = json!("no_op");
    value["choices"][0]["message"]["tool_calls"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let data = normalize(&value).unwrap();
    let reply = ModelReply::decode(data.as_bytes(), &Limits::default()).unwrap();
    assert_eq!(
        reply.calls().map(|c| c.call_id).collect::<Vec<_>>(),
        vec!["call-1", "call-2"]
    );
    assert!(matches!(
        reply.control_call(),
        Err(ReplyError::MultipleToolCalls)
    ));
    value["choices"][0]["message"]["tool_calls"][1]["function"]["arguments"] =
        json!("{\"x\":1,\"x\":2}");
    assert!(normalize(&value).is_err());
    value["choices"][0]["message"]["tool_calls"][1]["function"]["arguments"] = json!("{}");
    value["choices"][0]["message"]["tool_calls"][1]["id"] = json!("call-1");
    assert!(normalize(&value).is_err());
}

#[test]
fn terminal_schema_model_refusal_lengths_and_errors_fail_closed_without_raw_error_echo() {
    let good = completion("{}");
    for (key, value) in [
        ("finish_reason", json!("length")),
        ("finish_reason", json!("content_filter")),
        ("finish_reason", Value::Null),
        ("index", json!(1)),
    ] {
        let mut body = good.clone();
        body["choices"][0][key] = value;
        assert!(normalize(&body).is_err());
    }
    for (key, value) in [
        ("role", json!("user")),
        ("refusal", json!("SECRET_MARKER")),
        ("content", json!([])),
        ("reasoning_content", json!({})),
        ("function_call", json!({})),
    ] {
        let mut body = good.clone();
        body["choices"][0]["message"][key] = value;
        let err = normalize(&body).err().unwrap();
        assert!(!format!("{err:?}: {err}").contains("SECRET_MARKER"));
    }
    let mut body = good.clone();
    body["model"] = json!("different");
    assert_eq!(normalize(&body).err(), Some(NormalizeError::Model));
    body = good.clone();
    body["choices"] = json!([]);
    assert!(normalize(&body).is_err());
    body = good.clone();
    body["error"] = json!({"message":"SECRET_MARKER"});
    let err = normalize(&body).err().unwrap();
    assert!(!format!("{err:?}: {err}").contains("SECRET_MARKER"));
    body = good;
    body["choices"][0]["message"]["tool_calls"] = json!([]);
    assert_eq!(normalize(&body).err(), Some(NormalizeError::Incomplete));
}

#[test]
fn textual_reply_has_no_control_and_final_frame_bounds_include_embedding_and_generated_usage() {
    let response = json!({"model":"m","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":""}}]});
    let raw = serde_json::to_vec(&response).unwrap();
    let normalized = normalize_chat_completion(&raw, 1, "m", &Limits::default()).unwrap();
    assert!(
        ModelReply::decode(normalized.as_bytes(), &Limits::default())
            .unwrap()
            .control_call()
            .unwrap()
            .is_none()
    );
    assert!(normalized.as_bytes().len() > raw.len());
    let limits = Limits {
        max_model_reply_bytes: raw.len() as u64,
        ..Limits::default()
    };
    assert!(normalize_chat_completion(&raw, 1, "m", &limits).is_err());
    let args = "[".repeat(15) + "0" + &"]".repeat(15);
    let raw = serde_json::to_vec(&completion(&args)).unwrap();
    let limits = Limits {
        max_depth: 16,
        ..Limits::default()
    };
    assert!(json::parse(
        args.as_bytes(),
        &DecodeLimits {
            max_depth: 16,
            ..DecodeLimits::default()
        }
    )
    .is_ok());
    assert!(normalize_chat_completion(&raw, 1, "deepseek-v4.1-flash", &limits).is_err());
}
