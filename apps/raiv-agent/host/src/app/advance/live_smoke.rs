//! Opt-in real provider smoke; bounded native test administration, not effects/authentication proof.
use super::*;
use crate::app::interactive::GrantPlan;
#[path = "interactive_smoke.rs"]
mod interactive_smoke;
struct Real<'a> {
    inner: LiveProvider<'a>,
    posts: u64,
    cap: u64,
    diagnostic: bool,
}
impl ProviderPort for Real<'_> {
    fn guest_refused(&mut self, category: &'static str) {
        eprintln!("\nnative_reply_failure={category} posts={}", self.posts);
    }
    fn prepare(&mut self, definition: &Definition, input: &ActivationInput) -> Result<()> {
        self.inner.prepare(definition, input)
    }
    fn complete(&mut self, id: u64, payload: &[u8], limits: &Limits) -> Result<NormalizedReply> {
        if self.posts >= self.cap {
            return Err(fail("live smoke HTTP-call cap; no exposure"));
        }
        if let Ok(body) = serde_json::from_slice::<serde_json::Value>(payload) {
            if let Some(context) = body["messages"][1]["content"].as_str() {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(context) {
                    if data[5]["entries"]
                        .as_array()
                        .and_then(|entries| entries.last())
                        .is_some_and(|turn| turn["error"] == "invalid_arguments")
                    {
                        eprintln!("\nnative_corrective_feedback=invalid_arguments request_id={id}");
                    }
                }
            }
        }
        self.posts += 1;
        let result = if self.diagnostic {
            self.inner.client.as_ref().ok_or_else(||fail("diagnostic provider not prepared"))?.complete_diagnostic(id,payload,limits,&mut |report|eprintln!("provider_diagnostic={report:?}; untrusted finite classification, not raw message")).map_err(|e|Box::new(e) as Box<dyn std::error::Error>)
        } else {
            self.inner.complete(id, payload, limits)
        };
        if let Ok(reply) = &result {
            eprintln!(
                "\nnative_reasoning request_id={id} bytes={:?} posts={}",
                reply.reasoning_content().map(str::len),
                self.posts
            );
            match ModelReply::decode(reply.as_bytes(), limits) {
                Ok(parsed) => {
                    let control = match parsed.control_call() {
                        Ok(Some(call)) => match call.meta().tool_id {
                            "patch_memory" => "patch_memory",
                            "trade" => "trade",
                            "set_wake" => "set_wake",
                            "veto" => "veto",
                            "pass" => "pass",
                            _ => "other",
                        },
                        Ok(None) => "finish",
                        Err(_) => "invalid_or_multiple",
                    };
                    let shape = parsed.control_call().ok().flatten().map(|call| {
                        (
                            call.arguments_bytes().len(),
                            call.arguments_bytes() == b"\xa0",
                        )
                    });
                    eprintln!("\nnative_normalized_reply request_id={id} control={control} argument_bytes_and_empty_map={shape:?} text_bytes={} posts={}",parsed.text().len(),self.posts);
                }
                Err(_) => eprintln!("native_normalized_reply=invalid posts={}", self.posts),
            }
        }
        if let Err(error) = &result {
            // ClientError is a closed enum with static variants and at most a numeric status.
            if let Some(kind) =
                error.downcast_ref::<raiv_agent_host::inference::http::ClientError>()
            {
                eprintln!("\nnative_provider_class={kind:?}");
            }
            eprintln!("\nnative_provider_failure={error}; provider body withheld");
        }
        result
    }
}
#[test]
#[ignore = "explicit live_provider_smoke launcher diagnostic; one fresh event, no automatic retry"]
fn live_provider_smoke_launcher_general_assistant_diagnostic() {
    let max_tokens = match std::env::var("RAIV_LIVE_SMOKE").ok().as_deref() {
        Some("opencode-go/deepseek-v4.1-flash/launcher-diagnostic") => 256,
        Some("opencode-go/deepseek-v4.1-flash/launcher-diagnostic-1024") => 1024,
        _ => panic!("explicit bounded launcher diagnostic approval required"),
    };
    assert!(
        std::env::var("OPENCODE_API_KEY").is_ok_and(|value| !value.is_empty()),
        "selected credential unavailable; value withheld"
    );
    let mandate = "Assist the local operator. Preserve useful behavioral continuity with patch_memory when needed, then finish with a complete normal response without tool calls. External actions are not approved.";
    let [agent_id, world_id] = crate::app::interactive::random_ids().unwrap();
    let event_ids = crate::app::interactive::random_ids().unwrap();
    let mut home = Home::configured(|source| {
        source
            .replace("Assist the native operator. Preserve relevant behavioral continuity in sparse Memory and explicitly finish each request.", mandate)
            .replace("[inference]", "[inference]\nreasoning_effort = \"high\"")
            .replace("max_tokens = 1024", &format!("max_tokens = {max_tokens}"))
            .replace("timeout_ms = 60000", "timeout_ms = 20000")
            .replace(&"b0".repeat(32), &hex(&agent_id))
            .replace(&"cc".repeat(32), &hex(&world_id))
    });
    home.retain = true;
    eprintln!(
        "retained_launcher_diagnostic_namespace={}",
        home.root.display()
    );
    let selected = home.config.live.as_ref().unwrap();
    assert_eq!(selected.base_url, "https://opencode.ai/zen/go/v1");
    assert_eq!(selected.model, "deepseek-v4.1-flash");
    assert_eq!(selected.max_tokens, max_tokens);
    assert_eq!(selected.timeout, std::time::Duration::from_secs(20));
    assert_eq!(home.config.mandate.as_deref(), Some(mandate));
    let (world_digest, source) = home.select();
    let paired = WorldRecoveryPin {
        source,
        world_digest,
    };
    let plan = GrantPlan {
        activation_units: 100,
        episode_units: 100,
        model_calls: 3,
        actions: 0,
        max_events: 1,
    };
    let (input, grant) = plan.input("yp", paired, event_ids).unwrap();
    let before = home.world();
    let outcome = execute_native(
        &home.config,
        &home.config.definition().unwrap(),
        &home.module,
        NativeRequest {
            paired,
            input,
            grant,
            cause: Cause::ActorWake,
            script: None,
            capture_continuation: true,
        },
    );
    assert_eq!(home.world(), before);
    let raw = home.main();
    let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
    eprintln!("launcher_diagnostic revision={} admissions={} world_unchanged=true max_calls=3 max_tokens={max_tokens} timeout_s=20 actions=0", manifest.anchor().revision, manifest.admissions().len());
    if let Some(row) = manifest.admissions().first() {
        eprintln!(
            "launcher_diagnostic admission_status={:?} attempts={}",
            row.status, row.launch_attempts
        );
    }
    match outcome {
        Ok(accepted) => {
            eprintln!("launcher_diagnostic=accepted_boundary await_actor={} model_text_bytes={:?}; raw_text_withheld", accepted.await_actor, accepted.model_text.as_ref().map(String::len));
            if max_tokens == 1024 {
                assert!(accepted.await_actor);
                assert!(
                    accepted
                        .model_text
                        .as_ref()
                        .is_some_and(|text| !text.is_empty()),
                    "general chat produced no textual answer"
                );
                assert_eq!(manifest.anchor().revision, 1);
                assert_eq!(manifest.admissions()[0].status, AdmissionStatus::Consumed);
                eprintln!("LAUNCHER_TEXT_PROOF=passed new_identity=true nonempty_answer=true boundary_accepted=true no_effects=true");
            }
        }
        Err(error) => {
            if let Some(failure) = error.downcast_ref::<NativeFailure>() {
                eprintln!("launcher_diagnostic=refused category={failure}; raw_details_withheld");
            } else {
                eprintln!(
                    "launcher_diagnostic=refused category=native_unknown; raw_details_withheld"
                );
            }
            assert_ne!(
                max_tokens, 1024,
                "general-chat acceptance failed; finite category above, no automatic retry"
            );
        }
    }
    // Test success means the approved observation completed, not agent-task acceptance.
}
#[test]
#[ignore = "explicit one-call live_provider_smoke diagnostic only; never ordinary/CI/owned-PTY suite"]
fn live_provider_smoke_go_one_call_diagnostic() {
    assert_eq!(
        std::env::var("RAIV_LIVE_SMOKE").ok().as_deref(),
        Some("opencode-go/deepseek-v4.1-flash/natural-one-call")
    );
    let mut home = Home::configured(|source| {
        source
            .replace("max_tokens = 1024", "max_tokens = 256")
            .replace("timeout_ms = 60000", "timeout_ms = 20000")
    });
    home.retain = true;
    eprintln!("retained_native_smoke_namespace={}", home.root.display());
    let selected = home.config.live.as_ref().unwrap();
    assert_eq!(selected.base_url, "https://opencode.ai/zen/go/v1");
    assert_eq!(selected.model, "deepseek-v4.1-flash");
    assert_eq!(selected.api_key_env, "OPENCODE_API_KEY");
    eprintln!("selection=opencode-go endpoint=https://opencode.ai/zen/go/v1/chat/completions model=deepseek-v4.1-flash session_header=native_activation cap=1 max_tokens=256 timeout_s=20");
    let definition = home.config.definition().unwrap();
    let (world_digest, source) = home.select();
    let paired = WorldRecoveryPin {
        source,
        world_digest,
    };
    let plan = GrantPlan {
        activation_units: 100,
        episode_units: 100,
        model_calls: 1,
        actions: 0,
        max_events: 1,
    };
    let (input,grant)=plan.input("Reply briefly with OK and no tool calls to finish. Do not patch Memory or request any action.",paired,[[0xe4;32],[0xf4;32]]).unwrap();
    let before = home.world();
    let mut real = Real {
        inner: LiveProvider {
            config: selected,
            client: None,
            failure: None,
        },
        posts: 0,
        cap: 1,
        diagnostic: true,
    };
    let outcome = execute_request(
        &home.config,
        &definition,
        &home.module,
        NativeRequest {
            paired,
            input,
            grant,
            cause: Cause::ActorWake,
            script: None,
            capture_continuation: true,
        },
        Some(&mut real),
    );
    assert_eq!(home.world(), before);
    let raw = home.main();
    let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
    eprintln!(
        "diagnostic_posts={} revision={} admission_status={:?} attempts={} world_unchanged=true",
        real.posts,
        manifest.anchor().revision,
        manifest.admissions()[0].status,
        manifest.admissions()[0].launch_attempts
    );
    assert_eq!(real.posts, 1);
    match outcome {
        Ok(accepted) => {
            assert!(accepted.await_actor);
            eprintln!("diagnostic=accepted_boundary raw_model_text_withheld=true");
        }
        Err(_) => panic!("diagnostic refused or incomplete; safe category above; no retry"),
    }
}
#[test]
#[ignore = "explicit live_provider_smoke opt-in only; never ordinary/CI/owned-PTY suite"]
fn live_provider_smoke_opencode_go_deepseek_flash_two_accepted_boundaries_and_pinned_memory() {
    assert_eq!(
        std::env::var("RAIV_LIVE_SMOKE").ok().as_deref(),
        Some("opencode-go/deepseek-v4.1-flash/natural-memory"),
        "explicit bounded native live smoke approval required"
    );
    assert!(
        std::env::var("OPENCODE_API_KEY").is_ok_and(|value| !value.is_empty()),
        "selected environment credential unavailable; value withheld"
    );
    let mandate="For this bounded native memory-continuity smoke, never yield trade/set_wake. On an ActorWake asking to remember a nickname, call patch_memory exactly once with intents.nickname equal to the requested text. On the following inference reply Finish with a normal complete response without tool calls. On later ActorWake asking the nickname, read accepted Memory and answer in your final reply text without tool calls or changing Memory. At most one enabled tool per reply; no inferred thinking.";
    let mut home = Home::configured(|source| {
        source.replace("Assist the native operator. Preserve relevant behavioral continuity in sparse Memory and explicitly finish each request.",mandate).replace("max_tokens = 1024","max_tokens = 256").replace("timeout_ms = 60000","timeout_ms = 20000")
    });
    home.retain = true;
    eprintln!("retained_native_smoke_namespace={}", home.root.display());
    let selected = home.config.live.as_ref().unwrap();
    assert_eq!(selected.model, "deepseek-v4.1-flash");
    assert_eq!(selected.base_url, "https://opencode.ai/zen/go/v1");
    assert_eq!(selected.api_key_env, "OPENCODE_API_KEY");
    assert!(selected.stream);
    assert_eq!(selected.max_tokens, 256);
    assert_eq!(selected.timeout, std::time::Duration::from_secs(20));
    let definition = home.config.definition().unwrap();
    let (world_digest, source) = home.select();
    let first_pin = WorldRecoveryPin {
        source,
        world_digest,
    };
    let world_before = home.world();
    let plan = GrantPlan {
        activation_units: 100,
        episode_units: 100,
        model_calls: 2,
        actions: 0,
        max_events: 2,
    };
    let (input, grant) = plan
        .input(
            "Remember my nickname ORION-RAIV-7 in intents.nickname and explicitly finish.",
            first_pin,
            [[0xe1; 32], [0xf1; 32]],
        )
        .unwrap();
    let mut real = Real {
        inner: LiveProvider {
            config: selected,
            client: None,
            failure: None,
        },
        posts: 0,
        cap: 3,
        diagnostic: false,
    };
    let first = execute_request(
        &home.config,
        &definition,
        &home.module,
        NativeRequest {
            paired: first_pin,
            input,
            grant,
            cause: Cause::ActorWake,
            script: None,
            capture_continuation: true,
        },
        Some(&mut real),
    );
    if first.is_err() {
        eprintln!(
            "live_memory_smoke=unaccepted posts={} cap=3; no relaunch",
            real.posts
        );
    }
    let first = first.unwrap();
    assert!(
        first.await_actor,
        "live provider chose pending action; do not execute/retry"
    );
    assert_eq!(
        real.posts, 2,
        "first boundary requires real patch then explicit Finish"
    );
    let snapshot = home.snapshot();
    let expected = ModelValue::encode(
        ValueRef::Map(&[(
            "intents",
            ValueRef::Map(&[("nickname", ValueRef::Text("ORION-RAIV-7"))]),
        )]),
        &Limits::default().decode_limits(),
    )
    .unwrap();
    assert_eq!(
        snapshot.memory().unwrap().value_bytes(),
        expected.as_bytes()
    );
    let memory = snapshot.memory().unwrap().cid();
    assert_eq!(snapshot.state().revision(), 1);
    drop(snapshot);
    let next = first
        .next
        .expect("known accepted first pin unavailable; no retry");
    assert_eq!(home.world(), world_before);
    eprintln!(
        "accepted_first_root={} memory_cid={} posts={} world_unchanged=true",
        next.source.anchor.root, memory, real.posts
    );
    // Drop live transport and all prior session owners; fresh source approval is test administrator action.
    drop(real);
    crate::app::interactive::verify_initial(&home.config, &definition, &home.module, next).unwrap();
    let (input, grant) = plan
        .input(
            "What is my stored nickname? Answer it in final reply text without tool calls, without changing Memory.",
            next,
            [[0xe2; 32], [0xf2; 32]],
        )
        .unwrap();
    let mut fresh = Real {
        inner: LiveProvider {
            config: selected,
            client: None,
            failure: None,
        },
        posts: 2,
        cap: 3,
        diagnostic: false,
    };
    let second = execute_request(
        &home.config,
        &definition,
        &home.module,
        NativeRequest {
            paired: next,
            input,
            grant,
            cause: Cause::ActorWake,
            script: None,
            capture_continuation: true,
        },
        Some(&mut fresh),
    )
    .unwrap();
    assert!(second.await_actor);
    assert_eq!(fresh.posts, 3);
    let snapshot = home.snapshot();
    assert_eq!(snapshot.memory().unwrap().cid(), memory);
    assert_eq!(snapshot.state().revision(), 2);
    assert_eq!(snapshot.history().count(), 2);
    assert_eq!(home.world(), world_before);
    eprintln!("live_structural_continuity=passed accepted_boundaries=2 sparse_memory_unchanged=true posts=3 world_unchanged=true behavior_answer_not_yet_proven=true");
    assert!(
        second
            .model_text
            .as_deref()
            .is_some_and(|text| text.contains("ORION-RAIV-7")),
        "live response did not carry nickname; raw response withheld"
    );
    eprintln!("live_smoke=passed provider=opencode-go model=deepseek-v4.1-flash transport=HTTPS/SSE posts=3 accepted_boundaries=2 sparse_memory_unchanged=true continuation_answer=true world_unchanged=true no_effects=true");
}
