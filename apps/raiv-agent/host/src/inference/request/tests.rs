use super::*;
use crate::{
    inference::{
        normalize_chat_completion,
        sse::{ChatCompletionStream, StreamLimits},
    },
    output::Output,
    GuestVm, VmLimits,
};
use minicbor::encode::Write;
use raiv::{
    budget::BudgetAdmission,
    definition::MemoryProfile,
    format::{ModelValue, ValueRef},
    input::{ActivationInput, ActivationInputSpec, Cause},
    memory::Memory,
    position::Position,
};
pub(super) fn config() -> ProviderConfig {
    ProviderConfig {
        protocol: "openai-chat-completions".into(),
        base_url: "https://opencode.ai/zen/go/v1".into(),
        model: "deepseek-v4.1-flash".into(),
        api_key_env: "OPENCODE_API_KEY".into(),
        stream: true,
        max_tokens: 1024,
        max_request_bytes: 262144,
        max_response_bytes: 1048576,
        max_header_bytes: 16384,
        timeout: std::time::Duration::from_secs(30),
        stream_limits: StreamLimits {
            max_wire_bytes: 1048576,
            max_event_bytes: 65536,
            max_events: 4096,
        },
    }
}
pub(super) fn encoded(
    kind: &str,
    payload: impl FnOnce(&mut minicbor::Encoder<minicbor::encode::write::Cursor<[u8; 32768]>>),
) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(minicbor::encode::write::Cursor::new([0u8; 32768]));
    e.map(3)
        .unwrap()
        .str("type")
        .unwrap()
        .str(kind)
        .unwrap()
        .str("payload")
        .unwrap();
    payload(&mut e);
    e.str("abi_version").unwrap().u64(1).unwrap();
    e.writer().get_ref()[..e.writer().position()].to_vec()
}
pub(super) fn bootstrap(def: &Definition) -> Vec<u8> {
    let memory = if def.memory_profile() == MemoryProfile::IntentsStrategyTactics {
        Some(Memory::encode(&[], &Limits::default().decode_limits()).unwrap())
    } else {
        None
    };
    encoded("initialize", |e| {
        e.map(3).unwrap().str("memory").unwrap();
        if let Some(m) = memory {
            e.bytes(m.as_bytes()).unwrap();
        } else {
            e.null().unwrap();
        }
        e.str("agent_id")
            .unwrap()
            .bytes(&[0xb0; 32])
            .unwrap()
            .str("definition")
            .unwrap()
            .bytes(def.as_bytes())
            .unwrap();
    })
}
pub(super) fn activation(initial: &[u8], actions: u64) -> Vec<u8> {
    let Output::Initialized { root, blocks } = Output::decode(initial, &Limits::default()).unwrap()
    else {
        panic!()
    };
    let observation = ModelValue::encode(
        ValueRef::Map(&[("text", ValueRef::Text("Remember that my name is Ada."))]),
        &Limits::default().decode_limits(),
    )
    .unwrap();
    let input = ActivationInput::encode(
        &ActivationInputSpec {
            event_id: [0x71; 32],
            cause: Cause::ActorWake,
            observed_at: Position {
                domain: [0x30; 32],
                number: 0,
                reference: None,
            },
            observation: observation.as_bytes(),
            receipt: None,
            budget_admission: BudgetAdmission {
                admission_id: [0x72; 32],
                activation_units: 100,
                remaining_episode_units: 100,
                remaining_model_calls: 16,
                remaining_actions: actions,
            },
        },
        &Limits::default(),
    )
    .unwrap();
    encoded("activate", |e| {
        e.map(3)
            .unwrap()
            .str("root")
            .unwrap()
            .bytes(root.as_bytes())
            .unwrap()
            .str("input")
            .unwrap()
            .bytes(input.as_bytes())
            .unwrap()
            .str("blocks")
            .unwrap()
            .array(blocks.len())
            .unwrap();
        for block in blocks.iter() {
            e.map(2)
                .unwrap()
                .str("cid")
                .unwrap()
                .bytes(block.cid.as_bytes())
                .unwrap()
                .str("body")
                .unwrap()
                .bytes(block.body)
                .unwrap();
        }
    })
}
fn infer(output: &[u8]) -> (u64, Request<'_>) {
    let Output::Infer {
        request_id,
        request,
    } = Output::decode(output, &Limits::default()).unwrap()
    else {
        panic!()
    };
    (request_id, request)
}
fn profile(memory: MemoryProfile) -> Definition {
    guest::opencode_definition(
        memory,
        "https://opencode.ai/zen/go/v1",
        "deepseek-v4.1-flash",
        "Assist the native operator; remember relevant facts and finish each request explicitly.",
    )
    .unwrap()
}
fn exercise(mut step: impl FnMut(&[u8]) -> Vec<u8>) {
    let def = profile(MemoryProfile::IntentsStrategyTactics);
    let initial = step(&bootstrap(&def));
    let mut response = step(&activation(&initial, 4));
    for (turn, tool, args) in [
        (1, "patch_memory", r#"{"intents":{"operator_name":"Ada"}}"#),
        (2, "", "{}"),
    ] {
        let (id, request) = infer(&response);
        assert_eq!(id, turn);
        let cfg = config();
        let body = encode_chat_request(&def, request, &cfg, &Limits::default()).unwrap();
        let provider: Value = serde_json::from_slice(&body).unwrap();
        let mut exact = cfg.clone();
        exact.max_request_bytes = body.len() as u64;
        assert_eq!(
            encode_chat_request(&def, request, &exact, &Limits::default()).unwrap(),
            body
        );
        exact.max_request_bytes -= 1;
        assert_eq!(
            encode_chat_request(&def, request, &exact, &Limits::default()).err(),
            Some(RequestError::Limit)
        );
        assert_eq!(provider["model"], cfg.model);
        assert_eq!(provider["thinking"], json!({"type":"disabled"}));
        assert_eq!(provider["parallel_tool_calls"], false);
        assert!(provider["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["function"]["name"] != "no_op"
                && tool["function"].get("strict").is_none()));
        assert_eq!(provider["messages"][0]["role"], "system");
        assert_eq!(provider["messages"][1]["role"], "user");
        let original = value(request.context_bytes(), &Limits::default().decode_limits()).unwrap();
        let system: Value =
            serde_json::from_str(provider["messages"][0]["content"].as_str().unwrap()).unwrap();
        let data: Value =
            serde_json::from_str(provider["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(system, original[0]);
        if turn == 1 {
            assert_eq!(provider["messages"].as_array().unwrap().len(), 2);
        } else {
            assert_eq!(provider["messages"].as_array().unwrap().len(), 4);
            assert_eq!(provider["messages"][2]["role"], "assistant");
            assert_eq!(
                provider["messages"][2]["tool_calls"][0]["function"]["name"],
                "patch_memory"
            );
            assert_eq!(provider["messages"][3]["role"], "tool");
            let result: Value =
                serde_json::from_str(provider["messages"][3]["content"].as_str().unwrap()).unwrap();
            assert_eq!(result, original[6]["entries"][0]["results"][0]);
        }
        assert_eq!(
            data,
            Value::Array(original.as_array().unwrap()[1..].to_vec())
        );
        assert_eq!(
            provider["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["function"]["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            request.tools().map(|v| v.id).collect::<Vec<_>>()
        );
        let call =
            json!({"id":"call-live","type":"function","function":{"name":tool,"arguments":args}});
        let complete = json!({"model":cfg.model,"choices":[{"index":0,"finish_reason":if tool.is_empty(){"stop"}else{"tool_calls"},"message":{"role":"assistant","content":"Acknowledged.","tool_calls":if tool.is_empty(){vec![]}else{vec![call.clone()]}}}]});
        let normalized = normalize_chat_completion(
            &serde_json::to_vec(&complete).unwrap(),
            id,
            &cfg.model,
            &Limits::default(),
        )
        .unwrap();
        check_chat_reply_mode(&def, &normalized).unwrap();
        let mut thought = complete.clone();
        thought["choices"][0]["message"]["reasoning_content"] = json!("unexpected reasoning");
        let unexpected = normalize_chat_completion(
            &serde_json::to_vec(&thought).unwrap(),
            id,
            &cfg.model,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            check_chat_reply_mode(&def, &unexpected),
            Err(RequestError::ReplyMode)
        );
        let mut delta = call;
        delta["index"] = json!(0);
        let chunk = json!({"id":"mock-live","model":cfg.model,"choices":[{"index":0,"finish_reason":if tool.is_empty(){"stop"}else{"tool_calls"},"delta":{"role":"assistant","content":"Acknowledged.","tool_calls":if tool.is_empty(){vec![]}else{vec![delta]}}}]});
        let raw = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            serde_json::to_string(&chunk).unwrap()
        );
        let mut stream =
            ChatCompletionStream::new(id, &cfg.model, Limits::default(), cfg.stream_limits)
                .unwrap();
        stream.push(raw.as_bytes()).unwrap();
        assert_eq!(stream.finish().unwrap().as_bytes(), normalized.as_bytes());
        response = step(&encoded("inference_reply", |e| {
            e.map(2).unwrap().str("reply").unwrap();
            e.writer_mut().write_all(normalized.as_bytes()).unwrap();
            e.str("request_id").unwrap().u64(id).unwrap();
        }));
    }
    let Output::Boundary(candidate) = Output::decode(&response, &Limits::default()).unwrap() else {
        panic!()
    };
    assert_eq!(candidate.expected_revision(), 0);
    let expected = ModelValue::encode(
        ValueRef::Map(&[(
            "intents",
            ValueRef::Map(&[("operator_name", ValueRef::Text("Ada"))]),
        )]),
        &Limits::default().decode_limits(),
    )
    .unwrap();
    assert!(candidate.new_blocks().iter().any(|b| Memory::decode(
        b.body,
        &Limits::default().decode_limits()
    )
    .is_ok_and(|memory| memory.value_bytes() == expected.as_bytes())));
}
#[test]
fn native_opencode_profile_maps_exact_context_and_enabled_tools_across_retained_patch_finish() {
    let mut guest = guest::Guest::new();
    exercise(|frame| guest.step(frame).unwrap());
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_opencode_profile_mapping_mock_json_sse_equals_native_without_transcript_or_network() {
    let module = std::fs::read(std::env::var_os("RAIV_GUEST_WASM").expect("actual guest")).unwrap();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    let mut native = guest::Guest::new();
    exercise(|frame| {
        let expected = native.step(frame).unwrap();
        let actual = vm.round_trip(frame).unwrap();
        assert_eq!(expected, actual);
        actual
    });
}
#[test]
fn profile_and_mapping_refuse_model_endpoint_parameters_policy_rebind_and_output_overflow() {
    let def = profile(MemoryProfile::None);
    let mut guest = guest::Guest::new();
    let initial = guest.step(&bootstrap(&def)).unwrap();
    let output = guest.step(&activation(&initial, 4)).unwrap();
    let (_, request) = infer(&output);
    assert!(encode_chat_request(&def, request, &config(), &Limits::default()).is_ok());
    for mutation in 0..4 {
        let mut cfg = config();
        match mutation {
            0 => cfg.model = "other".into(),
            1 => cfg.base_url = "https://other/v1".into(),
            2 => cfg.protocol = "anthropic-messages".into(),
            _ => cfg.max_request_bytes = 16,
        };
        assert!(encode_chat_request(&def, request, &cfg, &Limits::default()).is_err());
    }
    let other = guest::opencode_definition(
        MemoryProfile::None,
        &config().base_url,
        &config().model,
        "different mandate",
    )
    .unwrap();
    assert_eq!(
        encode_chat_request(&other, request, &config(), &Limits::default()).err(),
        Some(RequestError::Context)
    );
    let scripted = guest::scripted_definition(MemoryProfile::None).unwrap();
    assert_eq!(
        encode_chat_request(&scripted, request, &config(), &Limits::default()).err(),
        Some(RequestError::Binding)
    );
}
#[test]
fn native_provider_tools_preserve_only_guest_budget_enabled_subset() {
    for memory in [MemoryProfile::None, MemoryProfile::IntentsStrategyTactics] {
        let def = profile(memory);
        let mut guest = guest::Guest::new();
        let initial = guest.step(&bootstrap(&def)).unwrap();
        let output = guest.step(&activation(&initial, 0)).unwrap();
        let (_, request) = infer(&output);
        let bytes = encode_chat_request(&def, request, &config(), &Limits::default()).unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let names: Vec<_> = body["tools"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        if names.is_empty() {
            for field in ["tools", "tool_choice", "parallel_tool_calls"] {
                assert!(body.get(field).is_none());
            }
        }
        assert_eq!(
            names,
            if memory == MemoryProfile::None {
                vec![]
            } else {
                vec!["patch_memory"]
            }
        );
    }
}
#[test]
#[ignore = "build fresh tool-free completion wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_tool_free_veto_final_reply_maps_without_api_tool_fields_and_equals_native() {
    let module = std::fs::read(std::env::var_os("RAIV_GUEST_WASM").unwrap()).unwrap();
    let def = profile(MemoryProfile::None);
    let mut native = guest::Guest::new();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    let initial = native.step(&bootstrap(&def)).unwrap();
    assert_eq!(vm.round_trip(&bootstrap(&def)).unwrap(), initial);
    let packet = activation(&initial, 0);
    let output = native.step(&packet).unwrap();
    assert_eq!(vm.round_trip(&packet).unwrap(), output);
    let (id, request) = infer(&output);
    assert_eq!(request.tools().count(), 0);
    let bytes = encode_chat_request(&def, request, &config(), &Limits::default()).unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    for field in ["tools", "tool_choice", "parallel_tool_calls"] {
        assert!(body.get(field).is_none());
    }
    let reply=normalize_chat_completion(&serde_json::to_vec(&json!({"model":config().model,"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":null}}]})).unwrap(),id,&config().model,&Limits::default()).unwrap();
    let packet = encoded("inference_reply", |e| {
        e.map(2).unwrap().str("reply").unwrap();
        e.writer_mut().write_all(reply.as_bytes()).unwrap();
        e.str("request_id").unwrap().u64(id).unwrap();
    });
    let final_output = native.step(&packet).unwrap();
    assert_eq!(vm.round_trip(&packet).unwrap(), final_output);
    assert!(matches!(
        Output::decode(&final_output, &Limits::default()).unwrap(),
        Output::Boundary(_)
    ));
}
#[test]
fn reference_profiles_bind_same_tools_without_noop_or_registered_finish() {
    for memory in [MemoryProfile::None, MemoryProfile::IntentsStrategyTactics] {
        let live = profile(memory);
        let scripted = guest::scripted_definition(memory).unwrap();
        assert_eq!(
            live.tools().collect::<Vec<_>>(),
            scripted.tools().collect::<Vec<_>>()
        );
        assert!(live
            .tools()
            .all(|t| t.id != "no_op" && t.control != raiv::definition::ToolControl::Finish));
        assert_eq!(
            live.tools().len(),
            if memory == MemoryProfile::None { 2 } else { 3 }
        );
    }
}
#[test]
fn tool_removal_preserves_disabled_inference_metadata_not_old_definition_identity() {
    for memory in [MemoryProfile::None, MemoryProfile::IntentsStrategyTactics] {
        let def = profile(memory);
        let expected = ModelValue::encode(
            ValueRef::Map(&[
                ("base_url", ValueRef::Text(&config().base_url)),
                ("model", ValueRef::Text(&config().model)),
                ("provider", ValueRef::Text("opencode-go")),
                ("protocol", ValueRef::Text("openai-chat-completions")),
                ("thinking", ValueRef::Bool(false)),
            ]),
            &Limits::default().decode_limits(),
        )
        .unwrap();
        assert_eq!(def.inference_profile_bytes(), expected.as_bytes());
    }
}
#[test]
fn closed_profile_byte_schema_rejects_extensions_duplicates_thinking_and_credential_endpoints() {
    let def = profile(MemoryProfile::None);
    let profile = guest::ChatProfile::decode(def.inference_profile_bytes()).unwrap();
    assert_eq!(profile.model, "deepseek-v4.1-flash");
    for base in [
        "http://x",
        "https://secret@x",
        "https://x/?key=secret",
        "https://x/#fragment",
    ] {
        assert!(guest::opencode_definition(MemoryProfile::None, base, "m", "mandate").is_err());
    }
    let bytes = def.inference_profile_bytes();
    let mut thinking = bytes.to_vec();
    *thinking.last_mut().unwrap() = 0xf5;
    assert!(guest::ChatProfile::decode(&thinking).is_err());
    let mut trailing = bytes.to_vec();
    trailing.push(0xf6);
    assert!(guest::ChatProfile::decode(&trailing).is_err());
    assert!(guest::ChatProfile::decode(b"\xa2\x65model\x61m\x65model\x61m").is_err());
}
