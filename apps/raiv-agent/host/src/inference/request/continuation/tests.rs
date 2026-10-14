use super::super::tests::{activation, bootstrap, config, encoded};
use super::*;
use crate::{
    inference::{normalize_chat_completion, sse::ChatCompletionStream},
    output::Output,
    run::{Phase, ProgramBinding, Run},
    VmLimits,
};
use raiv::{
    budget::BudgetAdmission,
    definition::MemoryProfile,
    format::{DaslCodec, ModelValue, ValueRef},
    graph::RestoredAgent,
    input::{ActivationInput, ActivationInputSpec, Cause},
    position::Position,
    preparation::{InputBinding, PreparedInput},
};
use serde_json::json;
fn definition(effort: guest::ReasoningEffort) -> Definition {
    guest::opencode_reasoning_definition(
        MemoryProfile::IntentsStrategyTactics,
        &config().base_url,
        &config().model,
        "Remember sparse facts and explicitly finish.",
        effort,
    )
    .unwrap()
}
fn reply(id: u64, tool: &str, args: &str, reason: Option<&str>) -> NormalizedReply {
    let mut body = json!({"model":config().model,"choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":"","tool_calls":[{"id":"native-call","type":"function","function":{"name":tool,"arguments":args}}]}}]});
    if tool.is_empty() {
        body["choices"][0]["finish_reason"] = json!("stop");
        body["choices"][0]["message"]["tool_calls"] = json!([]);
    }
    if let Some(reason) = reason {
        body["choices"][0]["message"]["reasoning_content"] = json!(reason);
    }
    normalize_chat_completion(
        &serde_json::to_vec(&body).unwrap(),
        id,
        &config().model,
        &Limits::default(),
    )
    .unwrap()
}
fn source(def: &Definition) -> (Vec<u8>, RestoredAgent) {
    let initial = guest::Guest::new().step(&bootstrap(def)).unwrap();
    let Output::Initialized { root, blocks } =
        Output::decode(&initial, &Limits::default()).unwrap()
    else {
        panic!()
    };
    let source = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    (initial, source)
}
fn packet(source: &RestoredAgent) -> PreparedInput {
    let input = ActivationInput::encode(
        &ActivationInputSpec {
            event_id: [0x71; 32],
            cause: Cause::ActorWake,
            observed_at: Position {
                domain: [0x30; 32],
                number: 0,
                reference: None,
            },
            observation: ModelValue::encode(
                ValueRef::Map(&[("text", ValueRef::Text("Remember that my name is Ada."))]),
                &Limits::default().decode_limits(),
            )
            .unwrap()
            .as_bytes(),
            receipt: None,
            budget_admission: BudgetAdmission {
                admission_id: [0x72; 32],
                activation_units: 100,
                remaining_episode_units: 100,
                remaining_model_calls: 16,
                remaining_actions: 4,
            },
        },
        &Limits::default(),
    )
    .unwrap();
    PreparedInput::new(
        InputBinding {
            agent_id: source.state().agent_id(),
            execution_domain: source.definition().execution_domain(),
            expected_root: source.root(),
            expected_revision: source.state().revision(),
            admitted_event_id: input.event_id(),
            admitted_input_digest: input.cid().digest(),
        },
        input,
    )
    .unwrap()
}
fn binding(module: &[u8], source: &RestoredAgent) -> ProgramBinding {
    ProgramBinding {
        module_digest: DaslCid::for_block(DaslCodec::Raw, module).digest(),
        definition: source.definition().cid(),
        execution_domain: source.definition().execution_domain(),
    }
}
#[test]
fn enabled_profile_is_explicit_exact_supported_model_and_reply_mode_never_guesses_or_drops_thought()
{
    for effort in [guest::ReasoningEffort::Low, guest::ReasoningEffort::High] {
        let def = definition(effort);
        let parsed = guest::ChatProfile::decode(def.inference_profile_bytes()).unwrap();
        assert_eq!(parsed.reasoning_effort, Some(effort));
        assert!(!def.policy().contains("Thinking is explicitly disabled"));
        let off = guest::opencode_definition(
            MemoryProfile::IntentsStrategyTactics,
            &config().base_url,
            &config().model,
            "Remember sparse facts and explicitly finish.",
        )
        .unwrap();
        assert_ne!(def.cid(), off.cid());
        assert_eq!(
            check_chat_reply_mode(&def, &reply(1, "no_op", "{}", None)),
            Err(RequestError::ReplyMode)
        );
        check_chat_reply_mode(&def, &reply(1, "patch_memory", "{}", Some(""))).unwrap();
        for reason in [None, Some(""), Some("opaque final")] {
            check_chat_reply_mode(&def, &reply(1, "", "{}", reason)).unwrap();
        }
        check_chat_reply_mode(&off, &reply(1, "", "{}", None)).unwrap();
        assert_eq!(
            check_chat_reply_mode(&off, &reply(1, "", "{}", Some("opaque final"))),
            Err(RequestError::ReplyMode)
        );
        assert_eq!(
            check_chat_reply_mode(&off, &reply(1, "no_op", "{}", Some("opaque"))),
            Err(RequestError::ReplyMode)
        );
        assert!(ReasoningSession::new(&off, Limits::default()).is_err());
        assert!(guest::opencode_reasoning_definition(
            MemoryProfile::None,
            &config().base_url,
            "other",
            "mandate",
            effort
        )
        .is_err());
        let mut raw = def.inference_profile_bytes().to_vec();
        let bool_index = raw.iter().position(|b| *b == 0xf5).unwrap();
        raw[bool_index] = 0xf4;
        assert!(guest::ChatProfile::decode(&raw).is_err());
    }
}
#[test]
fn exact_transient_join_preserves_reasoning_only_on_assistant_and_rejects_text_call_args_or_length_substitution(
) {
    let records = vec![reply(
        1,
        "no_op",
        r#"{"arguments":"{}"}"#,
        Some("NATIVE_ONLY"),
    )];
    let valid = json!({"role":"assistant","content":null,"tool_calls":[{"id":"native-call","type":"function","function":{"name":"no_op","arguments":r#"{"arguments":"{}"}"#}}]});
    for mutation in 0..5 {
        let mut messages = vec![valid.clone()];
        match mutation {
            0 => messages[0]["content"] = json!("substituted"),
            1 => messages[0]["tool_calls"][0]["id"] = json!("foreign"),
            2 => messages[0]["tool_calls"][0]["function"]["arguments"] = json!("{}"),
            3 => messages.push(valid.clone()),
            _ => messages.clear(),
        };
        assert_eq!(
            attach(&records, &mut messages, &Limits::default()),
            Err(RequestError::Continuation)
        );
    }
    let mut messages = vec![json!({"role":"user","content":"DATA"}), valid];
    attach(&records, &mut messages, &Limits::default()).unwrap();
    assert!(messages[0].get("reasoning_content").is_none());
    assert_eq!(messages[1]["reasoning_content"], "NATIVE_ONLY");
    assert!(!records[0]
        .as_bytes()
        .windows(11)
        .any(|w| w == b"NATIVE_ONLY"));
}
#[test]
fn native_reasoning_continuation_preserves_rejected_multicalls_then_no_call_finishes() {
    use minicbor::encode::Write;
    let def = definition(guest::ReasoningEffort::High);
    let (initial, _) = source(&def);
    let mut native = guest::Guest::new();
    native.step(&activation(&initial, 4)).unwrap();
    let mut records = Vec::new();
    let scenarios = [
        json!({"role":"assistant","content":"","reasoning_content":"private multi","tool_calls":[{"id":"m1","type":"function","function":{"name":"patch_memory","arguments":"{}"}},{"id":"m2","type":"function","function":{"name":"no_op","arguments":"{}"}}]}),
        json!({"role":"assistant","content":"no control","reasoning_content":"private no call"}),
    ];
    for (index, message) in scenarios.into_iter().enumerate() {
        let id = index as u64 + 1;
        let body = json!({"model":config().model,"choices":[{"index":0,"finish_reason":if index==0{"tool_calls"}else{"stop"},"message":message}]});
        let reply = normalize_chat_completion(
            &serde_json::to_vec(&body).unwrap(),
            id,
            &config().model,
            &Limits::default(),
        )
        .unwrap();
        let output = native
            .step(&encoded("inference_reply", |e| {
                e.map(2).unwrap().str("reply").unwrap();
                e.writer_mut().write_all(reply.as_bytes()).unwrap();
                e.str("request_id").unwrap().u64(id).unwrap();
            }))
            .unwrap();
        if index == 1 {
            assert!(matches!(
                Output::decode(&output, &Limits::default()).unwrap(),
                Output::Boundary(_)
            ));
            assert!(!output.windows(15).any(|bytes| bytes == b"private no call"));
            continue;
        }
        records.push(reply);
        let Output::Infer { request, .. } = Output::decode(&output, &Limits::default()).unwrap()
        else {
            panic!()
        };
        let body = super::super::encode_inner(
            &def,
            request,
            &config(),
            &Limits::default(),
            Some(&records),
        )
        .unwrap();
        let data: Value = serde_json::from_slice(&body).unwrap();
        let turns = data["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "assistant")
            .collect::<Vec<_>>();
        assert_eq!(turns.len(), records.len());
        assert_eq!(turns[0]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(turns[0]["tool_calls"][0]["function"]["arguments"], "null");
        assert_eq!(turns[0]["tool_calls"][1]["function"]["arguments"], "null");
        assert_eq!(turns[0]["reasoning_content"], "private multi");
    }
}
#[test]
#[ignore = "build fresh reasoning-enabled wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_reasoning_profile_retains_native_json_sse_thought_across_patch_correction_finish_not_guest_or_checkpoint(
) {
    use minicbor::encode::Write;
    let module = std::fs::read(std::env::var_os("RAIV_GUEST_WASM").unwrap()).unwrap();
    for effort in [guest::ReasoningEffort::Low, guest::ReasoningEffort::High] {
        let def = definition(effort);
        let (initial, source) = source(&def);
        let packet = packet(&source);
        let mut run = Run::start(
            &module,
            binding(&module, &source),
            &source,
            &packet,
            VmLimits::default(),
        )
        .unwrap();
        let mut native = guest::Guest::new();
        let mut expected = native.step(&activation(&initial, 4)).unwrap();
        let mut session = ReasoningSession::new(&def, Limits::default()).unwrap();
        for (id, tool, args) in [
            (1, "patch_memory", r#"{"intents":{"name":"Ada"}}"#),
            (2, "patch_memory", "[]"),
            (3, "", "{}"),
        ] {
            let (ticket, request) = run.pending().unwrap();
            let payload = session.encode(&def, ticket, request, &config()).unwrap();
            assert!(super::super::encode_chat_request(
                &def,
                request,
                &config(),
                &Limits::default()
            )
            .is_err());
            let mut short = config();
            short.max_request_bytes = payload.len() as u64 - 1;
            assert_eq!(
                super::super::encode_inner(
                    &def,
                    request,
                    &short,
                    &Limits::default(),
                    Some(&session.records)
                )
                .err(),
                Some(RequestError::Limit)
            );
            let data: Value = serde_json::from_slice(&payload).unwrap();
            assert_eq!(data["thinking"], json!({"type":"enabled"}));
            assert_eq!(data["reasoning_effort"], effort.as_str());
            assert!(!data["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("NATIVE_ONLY"));
            let assistants = data["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "assistant")
                .collect::<Vec<_>>();
            assert_eq!(assistants.len(), (id - 1) as usize);
            for (index, message) in assistants.iter().enumerate() {
                assert_eq!(message["reasoning_content"], format!("NATIVE_ONLY_{index}"));
            }
            let Output::Infer {
                request: oracle, ..
            } = Output::decode(&expected, &Limits::default()).unwrap()
            else {
                panic!()
            };
            assert_eq!(request.context_bytes(), oracle.context_bytes());
            let thought = format!("NATIVE_ONLY_{}", id - 1);
            let reply = if id == 2 {
                let chunk = json!({"model":config().model,"choices":[{"index":0,"finish_reason":"tool_calls","delta":{"role":"assistant","reasoning_content":thought,"tool_calls":[{"index":0,"id":"native-call","type":"function","function":{"name":tool,"arguments":args}}]}}]});
                let wire = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    serde_json::to_string(&chunk).unwrap()
                );
                let mut stream = ChatCompletionStream::new(
                    id,
                    &config().model,
                    Limits::default(),
                    config().stream_limits,
                )
                .unwrap();
                for byte in wire.as_bytes() {
                    stream.push(&[*byte]).unwrap();
                }
                stream.finish().unwrap()
            } else {
                reply(id, tool, args, if id == 3 { None } else { Some(&thought) })
            };
            check_chat_reply_mode(&def, &reply).unwrap();
            run.reply(ticket, reply.as_bytes()).unwrap();
            expected = native
                .step(&encoded("inference_reply", |e| {
                    e.map(2).unwrap().str("reply").unwrap();
                    e.writer_mut().write_all(reply.as_bytes()).unwrap();
                    e.str("request_id").unwrap().u64(id).unwrap();
                }))
                .unwrap();
            assert!(!expected.windows(11).any(|b| b == b"NATIVE_ONLY"));
            if run.phase() == Phase::Pending {
                session.record(&def, ticket, reply).unwrap();
            } else {
                let count = session.records.len();
                assert_eq!(
                    session.record(&def, ticket, reply),
                    Err(RequestError::Continuation)
                );
                assert_eq!(session.records.len(), count);
            }
        }
        assert_eq!(run.phase(), Phase::Boundary);
        let Output::Boundary(oracle) = Output::decode(&expected, &Limits::default()).unwrap()
        else {
            panic!()
        };
        assert_eq!(run.candidate().unwrap().new_root(), oracle.new_root());
        assert!(!run
            .candidate_snapshot()
            .unwrap()
            .blocks()
            .any(|(_, b)| b.windows(11).any(|w| w == b"NATIVE_ONLY")));
        drop(session);
        assert!(ReasoningSession::new(&def, Limits::default())
            .unwrap()
            .records
            .is_empty());
    }
}
#[test]
#[ignore = "build fresh reasoning-enabled wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_carrier_foreign_stale_repeated_tickets_limits_and_failed_reuse_are_refused() {
    let module = std::fs::read(std::env::var_os("RAIV_GUEST_WASM").unwrap()).unwrap();
    let def = definition(guest::ReasoningEffort::High);
    let (_, source) = source(&def);
    let packet = packet(&source);
    let start = || {
        Run::start(
            &module,
            binding(&module, &source),
            &source,
            &packet,
            VmLimits::default(),
        )
        .unwrap()
    };
    let mut run = start();
    let (ticket, request) = run.pending().unwrap();
    let mut duplicate = ReasoningSession::new(&def, Limits::default()).unwrap();
    duplicate.encode(&def, ticket, request, &config()).unwrap();
    assert_eq!(
        duplicate.encode(&def, ticket, request, &config()).err(),
        Some(RequestError::Continuation)
    );
    assert_eq!(
        duplicate.record(&def, ticket, reply(1, "no_op", "{}", Some("x"))),
        Err(RequestError::Continuation)
    );
    let mut session = ReasoningSession::new(&def, Limits::default()).unwrap();
    session.encode(&def, ticket, request, &config()).unwrap();
    let value = reply(1, "patch_memory", "{}", Some("native opaque"));
    run.reply(ticket, value.as_bytes()).unwrap();
    session.record(&def, ticket, value).unwrap();
    let (next, request) = run.pending().unwrap();
    let foreign = start();
    let (foreign_ticket, _) = foreign.pending().unwrap();
    assert_eq!(
        session
            .encode(&def, foreign_ticket, request, &config())
            .err(),
        Some(RequestError::Continuation)
    );
    assert_eq!(
        session.encode(&def, next, request, &config()).err(),
        Some(RequestError::Continuation)
    );
    let mut limited = ReasoningSession::new(&def, Limits::default()).unwrap();
    limited.pending = Some(next);
    limited.limits.max_context_bytes = 1;
    assert_eq!(
        limited.record(
            &def,
            next,
            reply(next.request_id(), "no_op", "{}", Some("large"))
        ),
        Err(RequestError::Limit)
    );
    assert!(limited.records.is_empty());
    assert_eq!(
        limited.encode(&def, next, request, &config()).err(),
        Some(RequestError::Continuation)
    );
    let mut per_reply = ReasoningSession::new(&def, Limits::default()).unwrap();
    per_reply.pending = Some(next);
    per_reply.limits.max_model_reply_bytes = 512;
    assert_eq!(
        per_reply.record(
            &def,
            next,
            reply(next.request_id(), "no_op", "{}", Some(&"r".repeat(512)))
        ),
        Err(RequestError::Limit)
    );
    assert!(per_reply.records.is_empty());
    let mut wrong = ReasoningSession::new(&def, Limits::default()).unwrap();
    wrong.pending = Some(next);
    assert_eq!(
        wrong.record(&def, next, reply(99, "no_op", "{}", Some("x"))),
        Err(RequestError::Continuation)
    );
}
