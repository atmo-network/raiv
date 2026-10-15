//! Real guest equality: partial/all-skipped feedback is transient Continue, not correction.

use super::*;

fn report(response: &[u8]) -> Value {
    let decoded = decode(response);
    assert_eq!(field(&decoded, "type"), &Value::Text("infer".into()));
    let Value::Bytes(bytes) = field(field(field(&decoded, "payload"), "request"), "context") else {
        panic!()
    };
    let Value::Array(sections) = decode(bytes) else {
        panic!()
    };
    let Value::Array(turns) = field(sections.last().unwrap(), "entries") else {
        panic!()
    };
    let Value::Array(results) = field(turns.last().unwrap(), "results") else {
        panic!()
    };
    assert_eq!(field(&results[0], "ok"), &Value::Bool(true));
    field(&results[0], "result").clone()
}

fn sequence(mut step: impl FnMut(&[u8]) -> Vec<u8>) {
    let data = genesis(1);
    let initial = step(&activate(&data, &packet()));
    let Value::Bytes(context) = field(
        field(field(&decode(&initial), "payload"), "request"),
        "context",
    )
    .clone() else {
        panic!()
    };
    let Value::Array(prefix) = decode(&context) else {
        panic!()
    };
    let Value::Text(policy) = field(&prefix[0], "policy") else {
        panic!()
    };
    for concept in [
        "low-cost semantic discipline",
        "unsupported_root",
        "no nested salvage",
        "without forced correction",
        "budgets and authority",
    ] {
        assert!(policy.contains(concept));
    }
    let Value::Array(tools) = field(&prefix[1], "tools") else {
        panic!()
    };
    let patch_tool = tools
        .iter()
        .find(|tool| field(tool, "id") == &Value::Text("patch_memory".into()))
        .unwrap();
    let Value::Text(description) = field(patch_tool, "description") else {
        panic!()
    };
    for concept in [
        "case-sensitive intents, strategy and tactics",
        "never remapped",
        "full memory",
        "processed does not imply changed",
        "Partial/all-skipped/no-effect remains Continue",
    ] {
        assert!(description.contains(concept));
    }
    let schema = field(patch_tool, "arguments_schema");
    assert_eq!(field(schema, "additionalProperties"), &Value::Bool(false));
    assert_eq!(field(schema, "type"), &Value::Text("object".into()));
    let Value::Map(properties) = field(schema, "properties") else {
        panic!()
    };
    assert_eq!(properties.len(), 3);
    for name in ["intents", "strategy", "tactics"] {
        assert_eq!(
            field(field(schema, "properties"), name),
            &map(vec![(
                "type",
                Value::Array(vec![
                    Value::Text("object".into()),
                    Value::Text("null".into())
                ])
            )])
        );
    }
    let mixed = map(vec![
        ("intents", map(vec![("keep", Value::Bool(false))])),
        ("goals", Value::Array(vec![Value::Null])),
        ("Tactics", map(vec![("wrong", Value::Bool(true))])),
    ]);
    let response = step(&progression::reply(
        1,
        1,
        vec![progression::call("patch_memory", mixed.clone())],
    ));
    let accepted = report(&response);
    assert_eq!(
        field(&accepted, "memory"),
        &map(vec![("intents", map(vec![("keep", Value::Bool(false))]))])
    );
    assert_eq!(field(&accepted, "changed"), &Value::Bool(true));
    assert_eq!(
        field(&accepted, "processed_planes"),
        &Value::Array(vec![Value::Text("intents".into())])
    );
    assert_eq!(
        field(&accepted, "skipped_planes"),
        &fixture(
            &serde_json::json!([{"plane":"goals","reason":"unsupported_root"},{"plane":"Tactics","reason":"unsupported_root"}])
        )
    );
    let Value::Bytes(context) = field(
        field(field(&decode(&response), "payload"), "request"),
        "context",
    )
    .clone() else {
        panic!()
    };
    let Value::Array(sections) = decode(&context) else {
        panic!()
    };
    assert_eq!(&sections[..6], &prefix[..6]);
    let only_unknown = map(vec![
        ("intent", Value::Null),
        ("operations", Value::Array(vec![Value::Null])),
    ]);
    let response = step(&progression::reply(
        2,
        2,
        vec![progression::call("patch_memory", only_unknown)],
    ));
    let unchanged = report(&response);
    assert_eq!(field(&unchanged, "memory"), field(&accepted, "memory"));
    assert_eq!(field(&unchanged, "changed"), &Value::Bool(false));
    assert_eq!(field(&unchanged, "processed_planes"), &Value::Array(vec![]));
    assert_eq!(
        field(&unchanged, "skipped_planes"),
        &fixture(
            &serde_json::json!([{"plane":"intent","reason":"unsupported_root"},{"plane":"operations","reason":"unsupported_root"}])
        )
    );
    let sealed = decode(&step(&progression::reply(3, 3, vec![])));
    assert_eq!(field(&sealed, "type"), &Value::Text("boundary".into()));
    let cand = field(field(&sealed, "payload"), "candidate");
    let Value::Array(blocks) = field(cand, "new_blocks") else {
        panic!()
    };
    let mut memory_found = false;
    for block in blocks {
        let Value::Bytes(bytes) = field(block, "body") else {
            panic!()
        };
        let body = decode(bytes);
        if field(&body, "kind") == &Value::Text("memory".into()) {
            memory_found = true;
            assert_eq!(field(&body, "value"), field(&accepted, "memory"));
        }
        if field(&body, "kind") == &Value::Text("boundary".into()) {
            assert_eq!(
                field(field(&body, "usage"), "model_calls"),
                &Value::Integer(3.into())
            );
            assert_eq!(
                field(field(&body, "usage"), "local_tool_calls"),
                &Value::Integer(2.into())
            );
        }
        assert!(!bytes
            .windows(b"unsupported_root".len())
            .any(|w| w == b"unsupported_root"));
        assert!(!bytes
            .windows(b"processed_planes".len())
            .any(|w| w == b"processed_planes"));
    }
    assert!(memory_found);
}

