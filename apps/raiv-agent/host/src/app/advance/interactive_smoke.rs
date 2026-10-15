//! Offline actual-WASM semantic-recall witnesses; live interactive evidence is test-live-launcher.sh.

use super::*;

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
