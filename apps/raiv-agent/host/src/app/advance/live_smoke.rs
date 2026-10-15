//! Opt-in real provider smoke; bounded native test administration, not effects/authentication proof.

use super::*;
use crate::app::interactive::GrantPlan;

#[path = "interactive_smoke.rs"]
mod interactive_smoke;

struct Real<'a> {
    inner: LiveProvider<'a>,
    posts: u64,
    cap: u64,
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
        let result = self.inner.complete(id, payload, limits);
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
