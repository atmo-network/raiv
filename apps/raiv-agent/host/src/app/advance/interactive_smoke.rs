//! Opt-in real TTY/native-driver behavioral witness; raw terminal buffers must stay RAM-only.
use super::*;
use std::io::IsTerminal;
const MANDATE:&str="Perform the current operator's bounded memory task using the enabled tools. To remember a nickname, call patch_memory once to set intents.nickname to the given nickname, then finish with a normal complete response without tool calls on the next reply. For a RECALL_COPY task, read the existing accepted intents.nickname from Memory and call patch_memory once to set tactics.recalled_nickname to exactly that existing value, preserving intents. Then finish with a normal complete response without tool calls. Never invent a nickname, call a yield tool, or write transcript/reasoning. At most one tool per reply; no-tool completion preserves Memory. Ordinary assistant content is optional; the semantic Memory operation is the result of this task.";
fn configured(high_reasoning: bool) -> Home {
    Home::configured(|source| {
        let source = if high_reasoning {
            source.replace("[inference]", "[inference]\nreasoning_effort = \"high\"")
        } else {
            source
        };
        source.replace("Assist the native operator. Preserve relevant behavioral continuity in sparse Memory and explicitly finish each request.",MANDATE).replace("max_tokens = 1024","max_tokens = 256").replace("timeout_ms = 60000","timeout_ms = 20000")
    })
}
fn expected(recalled: bool) -> ModelValue {
    let name = ValueRef::Text("ORION-RAIV-7");
    let intent_fields = [("nickname", name)];
    let tactic_fields = [("recalled_nickname", name)];
    let intents = ValueRef::Map(&intent_fields);
    let tactics = ValueRef::Map(&tactic_fields);
    let bounds = Limits::default().decode_limits();
    if recalled {
        ModelValue::encode(
            ValueRef::Map(&[("intents", intents), ("tactics", tactics)]),
            &bounds,
        )
        .unwrap()
    } else {
        ModelValue::encode(ValueRef::Map(&[("intents", intents)]), &bounds).unwrap()
    }
}
fn checked(home: &Home, revision: u64, model_calls: u64) {
    let snapshot = home.snapshot();
    assert_eq!(snapshot.state().revision(), revision);
    assert!(matches!(
        snapshot.state().continuation(),
        raiv::state::Continuation::AwaitActor
    ));
    assert!(
        snapshot.memory().unwrap().value_bytes() == expected(revision == 2).as_bytes(),
        "semantic Memory mismatch; raw bytes withheld"
    );
    assert_eq!(snapshot.history().count(), revision as usize);
    for episode in snapshot.history() {
        assert_eq!(episode.boundary_count(), 1);
        let boundary = snapshot
            .boundary(episode.boundaries().next().unwrap())
            .unwrap();
        assert!((2..=model_calls).contains(&boundary.usage().model_calls));
        assert_eq!(boundary.usage().local_tool_calls, 1);
    }
    let raw = home.main();
    let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
    assert_eq!(manifest.admissions().len(), revision as usize);
    assert!(manifest
        .admissions()
        .iter()
        .all(|r| r.status == AdmissionStatus::Consumed && r.launch_attempts == 1));
}
#[test]
#[ignore = "actual WASM offline behavioral witness; no network/key/TTY required"]
fn actual_semantic_recall_two_event_envelope_has_exact_memory_no_transcript_or_effects() {
    offline_semantics(false, false);
}
#[test]
#[ignore = "actual WASM bounded corrective-turn witness; no network/key/TTY required"]
fn actual_semantic_recall_one_corrective_call_preserves_patch_and_rejects_bad_finish() {
    offline_semantics(true, false);
}
#[test]
#[ignore = "actual WASM high-reasoning semantic envelope; no network/key/TTY"]
fn actual_semantic_recall_high_reasoning_has_same_capped_envelope_without_thought_replay() {
    offline_semantics(true, true);
}
fn offline_semantics(corrective: bool, high_reasoning: bool) {
    let home = configured(high_reasoning);
    let world = home.world();
    let (world_digest, source) = home.select();
    let mut paired = WorldRecoveryPin {
        source,
        world_digest,
    };
    let plan = GrantPlan {
        activation_units: 100,
        episode_units: 100,
        model_calls: if corrective { 3 } else { 2 },
        actions: 0,
        max_events: 1,
    };
    for index in 1..=2 {
        let prompt = if index == 1 {
            "Remember nickname ORION-RAIV-7."
        } else {
            "RECALL_COPY: use the saved intents.nickname, then finish."
        };
        let (input, grant) = plan
            .input(prompt, paired, [[0xe6 + index; 32], [0xf6 + index; 32]])
            .unwrap();
        if index == 2 {
            assert!(!input
                .observation_bytes()
                .windows(12)
                .any(|v| v == b"ORION-RAIV-7"));
        }
        let args = if index == 1 {
            r#"{"intents":{"nickname":"ORION-RAIV-7"}}"#
        } else {
            r#"{"tactics":{"recalled_nickname":"ORION-RAIV-7"}}"#
        };
        let mut frames = vec![Frame::Call("patch_memory", args)];
        if corrective {
            frames.push(Frame::Call("patch_memory", "[]"));
        }
        frames.push(Frame::Finish("Native mock reply."));
        let mut provider = Mock::new(frames);
        let accepted = execute_request(
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
            Some(&mut provider),
        )
        .unwrap();
        assert_eq!(provider.calls, plan.model_calls as usize);
        assert_eq!(provider.saw_invalid_arguments, corrective);
        assert!(accepted.await_actor);
        paired = accepted.next.unwrap();
        checked(&home, index as u64, plan.model_calls);
        assert_eq!(home.world(), world);
    }
}
#[test]
#[ignore = "explicit live_provider_smoke plus owned PTY input fencing; never ordinary/CI suite"]
fn live_provider_smoke_interactive_memory_recall() {
    interactive_smoke(2, "opencode-go/deepseek-v4.1-flash/natural-recall", false);
}
#[test]
#[ignore = "explicit live_provider_smoke corrective-turn approval plus owned PTY; never CI"]
fn live_provider_smoke_interactive_memory_recall_with_correction() {
    interactive_smoke(
        3,
        "opencode-go/deepseek-v4.1-flash/natural-recall-correction",
        false,
    );
}
#[test]
#[ignore = "explicit live_provider_smoke high-reasoning approval plus owned PTY; never CI"]
fn live_provider_smoke_interactive_memory_recall_with_high_reasoning() {
    interactive_smoke(
        3,
        "opencode-go/deepseek-v4.1-flash/natural-recall-high",
        true,
    );
}
fn interactive_smoke(model_calls: u64, approval: &str, high_reasoning: bool) {
    assert_eq!(
        std::env::var("RAIV_LIVE_SMOKE").ok().as_deref(),
        Some(approval)
    );
    assert!(std::io::stdin().is_terminal() && std::io::stdout().is_terminal());
    let mut home = configured(high_reasoning);
    assert_eq!(
        home.config.reasoning_effort,
        high_reasoning.then_some(guest::ReasoningEffort::High)
    );
    home.retain = true;
    eprintln!("\nSMOKE_HOME={}", home.root.display());
    let selected = home.config.live.as_ref().unwrap();
    assert_eq!(selected.base_url, "https://opencode.ai/zen/go/v1");
    assert_eq!(selected.model, "deepseek-v4.1-flash");
    assert_eq!(selected.max_tokens, 256);
    assert_eq!(selected.timeout, std::time::Duration::from_secs(20));
    let plan = home.root.join("interactive-grants.toml");
    fs::write(&plan,format!("activation_units=\"100\"\nremaining_episode_units=\"100\"\nremaining_model_calls=\"{model_calls}\"\nremaining_actions=\"0\"\nmax_events=\"1\"\n")).unwrap();
    let world = home.world();
    let mut posts = 0;
    for stage in 1..=2 {
        // The test administrator selects a new known accepted source only after checking full prior outcome.
        let (world_digest, pin) = home.select();
        if stage == 2 {
            checked(&home, 1, model_calls);
        }
        crate::app::interactive::verify_initial(
            &home.config,
            &home.config.definition().unwrap(),
            &home.module,
            WorldRecoveryPin {
                source: pin,
                world_digest,
            },
        )
        .unwrap();
        eprintln!("SMOKE_STAGE={stage}");
        let mut real = Real {
            inner: LiveProvider {
                config: selected,
                client: None,
                failure: None,
            },
            posts,
            cap: model_calls * 2,
            diagnostic: false,
        };
        let result = crate::app::interactive::run_with(
            &home.config,
            &home.config.definition().unwrap(),
            &home.module,
            &plan,
            &home.root.join("pin.toml"),
            &hex(&world_digest),
            |request| {
                execute_request(
                    &home.config,
                    &home.config.definition()?,
                    &home.module,
                    request,
                    Some(&mut real),
                )
            },
        )
        .unwrap();
        let stage_calls = real.posts - posts;
        posts = real.posts;
        assert!((2..=model_calls).contains(&stage_calls));
        assert!(result.contains("interactive=closed"));
        checked(&home, stage, model_calls);
        assert_eq!(home.world(), world);
        eprintln!("\nSMOKE_PROOF stage={stage} accepted=true semantic_memory=true calls={stage_calls} world_unchanged=true");
    }
    let snapshot = home.snapshot();
    let last = snapshot.history().last().unwrap();
    let boundary = snapshot
        .boundary(last.boundaries().next().unwrap())
        .unwrap();
    let input = snapshot.input(boundary.input_link()).unwrap();
    assert!(
        !input
            .observation_bytes()
            .windows(12)
            .any(|v| v == b"ORION-RAIV-7"),
        "second prompt must not repeat the nickname"
    );
    eprintln!("\nSMOKE_RESULT=passed stages=2 calls={posts} independent_pinned_restart=true behavioral_recall=true no_effects=true");
}
