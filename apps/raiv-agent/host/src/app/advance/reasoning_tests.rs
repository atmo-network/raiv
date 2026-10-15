//! Application lifecycle witnesses with synthetic provider thoughts; never network/credential access.

use super::*;

fn home(effort: &str) -> Home {
    Home::configured(|s| {
        s.replace(
            "[inference]",
            &format!("[inference]\nreasoning_effort = \"{effort}\""),
        )
    })
}

fn private_storage(path: &Path) {
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            private_storage(&entry.path());
        } else {
            assert!(kind.is_file());
            assert!(!fs::read(entry.path())
                .unwrap()
                .windows(19)
                .any(|v| v == b"NATIVE_ONLY_THOUGHT"));
        }
    }
}

#[test]
#[ignore = "actual WASM offline reasoning driver lifecycle; no network/key"]
fn actual_reasoning_application_patch_correction_finish_fresh_source_and_ui_do_not_restore_thought()
{
    for effort in ["low", "high"] {
        let mut home = home(effort);
        let world = home.world();
        let mut port = Mock::new([
            Frame::Call("patch_memory", r#"{"intents":{"name":"Ada"}}"#),
            Frame::Call("patch_memory", "[]"),
            Frame::Finish("Native mock reply."),
        ]);
        home.input(1, Cause::ActorWake, None, 3);
        port.sse = effort == "high";
        let result = home.drive(&mut port, Cause::ActorWake).unwrap();
        assert!(result.contains("model_replies=3"));
        assert!(port.saw_invalid_arguments);
        assert_eq!(port.calls, 3);
        let memory = home.snapshot().memory().unwrap().value_bytes().to_vec();
        assert_eq!(home.snapshot().state().revision(), 1);
        // Neither administrative reconfiguration nor a different effort may adopt this accepted source.
        home.select();
        let raw = home.main();
        let old = home.config.reasoning_effort;
        for changed in [
            None,
            Some(if effort == "high" {
                guest::ReasoningEffort::Low
            } else {
                guest::ReasoningEffort::High
            }),
        ] {
            home.config.reasoning_effort = changed;
            let mut rejected = Mock::new([Frame::Failure]);
            assert!(home.drive(&mut rejected, Cause::ActorWake).is_err());
            assert_eq!(rejected.prepared, 0);
            assert_eq!(home.main(), raw);
        }
        home.config.reasoning_effort = old;
        // Reusing the transport object cannot restore prior native reasoning: new invocation/new Run.
        home.input(2, Cause::ActorWake, None, 1);
        port.frames = VecDeque::from([Frame::Finish("Native mock reply.")]);
        port.sse = !port.sse;
        home.drive(&mut port, Cause::ActorWake).unwrap();
        assert_eq!(port.prepared, 2);
        assert_eq!(port.calls, 4);
        assert_eq!(home.snapshot().state().revision(), 2);
        assert_eq!(home.snapshot().memory().unwrap().value_bytes(), memory);
        assert_eq!(home.world(), world);
        private_storage(&home.root);
    }
    // Same production execute_request via actual Controller/TerminalUi, with fresh UI and known commit.
    interactive_continuation(home("high"));
}

#[test]
#[ignore = "actual WASM natural Finish from complete JSON/SSE no-call reply; no network/key"]
fn actual_natural_finish_commits_memory_and_text_once_with_no_finish_tool_call_or_reasoning_replay()
{
    for (effort, text, stream, thought) in [
        ("low", "Saved.", false, false),
        ("high", "", true, false),
        ("high", "Saved.", true, true),
    ] {
        let home = home(effort);
        let world = home.world();
        home.input(1, Cause::ActorWake, None, 2);
        let mut port = Mock::new([
            Frame::Call("patch_memory", r#"{"intents":{"name":"Ada"}}"#),
            if thought {
                Frame::Finish(text)
            } else {
                Frame::FinishWithoutReasoning(text)
            },
            Frame::Failure,
        ]);
        port.sse = stream;
        let (world_digest, source) = home.select();
        let accepted = execute_request(
            &home.config,
            &home.config.definition().unwrap(),
            &home.module,
            NativeRequest {
                paired: WorldRecoveryPin {
                    source,
                    world_digest,
                },
                input: ActivationInput::decode(
                    &fs::read(home.root.join("input.cbor")).unwrap(),
                    &Limits::default(),
                )
                .unwrap(),
                grant: BudgetAdmission::decode(
                    &fs::read(home.root.join("grant.cbor")).unwrap(),
                    &Limits::default().decode_limits(),
                )
                .unwrap(),
                cause: Cause::ActorWake,
                script: None,
                capture_continuation: true,
            },
            Some(&mut port),
        )
        .unwrap();
        assert!(accepted.await_actor);
        assert_eq!(
            accepted.model_text.as_deref(),
            Some(if text.is_empty() {
                "Native mock reply."
            } else {
                text
            })
        );
        assert_eq!(port.calls, 2);
        assert_eq!(port.frames.len(), 1);
        let snapshot = home.snapshot();
        assert_eq!(snapshot.state().revision(), 1);
        assert!(snapshot.current_episode().is_none());
        let boundary = snapshot
            .boundary(
                snapshot
                    .history()
                    .next()
                    .unwrap()
                    .boundaries()
                    .next()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(boundary.usage().model_calls, 2);
        assert_eq!(boundary.usage().local_tool_calls, 1);
        assert_eq!(
            snapshot.memory().unwrap().value_bytes(),
            ModelValue::encode(
                ValueRef::Map(&[("intents", ValueRef::Map(&[("name", ValueRef::Text("Ada"))]))]),
                &Limits::default().decode_limits()
            )
            .unwrap()
            .as_bytes()
        );
        assert_eq!(home.world(), world);
        private_storage(&home.root);
        home.select();
        home.input(2, Cause::ActorWake, None, 1);
        port.frames = VecDeque::from([Frame::FinishWithoutReasoning(""), Frame::Failure]);
        home.drive(&mut port, Cause::ActorWake).unwrap();
        assert_eq!(port.calls, 3);
        assert_eq!(home.snapshot().state().revision(), 2);
        assert_eq!(home.world(), world);
        private_storage(&home.root);
    }
}

#[test]
#[ignore = "actual WASM offline reasoning yield/resume; no network/key"]
fn actual_reasoning_resume_requires_installed_receipt_and_starts_without_previous_episode_thought()
{
    let home = home("high");
    let before = home.world();
    let mut port = Mock::new([
        Frame::Call("patch_memory", r#"{"intents":{"name":"Ada"}}"#),
        Frame::Call("trade", "{}"),
    ]);
    home.drive(&mut port, Cause::ActorWake).unwrap();
    assert_eq!(home.world(), before);
    let receipt = home.settle();
    let installed = home.world();
    home.select();
    home.input(2, Cause::ActionResult, Some(&receipt), 2);
    port.frames = VecDeque::from([
        Frame::Call("patch_memory", r#"{"strategy":{"status":"done"}}"#),
        Frame::Finish("Native mock reply."),
    ]);
    port.sse = true;
    home.drive(&mut port, Cause::ActionResult).unwrap();
    assert_eq!(port.prepared, 2);
    assert_eq!(port.calls, 4);
    assert_eq!(home.world(), installed);
    let snapshot = home.snapshot();
    assert_eq!(snapshot.state().revision(), 2);
    assert_eq!(snapshot.history().count(), 1);
    assert_eq!(snapshot.history().next().unwrap().boundary_count(), 2);
    private_storage(&home.root);
}

#[test]
#[ignore = "actual WASM offline aggregate reasoning retention bound; no network/key"]
fn actual_reasoning_aggregate_retention_overflow_discards_ram_patch_without_sixth_exposure() {
    let home = home("high");
    let before = home.main();
    let anchor = CheckpointManifest::decode(&before, &bounds())
        .unwrap()
        .anchor();
    let world = home.world();
    let mut frames = (0..5).map(|_| Frame::LongReasoning).collect::<Vec<_>>();
    frames.push(Frame::Failure);
    let mut port = Mock::new(frames);
    let error = home
        .drive(&mut port, Cause::ActorWake)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("reasoning continuation failed"));
    assert_eq!(port.calls, 5);
    assert_eq!(port.frames.len(), 1);
    let raw = home.main();
    let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
    assert_eq!(manifest.anchor(), anchor);
    assert_eq!(manifest.admissions()[0].status, AdmissionStatus::Launched);
    assert_eq!(home.snapshot().memory().unwrap().value_bytes(), b"\xa0");
    assert_eq!(home.world(), world);
    private_storage(&home.root);
}

#[test]
#[ignore = "actual WASM offline reasoning failure/bounds/no-relaunch; no network/key"]
fn actual_reasoning_missing_mode_stream_correlation_and_retention_failure_keep_launched_old_root() {
    for frame in [
        Frame::MissingReasoning,
        Frame::WrongId,
        Frame::Incomplete,
        Frame::Failure,
        Frame::OversizedReasoning,
    ] {
        let home = home("high");
        let before = home.main();
        let anchor = CheckpointManifest::decode(&before, &bounds())
            .unwrap()
            .anchor();
        let world = home.world();
        let mut prepare = Mock::new([Frame::Failure]);
        prepare.fail_prepare = true;
        assert!(home.drive(&mut prepare, Cause::ActorWake).is_err());
        assert_eq!(home.main(), before);
        assert_eq!(prepare.calls, 0);
        let missing = matches!(frame, Frame::MissingReasoning);
        let mut port = Mock::new([frame]);
        let error = home
            .drive(&mut port, Cause::ActorWake)
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("NATIVE_ONLY"));
        assert!(!error.contains("opaque-provider"));
        if missing {
            assert_eq!(port.refused, ["reply_mode"]);
        }
        assert_eq!(port.calls, 1);
        let raw = home.main();
        let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
        assert_eq!(manifest.anchor(), anchor);
        assert_eq!(manifest.admissions().len(), 1);
        assert_eq!(manifest.admissions()[0].status, AdmissionStatus::Launched);
        assert_eq!(manifest.admissions()[0].launch_attempts, 1);
        home.select();
        let mut retry = Mock::new([Frame::Failure]);
        assert!(home.drive(&mut retry, Cause::ActorWake).is_err());
        assert_eq!(retry.prepared, 0);
        assert_eq!(retry.calls, 0);
        assert_eq!(home.main(), raw);
        assert_eq!(home.world(), world);
        private_storage(&home.root);
    }
}

#[test]
#[ignore = "actual WASM offline full reasoning request cap; no network/key"]
fn actual_reasoning_combined_request_overflow_refuses_before_second_exposure() {
    let baseline = home("high");
    let mut port = Mock::new([
        Frame::Call("patch_memory", "{}"),
        Frame::Finish("Native mock reply."),
    ]);
    baseline.drive(&mut port, Cause::ActorWake).unwrap();
    assert!(port.request_sizes[1] > port.request_sizes[0]);
    let home = Home::configured(|s| {
        s.replace("[inference]", "[inference]\nreasoning_effort = \"high\"")
            .replace(
                "max_request_bytes = 262144",
                &format!("max_request_bytes = {}", port.request_sizes[0]),
            )
    });
    let before = home.main();
    let anchor = CheckpointManifest::decode(&before, &bounds())
        .unwrap()
        .anchor();
    let world = home.world();
    let mut limited = Mock::new([Frame::Call("patch_memory", "{}"), Frame::Failure]);
    assert!(home.drive(&mut limited, Cause::ActorWake).is_err());
    assert_eq!(limited.calls, 1);
    assert_eq!(limited.frames.len(), 1);
    let raw = home.main();
    let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
    assert_eq!(manifest.anchor(), anchor);
    assert_eq!(manifest.admissions()[0].status, AdmissionStatus::Launched);
    assert_eq!(home.world(), world);
    private_storage(&home.root);
}