fn provider_json_sequence(mut step: impl FnMut(&[u8]) -> Vec<u8>, stream: bool) {
    assert_eq!(
        field(&decode(&step(&activate(&genesis(1), &packet()))), "type"),
        &Value::Text("infer".into())
    );
    for (id, tool, args) in [
        (
            1,
            "patch_memory",
            r#"{"goals":[null],"intents":{"keep":false}}"#,
        ),
        (2, "", "{}"),
    ] {
        let body = serde_json::json!({"model":"fixture-model","choices":[{"index":0,"finish_reason":if tool.is_empty(){"stop"}else{"tool_calls"},"message":{"role":"assistant","content":null,"reasoning_content":"PROVIDER_REASONING_NATIVE_ONLY","tool_calls":if tool.is_empty(){vec![]}else{vec![serde_json::json!({"id":"json-call","type":"function","function":{"name":tool,"arguments":args}})]}}}]});
        let normalized = if stream {
            use crate::inference::sse::{ChatCompletionStream, StreamLimits};
            let mut delta = body["choices"][0]["message"].clone();
            if !tool.is_empty() {
                delta["tool_calls"][0]["index"] = serde_json::json!(0);
            }
            let chunk = serde_json::json!({"id":"mock-sse","model":"fixture-model","choices":[{"index":0,"finish_reason":if tool.is_empty(){"stop"}else{"tool_calls"},"delta":delta}]});
            let raw = format!(
                "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
                serde_json::to_string(&chunk).unwrap()
            );
            let mut assembler = ChatCompletionStream::new(
                id,
                "fixture-model",
                raiv::limits::Limits::default(),
                StreamLimits {
                    max_wire_bytes: 65536,
                    max_event_bytes: 4096,
                    max_events: 8,
                },
            )
            .unwrap();
            for byte in raw.as_bytes() {
                assembler.push(&[*byte]).unwrap();
            }
            assembler.finish().unwrap()
        } else {
            crate::inference::normalize_chat_completion(
                &serde_json::to_vec(&body).unwrap(),
                id,
                "fixture-model",
                &raiv::limits::Limits::default(),
            )
            .unwrap()
        };
        let input = encode(&map(vec![
            ("abi_version", Value::Integer(1.into())),
            ("type", Value::Text("inference_reply".into())),
            (
                "payload",
                map(vec![
                    ("request_id", Value::Integer(id.into())),
                    ("reply", decode(normalized.as_bytes())),
                ]),
            ),
        ]));
        let response = step(&input);
        assert!(!response
            .windows(b"PROVIDER_REASONING_NATIVE_ONLY".len())
            .any(|w| w == b"PROVIDER_REASONING_NATIVE_ONLY"));
        if id == 1 {
            assert_eq!(
                field(&report(&response), "memory"),
                &map(vec![("intents", map(vec![("keep", Value::Bool(false))]))])
            );
        } else {
            assert_eq!(
                field(&decode(&response), "type"),
                &Value::Text("boundary".into())
            );
        }
    }
}

