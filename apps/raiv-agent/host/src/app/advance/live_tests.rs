//! Trusted native provider-port witnesses, actual VM only; not live TLS/authentication proof.
use super::*;
use crate::app::initialize;
use raiv::{
    format::{DaslCid, DaslCodec},
    format::{ModelValue, ValueRef},
    graph::RestoredAgent,
    input::ActivationInputSpec,
    position::Position,
    receipt::Outcome,
};
use raiv_agent_host::{
    admission::AdmissionStatus,
    checkpoint::CheckpointManifest,
    inference::{normalize_chat_completion, sse::ChatCompletionStream},
    policy::current_pin,
    recovery::LocalRecoveryPin,
    simulator::world::{ScriptedResult, WorldGenesis},
};
use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "live_smoke.rs"]
mod live_smoke;
#[path = "reasoning_tests.rs"]
mod reasoning_tests;
#[test]
#[ignore = "actual WASM offline native diagnostic binding; no credential lookup/network"]
fn actual_native_provider_preflight_diagnostic_is_typed_and_preserves_checkpoint_and_world() {
    use crate::app::interactive::GrantPlan;
    let mut home = Home::new();
    // Invalid name refuses in data-only config validation, before lookup or transport setup.
    home.config.live.as_mut().unwrap().api_key_env.clear();
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
    let (input, grant) = plan.input("yp", paired, [[0xd1; 32], [0xd2; 32]]).unwrap();
    let main = home.main();
    let world = home.world();
    let result = execute_native(
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
    let Err(error) = result else {
        panic!("invalid configuration accepted")
    };
    let diagnostic = error
        .downcast_ref::<NativeFailure>()
        .expect("typed diagnostic");
    assert_eq!(
        diagnostic.to_string(),
        format!("provider: {}", ClientError::Configuration)
    );
    assert_eq!(home.main(), main);
    assert_eq!(home.world(), world);
    let manifest = CheckpointManifest::decode(&main, &bounds()).unwrap();
    assert!(manifest.admissions().is_empty());
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Home {
    root: PathBuf,
    config: Config,
    module: Vec<u8>,
    retain: bool,
}
impl Drop for Home {
    fn drop(&mut self) {
        if !self.retain {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
impl Home {
    fn new() -> Self {
        Self::configured(|source| source)
    }
    fn configured(configure: impl FnOnce(String) -> String) -> Self {
        let path = std::env::var_os("RAIV_GUEST_WASM").expect("actual module");
        let module = fs::read(&path).unwrap();
        let root = std::env::temp_dir().join(format!(
            "raiv-live-driver-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("agent")).unwrap();
        fs::create_dir(root.join("world")).unwrap();
        let digest = hex(&DaslCid::for_block(DaslCodec::Raw, &module).digest());
        let quoted = PathBuf::from(path)
            .to_str()
            .unwrap()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let source = include_str!("../../../../raiv-agent.opencode.toml")
            .replace(
                "../../target/wasm32v1-none/release/raiv_agent.wasm",
                &quoted,
            )
            .replace(&"00".repeat(32), &digest)
            .replace("./var/live-blocks", "agent")
            .replace("./var/live-world", "world");
        let source = configure(source);
        let file = root.join("config.toml");
        fs::write(&file, source).unwrap();
        let config = Config::load(&file).unwrap();
        let caller = Limits::default();
        let definition = config.definition().unwrap();
        let mut agent =
            DiskBlockStore::open_existing(&config.directory, caller.decode_limits()).unwrap();
        let (registry, book) = initialize(&config, &definition, &module, &mut agent).unwrap();
        let reg = registry.agent(config.agent).unwrap();
        let pin = current_pin(reg, &book, &mut agent, &bounds(), &caller).unwrap();
        let selected = config.world.as_ref().unwrap();
        let limits = limits(&config);
        let mut ws =
            DiskBlockStore::open_existing(&selected.directory, caller.decode_limits()).unwrap();
        ScriptedWorld::bootstrap(
            reg,
            &book,
            &mut agent,
            &mut ws,
            &WorldGenesis {
                world_id: selected.id,
                virtual_units: selected.units,
                capacity: selected.capacity,
                source: pin,
            },
            &limits,
        )
        .unwrap();
        drop(ws);
        drop(agent);
        let home = Self {
            root,
            config,
            module,
            retain: false,
        };
        home.select();
        home.input(1, Cause::ActorWake, None, 16);
        home
    }
    fn main(&self) -> Vec<u8> {
        fs::read(self.config.directory.join("raiv-manifest.cbor")).unwrap()
    }
    fn world(&self) -> Vec<u8> {
        fs::read(
            self.config
                .world
                .as_ref()
                .unwrap()
                .directory
                .join("raiv-world.cbor"),
        )
        .unwrap()
    }
    fn select(&self) -> ([u8; 32], LocalRecoveryPin) {
        let raw = self.main();
        let data = CheckpointManifest::decode(&raw, &bounds()).unwrap();
        let pin = LocalRecoveryPin {
            manifest_digest: DaslCid::for_block(DaslCodec::Raw, &raw).digest(),
            anchor: data.anchor(),
            program: data.program_binding(),
            resources: data.resources(),
        };
        fs::write(
            self.root.join("pin.toml"),
            format!(
                "root=\"{}\"\nrevision=\"{}\"\nmanifest_digest=\"{}\"\n",
                pin.anchor.root,
                pin.anchor.revision,
                hex(&pin.manifest_digest)
            ),
        )
        .unwrap();
        (
            DaslCid::for_block(DaslCodec::Raw, &self.world()).digest(),
            pin,
        )
    }
    fn input(&self, index: u8, cause: Cause, receipt: Option<&[u8]>, calls: u64) {
        let grant = BudgetAdmission {
            admission_id: [0x80 + index; 32],
            activation_units: 100,
            remaining_episode_units: 100,
            remaining_model_calls: calls,
            remaining_actions: 4,
        };
        let receipt =
            receipt.map(|bytes| raiv::receipt::Receipt::decode(bytes, &Limits::default()).unwrap());
        let input = ActivationInput::encode(
            &ActivationInputSpec {
                event_id: [0x90 + index; 32],
                cause,
                observed_at: Position {
                    domain: [1; 32],
                    number: index as u64,
                    reference: None,
                },
                observation: &[0xa0],
                receipt: receipt.as_ref(),
                budget_admission: grant,
            },
            &Limits::default(),
        )
        .unwrap();
        fs::write(self.root.join("input.cbor"), input.as_bytes()).unwrap();
        fs::write(
            self.root.join("grant.cbor"),
            grant.encode(&Limits::default().decode_limits()).unwrap(),
        )
        .unwrap();
    }
    fn drive(&self, port: &mut Mock, cause: Cause) -> Result<String> {
        let digest = hex(&DaslCid::for_block(DaslCodec::Raw, &self.world()).digest());
        execute_using(
            &self.config,
            &self.config.definition()?,
            &self.module,
            RequestFiles {
                event: &self.root.join("input.cbor"),
                grant: &self.root.join("grant.cbor"),
                replies: None,
                pin: &self.root.join("pin.toml"),
                world_digest: &digest,
            },
            cause,
            Some(port),
        )
    }
    fn snapshot(&self) -> RestoredAgent {
        let raw = self.main();
        let data = CheckpointManifest::decode(&raw, &bounds()).unwrap();
        let store = DiskBlockStore::open_existing(
            &self.config.directory,
            Limits::default().decode_limits(),
        )
        .unwrap();
        data.restore(&store, &Limits::default()).unwrap()
    }
    fn settle(&self) -> Vec<u8> {
        let (digest, pin) = self.select();
        let caller = Limits::default();
        let limits = limits(&self.config);
        let mut registry = Registry::new(1).unwrap();
        let mut book = AdmissionBook::new(1024).unwrap();
        let mut agent =
            DiskBlockStore::open_existing(&self.config.directory, caller.decode_limits()).unwrap();
        let mut store = DiskBlockStore::open_existing(
            &self.config.world.as_ref().unwrap().directory,
            caller.decode_limits(),
        )
        .unwrap();
        adopt_local(
            &self.module,
            &mut agent,
            &mut registry,
            &mut book,
            pin,
            &bounds(),
            &caller,
        )
        .unwrap();
        let reg = registry.agent(self.config.agent).unwrap();
        let mut world = ScriptedWorld::adopt(
            reg,
            &book,
            &mut agent,
            &mut store,
            self.config.world.as_ref().unwrap().capacity,
            &WorldRecoveryPin {
                source: pin,
                world_digest: digest,
            },
            &limits,
        )
        .unwrap();
        let ticket = world
            .authorize_pending(reg, &book, &mut agent, &limits)
            .unwrap();
        let id = pending(reg.snapshot(), &caller).unwrap().action_id();
        world
            .begin(ticket, reg, &book, &mut agent, &limits)
            .unwrap();
        world
            .settle(
                ticket,
                reg,
                &book,
                &mut agent,
                ScriptedResult {
                    outcome: Outcome::Succeeded,
                    result: &[0xa0],
                    debit: 1,
                },
                &limits,
            )
            .unwrap();
        world.state().ledger().receipt_bytes(id).unwrap().to_vec()
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn limits(config: &Config) -> LocalExecutorLimits {
    LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: Limits::default().decode_limits(),
            max_actions: config.world.as_ref().unwrap().capacity as u64,
        },
        manifest: bounds(),
        caller: Limits::default(),
    }
}
enum Frame {
    Finish(&'static str),
    FinishWithoutReasoning(&'static str),
    Call(&'static str, &'static str),
    Failure,
    Reasoning,
    MissingReasoning,
    OversizedReasoning,
    LongReasoning,
    WrongId,
    Incomplete,
}
struct Mock {
    frames: VecDeque<Frame>,
    prepared: usize,
    calls: usize,
    fail_prepare: bool,
    sse: bool,
    refused: Vec<&'static str>,
    empty_finish_text: bool,
    saw_invalid_arguments: bool,
    reasoning_effort: Option<guest::ReasoningEffort>,
    thoughts: Vec<String>,
    request_sizes: Vec<usize>,
}
impl Mock {
    fn new(frames: impl IntoIterator<Item = Frame>) -> Self {
        Self {
            frames: frames.into_iter().collect(),
            prepared: 0,
            calls: 0,
            fail_prepare: false,
            sse: false,
            refused: Vec::new(),
            empty_finish_text: false,
            saw_invalid_arguments: false,
            reasoning_effort: None,
            thoughts: Vec::new(),
            request_sizes: Vec::new(),
        }
    }
}
impl ProviderPort for Mock {
    fn guest_refused(&mut self, category: &'static str) {
        self.refused.push(category);
    }
    fn prepare(&mut self, definition: &Definition, input: &ActivationInput) -> Result<()> {
        self.prepared += 1;
        self.reasoning_effort = guest::ChatProfile::decode(definition.inference_profile_bytes())
            .unwrap()
            .reasoning_effort;
        self.thoughts.clear();
        self.saw_invalid_arguments = false;
        assert!(matches!(
            input.cause(),
            Cause::ActorWake | Cause::ActionResult
        ));
        if self.fail_prepare {
            return Err(fail("mock missing credential before durable reservation"));
        }
        Ok(())
    }
    fn complete(&mut self, id: u64, payload: &[u8], caller: &Limits) -> Result<NormalizedReply> {
        self.calls += 1;
        self.request_sizes.push(payload.len());
        let body: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(body["model"], "deepseek-v4.1-flash");
        let assistants = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "assistant")
            .collect::<Vec<_>>();
        if let Some(effort) = self.reasoning_effort {
            assert_eq!(body["thinking"], serde_json::json!({"type":"enabled"}));
            assert_eq!(body["reasoning_effort"], effort.as_str());
            assert_eq!(assistants.len(), self.thoughts.len());
            for (message, thought) in assistants.iter().zip(&self.thoughts) {
                assert_eq!(message["reasoning_content"], thought.as_str());
            }
        } else {
            assert_eq!(body["thinking"], serde_json::json!({"type":"disabled"}));
            assert!(body.get("reasoning_effort").is_none());
            assert!(assistants
                .iter()
                .all(|m| m.get("reasoning_content").is_none()));
        }
        assert!(!body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("NATIVE_ONLY_THOUGHT"));
        assert_eq!(body["messages"][0]["role"], "system");
        let data: serde_json::Value =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(data.as_array().unwrap().len(), 6);
        assert_eq!(data[5]["kind"], "ram_trajectory");
        self.saw_invalid_arguments |= data[5]["entries"]
            .as_array()
            .and_then(|entries| entries.last())
            .is_some_and(|turn| turn["error"] == "invalid_arguments");
        if self.saw_invalid_arguments {
            let last = body["messages"].as_array().unwrap().last().unwrap();
            assert_eq!(last["role"], "tool");
            let result: serde_json::Value =
                serde_json::from_str(last["content"].as_str().unwrap()).unwrap();
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], "invalid_arguments");
        }
        assert_eq!(body["parallel_tool_calls"], false);
        let frame = self
            .frames
            .pop_front()
            .expect("no provider exposure after boundary/failure");
        let final_text = match &frame {
            Frame::Finish(text) | Frame::FinishWithoutReasoning(text) => Some(*text),
            Frame::Reasoning | Frame::WrongId => Some("Native mock reply."),
            _ => None,
        };
        let oversized = matches!(frame, Frame::OversizedReasoning);
        let thought = self
            .reasoning_effort
            .map(|_| format!("NATIVE_ONLY_THOUGHT_{}_{}", self.prepared, self.calls));
        let (tool, args, reason, reply_id) = match frame {
            Frame::Call(t, a) => (t, a, thought, id),
            Frame::Finish(_) => ("", "", thought, id),
            Frame::FinishWithoutReasoning(_) => ("", "", None, id),
            Frame::Reasoning => ("", "{}", Some("unexpected thinking".to_owned()), id),
            Frame::MissingReasoning => ("patch_memory", "{}", None, id),
            Frame::OversizedReasoning => (
                "patch_memory",
                "{}",
                Some("NATIVE_ONLY_THOUGHT".repeat(caller.max_model_reply_bytes as usize / 18 + 1)),
                id,
            ),
            Frame::LongReasoning => (
                "patch_memory",
                r#"{"intents":{"name":"Ada"}}"#,
                Some(format!("NATIVE_ONLY_THOUGHT{}", "r".repeat(30000))),
                id,
            ),
            Frame::WrongId => ("", "{}", thought, id + 100),
            Frame::Failure => return Err(fail("opaque-provider-secret-body")),
            Frame::Incomplete => {
                let mut s = ChatCompletionStream::new(
                    id,
                    "deepseek-v4.1-flash",
                    *caller,
                    self_config_stream(),
                )
                .unwrap();
                s.push(b"data: [DONE]\n\n").unwrap_err();
                return Err(fail("incomplete-stream-provider-body"));
            }
        };
        let mut response = serde_json::json!({"model":"deepseek-v4.1-flash","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":"Native mock reply.","tool_calls":[{"id":"call-live","type":"function","function":{"name":tool,"arguments":args}}]}}]});
        if let Some(text) = final_text {
            response["choices"][0]["finish_reason"] = serde_json::json!("stop");
            response["choices"][0]["message"]["tool_calls"] = serde_json::json!([]);
            response["choices"][0]["message"]["content"] = serde_json::json!(text);
        }
        if self.empty_finish_text && final_text.is_some() {
            response["choices"][0]["message"]["content"] = serde_json::Value::Null;
        }
        if let Some(reason) = reason {
            response["choices"][0]["message"]["reasoning_content"] = serde_json::json!(reason);
            self.thoughts.push(reason);
        }
        // Adversarial test port can normalize under wider bounds; production HTTP never does.
        let reply_limits = if oversized {
            Limits {
                max_model_reply_bytes: caller.max_model_reply_bytes * 2,
                ..*caller
            }
        } else {
            *caller
        };
        if self.sse {
            let mut delta = response["choices"][0]["message"].clone();
            if final_text.is_none() {
                delta["tool_calls"][0]["index"] = serde_json::json!(0);
            }
            let chunk = serde_json::json!({"model":"deepseek-v4.1-flash","choices":[{"index":0,"finish_reason":if final_text.is_some(){"stop"}else{"tool_calls"},"delta":delta}]});
            let wire = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                serde_json::to_string(&chunk).unwrap()
            );
            let mut s = ChatCompletionStream::new(
                reply_id,
                "deepseek-v4.1-flash",
                reply_limits,
                self_config_stream(),
            )
            .unwrap();
            s.push(wire.as_bytes()).unwrap();
            return Ok(s.finish().unwrap());
        }
        Ok(normalize_chat_completion(
            &serde_json::to_vec(&response).unwrap(),
            reply_id,
            "deepseek-v4.1-flash",
            &reply_limits,
        )?)
    }
}
fn self_config_stream() -> raiv_agent_host::inference::sse::StreamLimits {
    raiv_agent_host::inference::sse::StreamLimits {
        max_wire_bytes: 1048576,
        max_event_bytes: 65536,
        max_events: 4096,
    }
}
#[test]
#[ignore = "owned-PTY interactive exit witness: use script with selected actual guest and ESC input"]
fn owned_tty_interactive_command_exit_without_submit_never_admits_or_calls_provider() {
    use std::ffi::OsString;
    use std::io::IsTerminal;
    assert!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "explicitly owned PTY only"
    );
    let home = Home::new();
    let before = home.main();
    let world = home.world();
    let (digest, _) = home.select();
    let grants = home.root.join("grants.toml");
    fs::write(
        &grants,
        include_str!("../../../../raiv-agent.interactive-grants.toml"),
    )
    .unwrap();
    let result = crate::app::run(vec![
        OsString::from("interactive"),
        OsString::from("--grants"),
        grants.into_os_string(),
        OsString::from("--pin"),
        home.root.join("pin.toml").into_os_string(),
        OsString::from("--world-pin"),
        OsString::from(hex(&digest)),
        OsString::from("--config"),
        home.root.join("config.toml").into_os_string(),
    ])
    .unwrap();
    assert!(result.contains("interactive=closed"));
    assert_eq!(home.main(), before);
    assert_eq!(home.world(), world);
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_interactive_controller_admits_only_current_prompt_and_approved_known_continuation_without_transcript_or_effect_replay(
) {
    interactive_continuation(Home::new());
}
fn interactive_continuation(home: Home) {
    use crate::app::interactive::{verify_initial, Controller, GrantPlan};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use raiv_agent_host::terminal::{Signal, TerminalUi, UiLimits};
    fn submit(ui: &mut TerminalUi, text: &str) {
        ui.handle(Event::Paste(text.into())).unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal.draw(|f| ui.render(f)).unwrap();
        assert_eq!(
            ui.handle(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            )))
            .unwrap(),
            Signal::Submit
        );
    }
    let plan = GrantPlan {
        activation_units: 100,
        episode_units: 100,
        model_calls: 16,
        actions: 4,
        max_events: 4,
    };
    let (digest, pin) = home.select();
    let paired = WorldRecoveryPin {
        source: pin,
        world_digest: digest,
    };
    verify_initial(
        &home.config,
        &home.config.definition().unwrap(),
        &home.module,
        paired,
    )
    .unwrap();
    let mut controller = Controller {
        paired,
        remaining: plan.max_events,
        blocked: false,
    };
    let mut ui = TerminalUi::new(UiLimits {
        prompt_bytes: 128,
        history_bytes: 131072,
        history_entries: 32,
    })
    .unwrap();
    submit(&mut ui, "Remember my name Ada.");
    let mut mock = Mock::new([
        Frame::Call("patch_memory", r#"{"intents":{"name":"Ada"}}"#),
        Frame::Finish("Native mock reply."),
    ]);
    let before = home.world();
    controller
        .submit(&mut ui, &plan, [[0xa1; 32], [0xb1; 32]], |request| {
            assert_eq!(request.input.cause(), Cause::ActorWake);
            assert!(request
                .input
                .observation_bytes()
                .windows(21)
                .any(|v| v == b"Remember my name Ada."));
            execute_request(
                &home.config,
                &home.config.definition()?,
                &home.module,
                request,
                Some(&mut mock),
            )
        })
        .unwrap();
    assert!(!controller.blocked);
    assert_eq!(controller.remaining, 3);
    assert_eq!(controller.paired.source.anchor.revision, 1);
    assert!(ui.prompt().is_empty());
    assert_eq!(home.world(), before);
    let approved = controller.paired;
    verify_initial(
        &home.config,
        &home.config.definition().unwrap(),
        &home.module,
        approved,
    )
    .unwrap();
    // Fresh native/UI instance: approved pin, no persistent UI history/prompt restoration.
    let mut fresh = Controller {
        paired: approved,
        remaining: plan.max_events,
        blocked: false,
    };
    let mut restarted = TerminalUi::new(UiLimits {
        prompt_bytes: 128,
        history_bytes: 131072,
        history_entries: 32,
    })
    .unwrap();
    assert!(restarted.prompt().is_empty());
    submit(&mut restarted, "Who am I?");
    let mut second = Mock::new([Frame::Finish("Native mock reply.")]);
    fresh
        .submit(&mut restarted, &plan, [[0xa2; 32], [0xb2; 32]], |request| {
            let expected = ModelValue::encode(
                ValueRef::Map(&[("text", ValueRef::Text("Who am I?"))]),
                &Limits::default().decode_limits(),
            )
            .unwrap();
            assert_eq!(request.input.observation_bytes(), expected.as_bytes());
            execute_request(
                &home.config,
                &home.config.definition()?,
                &home.module,
                request,
                Some(&mut second),
            )
        })
        .unwrap();
    assert!(!fresh.blocked);
    assert_eq!(fresh.paired.source.anchor.revision, 2);
    assert_eq!(home.world(), before);
    let snapshot = home.snapshot();
    let expected = ModelValue::encode(
        ValueRef::Map(&[("intents", ValueRef::Map(&[("name", ValueRef::Text("Ada"))]))]),
        &Limits::default().decode_limits(),
    )
    .unwrap();
    assert_eq!(
        snapshot.memory().unwrap().value_bytes(),
        expected.as_bytes()
    );
    assert_eq!(snapshot.history().count(), 2);
    // Known acceptance with missing next pin must not become retryable failed submission.
    submit(&mut restarted, "Third prompt.");
    let mut third = Mock::new([Frame::Finish("Native mock reply.")]);
    fresh
        .submit(&mut restarted, &plan, [[0xa3; 32], [0xb3; 32]], |request| {
            let mut accepted = execute_request(
                &home.config,
                &home.config.definition()?,
                &home.module,
                request,
                Some(&mut third),
            )?;
            accepted.next = None;
            Ok(accepted)
        })
        .unwrap();
    assert!(fresh.blocked);
    assert!(restarted.prompt().is_empty());
    assert_eq!(home.snapshot().state().revision(), 3);
    assert_eq!(home.world(), before);
    assert_eq!(
        restarted
            .handle(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            )))
            .unwrap(),
        Signal::None
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| restarted.render(frame)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Native mock reply."));
    assert!(!rendered.contains("NATIVE_ONLY_THOUGHT"));
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_interactive_controller_unknown_attempt_or_pending_action_is_blocked_never_relaunches() {
    use crate::app::interactive::{verify_initial, Controller, GrantPlan};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use raiv_agent_host::terminal::{Signal, TerminalUi, UiLimits};
    for frame in [Frame::Failure, Frame::Call("trade", "{}")] {
        let home = Home::new();
        let (digest, pin) = home.select();
        let paired = WorldRecoveryPin {
            source: pin,
            world_digest: digest,
        };
        let plan = GrantPlan {
            activation_units: 100,
            episode_units: 100,
            model_calls: 16,
            actions: 4,
            max_events: 2,
        };
        let mut controller = Controller {
            paired,
            remaining: 2,
            blocked: false,
        };
        let mut ui = TerminalUi::new(UiLimits {
            prompt_bytes: 128,
            history_bytes: 131072,
            history_entries: 32,
        })
        .unwrap();
        ui.handle(Event::Paste("current prompt".into())).unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal.draw(|f| ui.render(f)).unwrap();
        assert_eq!(
            ui.handle(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            )))
            .unwrap(),
            Signal::Submit
        );
        let world = home.world();
        let mut mock = Mock::new([frame]);
        controller
            .submit(&mut ui, &plan, [[0xc1; 32], [0xd1; 32]], |request| {
                execute_request(
                    &home.config,
                    &home.config.definition()?,
                    &home.module,
                    request,
                    Some(&mut mock),
                )
            })
            .unwrap();
        assert!(controller.blocked);
        assert_eq!(mock.calls, 1);
        assert_eq!(home.world(), world);
        assert_eq!(
            ui.handle(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            )))
            .unwrap(),
            Signal::None
        );
        let (digest, pin) = home.select();
        assert!(verify_initial(
            &home.config,
            &home.config.definition().unwrap(),
            &home.module,
            WorldRecoveryPin {
                source: pin,
                world_digest: digest
            }
        )
        .is_err());
    }
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_bounded_memory_two_call_envelope_matches_live_smoke_without_network() {
    let home = Home::new();
    let (world_digest, source) = home.select();
    let paired = WorldRecoveryPin {
        source,
        world_digest,
    };
    let plan = crate::app::interactive::GrantPlan {
        activation_units: 100,
        episode_units: 100,
        model_calls: 2,
        actions: 0,
        max_events: 1,
    };
    let (input, grant) = plan
        .input(
            "Remember nickname ORION-RAIV-7 and finish.",
            paired,
            [[0xe5; 32], [0xf5; 32]],
        )
        .unwrap();
    let mut mock = Mock::new([
        Frame::Call("patch_memory", r#"{"intents":{"nickname":"ORION-RAIV-7"}}"#),
        Frame::Finish("Native mock reply."),
    ]);
    mock.empty_finish_text = true;
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
        Some(&mut mock),
    )
    .unwrap();
    assert!(accepted.await_actor);
    assert_eq!(accepted.model_text.as_deref(), Some("Native mock reply."));
    assert_eq!(mock.calls, 2);
    assert_eq!(home.snapshot().state().revision(), 1);
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_live_driver_sparse_patch_yield_installed_ledger_resume_fresh_wake_without_effect_replay()
{
    let home = Home::new();
    let mut first = Mock::new([
        Frame::Call("patch_memory", r#"{"intents":{"name":"Ada"}}"#),
        Frame::Call("trade", "{}"),
        Frame::Failure,
    ]);
    let world_before = home.world();
    let output = home.drive(&mut first, Cause::ActorWake).unwrap();
    assert!(output.contains("mode=live-accepted-boundary"));
    assert_eq!(first.calls, 2);
    assert_eq!(first.frames.len(), 1);
    assert_eq!(home.world(), world_before);
    let snapshot = home.snapshot();
    let episode_id = snapshot.current_episode().unwrap().id();
    assert!(matches!(
        snapshot.state().continuation(),
        raiv::state::Continuation::AwaitAction { .. }
    ));
    assert_eq!(snapshot.state().revision(), 1);
    drop(snapshot);
    home.select();
    home.input(2, Cause::ActorWake, None, 16);
    let before = home.main();
    let mut reject = Mock::new([Frame::Failure]);
    assert!(home.drive(&mut reject, Cause::ActorWake).is_err());
    assert_eq!(reject.prepared, 0);
    assert_eq!(home.main(), before);
    let receipt = home.settle();
    let installed = home.world();
    home.select();
    let prior = raiv::receipt::Receipt::decode(&receipt, &Limits::default()).unwrap();
    let forged = raiv::receipt::Receipt::encode(
        &raiv::receipt::ReceiptSpec {
            action_id: prior.action_id(),
            command_digest: prior.command_digest(),
            outcome: prior.outcome(),
            result: b"\xa1\x61x\x01",
            execution_reference: None,
            finalized_at: prior.finalized_at(),
            costs: prior.costs_bytes(),
        },
        &Limits::default(),
    )
    .unwrap();
    home.input(2, Cause::ActionResult, Some(forged.as_bytes()), 16);
    let before = home.main();
    let mut rejected = Mock::new([Frame::Failure]);
    assert!(home.drive(&mut rejected, Cause::ActionResult).is_err());
    assert_eq!(rejected.prepared, 0);
    assert_eq!(home.main(), before);
    assert_eq!(home.world(), installed);
    home.input(2, Cause::ActionResult, Some(&receipt), 16);
    let mut unprepared = Mock::new([Frame::Failure]);
    unprepared.fail_prepare = true;
    assert!(home.drive(&mut unprepared, Cause::ActionResult).is_err());
    assert_eq!(unprepared.calls, 0);
    assert_eq!(home.main(), before);
    assert_eq!(home.world(), installed);
    let mut second = Mock::new([
        Frame::Call("patch_memory", r#"{"strategy":{"status":"done"}}"#),
        Frame::Finish("Native mock reply."),
        Frame::Failure,
    ]);
    second.sse = true;
    assert!(home
        .drive(&mut second, Cause::ActionResult)
        .unwrap()
        .contains("model_replies=2"));
    assert_eq!(second.calls, 2);
    assert_eq!(home.world(), installed);
    let snapshot = home.snapshot();
    assert_eq!(snapshot.state().revision(), 2);
    assert_eq!(snapshot.state().current_episode_link(), None);
    let expected = ModelValue::encode(
        ValueRef::Map(&[
            ("intents", ValueRef::Map(&[("name", ValueRef::Text("Ada"))])),
            (
                "strategy",
                ValueRef::Map(&[("status", ValueRef::Text("done"))]),
            ),
        ]),
        &Limits::default().decode_limits(),
    )
    .unwrap();
    assert_eq!(
        snapshot.memory().unwrap().value_bytes(),
        expected.as_bytes()
    );
    assert_eq!(snapshot.history().next().unwrap().id(), episode_id);
    assert_eq!(snapshot.history().next().unwrap().boundary_count(), 2);
    drop(snapshot);
    home.select();
    home.input(3, Cause::ActorWake, None, 16);
    let mut third = Mock::new([Frame::Finish("Native mock reply."), Frame::Failure]);
    home.drive(&mut third, Cause::ActorWake).unwrap();
    assert_eq!(third.calls, 1);
    assert_eq!(home.world(), installed);
    assert_eq!(
        home.snapshot().memory().unwrap().value_bytes(),
        expected.as_bytes()
    );
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn actual_live_failures_keep_launched_no_boundary_no_relaunch_and_prepare_failure_is_unattempted() {
    for frame in [
        Frame::Failure,
        Frame::Reasoning,
        Frame::WrongId,
        Frame::Incomplete,
        Frame::Call("unknown-tool", "{}"),
        Frame::Call("no_op", r#"{"reason":"x"}"#),
    ] {
        let home = Home::new();
        let old = home.main();
        let world = home.world();
        let old_anchor = CheckpointManifest::decode(&old, &bounds())
            .unwrap()
            .anchor();
        let mut prepare = Mock::new([Frame::Failure]);
        prepare.fail_prepare = true;
        assert!(home.drive(&mut prepare, Cause::ActorWake).is_err());
        assert_eq!(home.main(), old);
        assert_eq!(home.world(), world);
        assert_eq!(prepare.calls, 0);
        let wrong_id = matches!(frame, Frame::WrongId);
        let wrong_mode = matches!(frame, Frame::Reasoning);
        let bad = matches!(frame, Frame::Call(_, _));
        if bad {
            home.input(1, Cause::ActorWake, None, 1);
        }
        let mut mock = Mock::new([frame]);
        let error = home
            .drive(&mut mock, Cause::ActorWake)
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("opaque-provider-secret-body"));
        assert!(!error.contains("incomplete-stream-provider-body"));
        if wrong_id {
            assert_eq!(mock.refused, ["request_correlation"]);
        } else if bad {
            assert_eq!(mock.refused, ["guest_failure", "guest_activation_failed"]);
        } else if wrong_mode {
            assert_eq!(mock.refused, ["reply_mode"]);
        } else {
            assert!(mock.refused.is_empty());
        }
        assert_eq!(mock.calls, 1);
        assert_eq!(home.world(), world);
        let raw = home.main();
        let manifest = CheckpointManifest::decode(&raw, &bounds()).unwrap();
        assert_eq!(manifest.anchor(), old_anchor);
        assert_eq!(manifest.admissions().len(), 1);
        assert_eq!(manifest.admissions()[0].status, AdmissionStatus::Launched);
        assert_eq!(manifest.admissions()[0].launch_attempts, 1);
        home.select();
        let mut retry = Mock::new([Frame::Failure]);
        assert!(home.drive(&mut retry, Cause::ActorWake).is_err());
        assert_eq!(retry.prepared, 0);
        assert_eq!(retry.calls, 0);
        assert_eq!(home.main(), raw);
    }
}