#[test]
fn native_mock_provider_json_normalization_drives_retained_guest_without_native_reasoning() {
    let mut guest = guest::Guest::new();
    provider_json_sequence(|input| guest.step(input).unwrap(), false);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_mock_provider_json_normalization_equals_native_across_patch_then_explicit_finish() {
    let module = wasm();
    let mut native = guest::Guest::new();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    provider_json_sequence(
        |input| {
            let expected = native.step(input).unwrap();
            let actual = vm.round_trip(input).unwrap();
            assert_eq!(expected, actual);
            actual
        },
        false,
    );
}

#[test]
fn native_mock_provider_sse_completes_before_retained_guest_advancement() {
    let mut guest = guest::Guest::new();
    provider_json_sequence(|input| guest.step(input).unwrap(), true);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_mock_provider_sse_completion_equals_native_across_patch_then_explicit_finish() {
    let module = wasm();
    let mut native = guest::Guest::new();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    provider_json_sequence(
        |input| {
            let expected = native.step(input).unwrap();
            let actual = vm.round_trip(input).unwrap();
            assert_eq!(expected, actual);
            actual
        },
        true,
    );
}

fn depth_failure(mut step: impl FnMut(&[u8]) -> Vec<u8>) {
    assert_eq!(
        field(&decode(&step(&activate(&genesis(1), &packet()))), "type"),
        &Value::Text("infer".into())
    );
    let mut value = Value::Text("leaf".into());
    for _ in 0..23 {
        value = map(vec![("child", value)]);
    }
    let arguments = map(vec![("tactics", map(vec![("deep", value)]))]);
    let error = decode(&step(&progression::reply(
        1,
        1,
        vec![progression::call("patch_memory", arguments)],
    )));
    assert_eq!(field(&error, "type"), &Value::Text("error".into()));
    assert_eq!(
        field(field(&error, "payload"), "code"),
        &Value::Text("activation_failed".into())
    );
}

#[test]
fn native_organic_report_embedding_depth_failure_never_seals_a_candidate() {
    let mut guest = guest::Guest::new();
    depth_failure(|input| guest.step(input).unwrap());
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_organic_report_embedding_depth_failure_equals_native_without_boundary() {
    let mut guest = guest::Guest::new();
    let module = wasm();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    depth_failure(|input| {
        let expected = guest.step(input).unwrap();
        let actual = vm.round_trip(input).unwrap();
        assert_eq!(actual, expected);
        actual
    });
}

#[test]
fn native_organic_patch_partial_and_all_skipped_continue_with_feedback_and_no_persistent_diagnostics(
) {
    let mut guest = guest::Guest::new();
    sequence(|input| guest.step(input).unwrap());
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_organic_patch_reports_equal_native_across_retained_activation_then_finish() {
    let module = wasm();
    let mut native = guest::Guest::new();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    sequence(|input| {
        let expected = native.step(input).unwrap();
        let actual = vm.round_trip(input).unwrap();
        assert_eq!(expected, actual);
        actual
    });
}
