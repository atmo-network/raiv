//! Native terminal operator admission. Not external Actor authentication or an effect executor.
use super::{
    advance::{self, Accepted, NativeRequest},
    bounds,
    config::{decimal, id, Config},
    fail, read_bounded,
    world::selected_pin,
    Result,
};
use raiv::{
    budget::BudgetAdmission,
    definition::Definition,
    format::{DecodeLimits, ModelValue, ValueRef},
    input::{ActivationInput, ActivationInputSpec, Cause},
    limits::Limits,
    position::Position,
    state::Continuation,
};
use raiv_agent_host::{
    admission::{AdmissionBook, AdmissionStatus},
    checkpoint::CheckpointManifest,
    disk::DiskBlockStore,
    recovery::adopt_local,
    registry::Registry,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{ScriptedWorld, WorldRecoveryPin},
    },
    terminal::{Role, Signal, TerminalSession, TerminalUi, UiLimits},
};
use std::{path::Path, str::FromStr, time::Duration};
pub(super) struct GrantPlan {
    pub activation_units: u64,
    pub episode_units: u64,
    pub model_calls: u64,
    pub actions: u64,
    pub max_events: usize,
}
impl GrantPlan {
    fn load(path: &Path) -> Result<Self> {
        let raw = read_bounded(path, 4096)?;
        let text = std::str::from_utf8(&raw).map_err(|_| fail("grant plan UTF-8"))?;
        Self::parse(text)
    }
    fn parse(text: &str) -> Result<Self> {
        let table = toml::Table::from_str(text).map_err(|_| fail("invalid grant plan TOML"))?;
        if table.len() != 5
            || [
                "activation_units",
                "remaining_episode_units",
                "remaining_model_calls",
                "remaining_actions",
                "max_events",
            ]
            .iter()
            .any(|k| !table.contains_key(*k))
        {
            return Err(fail(
                "grant plan requires exactly five native approval fields",
            ));
        }
        let value =
            |key: &str| -> Result<u64> {
                decimal(table[key].as_str().ok_or_else(|| {
                    fail("grant plan fields require canonical unsigned decimal text")
                })?)
            };
        let plan = Self {
            activation_units: value("activation_units")?,
            episode_units: value("remaining_episode_units")?,
            model_calls: value("remaining_model_calls")?,
            actions: value("remaining_actions")?,
            max_events: usize::try_from(value("max_events")?)?,
        };
        if plan.activation_units == 0
            || plan.episode_units == 0
            || plan.model_calls == 0
            || plan.max_events == 0
            || plan.max_events > 1024
        {
            return Err(fail("interactive finite grant plan bounds invalid"));
        }
        Ok(plan)
    }
    pub(super) fn input(
        &self,
        text: &str,
        source: WorldRecoveryPin,
        ids: [[u8; 32]; 2],
    ) -> Result<(ActivationInput, BudgetAdmission)> {
        let caller = Limits::default();
        let observation = ModelValue::encode(
            ValueRef::Map(&[("text", ValueRef::Text(text))]),
            &DecodeLimits {
                max_block_bytes: caller.max_observation_bytes,
                ..caller.decode_limits()
            },
        )?;
        let grant = BudgetAdmission {
            admission_id: ids[1],
            activation_units: self.activation_units,
            remaining_episode_units: self.episode_units,
            remaining_model_calls: self.model_calls,
            remaining_actions: self.actions,
        };
        let input = ActivationInput::encode(
            &ActivationInputSpec {
                event_id: ids[0],
                cause: Cause::ActorWake,
                observed_at: Position {
                    domain: source.source.program.execution_domain,
                    number: source
                        .source
                        .anchor
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| fail("native observed ordinal overflow"))?,
                    reference: None,
                },
                observation: observation.as_bytes(),
                receipt: None,
                budget_admission: grant,
            },
            &caller,
        )?;
        Ok((input, grant))
    }
}
pub(super) fn random_ids() -> Result<[[u8; 32]; 2]> {
    let mut bytes = [0u8; 64];
    let mut at = 0;
    while at < bytes.len() {
        // SAFETY: native Linux randomness writes only the remaining live stack byte slice.
        let n = unsafe { libc::getrandom(bytes[at..].as_mut_ptr().cast(), bytes.len() - at, 0) };
        if n < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(fail("native fresh event/grant randomness unavailable"));
        }
        if n == 0 {
            return Err(fail("native fresh event/grant randomness unavailable"));
        }
        at += usize::try_from(n)?;
    }
    let mut ids = [[0; 32]; 2];
    ids[0].copy_from_slice(&bytes[..32]);
    ids[1].copy_from_slice(&bytes[32..]);
    Ok(ids)
}
pub(super) fn verify_initial(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    pin: WorldRecoveryPin,
) -> Result<()> {
    if config.live.is_none() {
        return Err(fail("interactive command requires live configuration"));
    }
    let caller = Limits::default();
    let selected = config
        .world
        .as_ref()
        .ok_or_else(|| fail("interactive requires separately provisioned synthetic world"))?;
    let limits = LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: caller.decode_limits(),
            max_actions: selected.capacity as u64,
        },
        manifest: bounds(),
        caller,
    };
    let mut agent = DiskBlockStore::open_existing(&config.directory, caller.decode_limits())?;
    let mut ws = DiskBlockStore::open_existing(&selected.directory, caller.decode_limits())?;
    let mut registry = Registry::new(1)?;
    let mut book = AdmissionBook::new(1024)?;
    adopt_local(
        module,
        &mut agent,
        &mut registry,
        &mut book,
        pin.source,
        &bounds(),
        &caller,
    )?;
    let raw = agent
        .manifest(bounds().wire)
        .read()?
        .ok_or_else(|| fail("interactive approved source unavailable"))?;
    let manifest = CheckpointManifest::decode(&raw, &bounds())?;
    if manifest
        .admissions()
        .iter()
        .any(|r| r.status != AdmissionStatus::Consumed)
    {
        return Err(fail(
            "interactive source has unresolved admission; never relaunch",
        ));
    }
    let reg = registry
        .agent(config.agent)
        .ok_or_else(|| fail("interactive approved source unavailable"))?;
    if reg.program_binding().definition != definition.cid()
        || !matches!(
            reg.snapshot().state().continuation(),
            Continuation::AwaitActor
        )
    {
        return Err(fail("interactive requires approved AwaitActor; pending effects require separate execute-pending/resume"));
    }
    let world = ScriptedWorld::adopt(
        reg,
        &book,
        &mut agent,
        &mut ws,
        selected.capacity,
        &pin,
        &limits,
    )?;
    if world.state().world_id() != selected.id || world.state().initial_units() != selected.units {
        return Err(fail(
            "interactive world differs from approved configuration",
        ));
    }
    Ok(())
}
fn refusal_message(error: &(dyn std::error::Error + 'static)) -> String {
    // Only our closed native diagnostic type may be formatted, never an arbitrary error chain.
    match error.downcast_ref::<advance::NativeFailure>() {
        Some(failure) => format!("Native advancement refused: {failure}. Outcome may be unresolved; inspect checkpoints. Never auto-refund/relaunch/replay."),
        None => "Native advancement refused or outcome unresolved. Details withheld; inspect independently selected checkpoints. Never auto-refund/relaunch/replay.".into(),
    }
}
pub(super) struct Controller {
    pub paired: WorldRecoveryPin,
    pub remaining: usize,
    pub blocked: bool,
}
impl Controller {
    /// Only called for a locked native terminal Submit. Caller driver still validates full authority.
    pub(super) fn submit(
        &mut self,
        ui: &mut TerminalUi,
        plan: &GrantPlan,
        ids: [[u8; 32]; 2],
        mut drive: impl FnMut(NativeRequest) -> Result<Accepted>,
    ) -> Result<()> {
        if !ui.is_pending() || self.blocked || self.remaining == 0 {
            return Err(fail(
                "interactive submit not authorized by current native controller",
            ));
        }
        let result = (|| -> Result<Accepted> {
            let (input, grant) = plan.input(ui.prompt(), self.paired, ids)?;
            // Consume this invocation's approval even if the outcome is unknown; never auto-retry.
            self.remaining -= 1;
            drive(NativeRequest {
                paired: self.paired,
                input,
                grant,
                cause: Cause::ActorWake,
                script: None,
                capture_continuation: true,
            })
        })();
        match result {
            Ok(accepted) => {
                ui.complete_input(true);
                if let Some(text) = accepted.model_text {
                    let _ = ui.push_history(Role::Model, &text);
                } else {
                    let _ = ui.push_history(
                        Role::Status,
                        "No assistant text was supplied. Acceptance is not a textual answer.",
                    );
                }
                let _ = ui.push_history(Role::Status, &accepted.display);
                match accepted.next {
                    Some(pin) if accepted.await_actor && self.remaining > 0 => {
                        self.paired = pin;
                        let _=ui.push_history(Role::Status,"Boundary ACCEPTED · AwaitActor. Enter can approve a fresh event on the exported known RAM pin.");
                    }
                    _ => {
                        self.blocked = true;
                        ui.set_enabled(false);
                        let _=ui.push_history(Role::Status,"Boundary accepted. Prompt blocked: next approved pin unavailable, pending action, or finite grant plan exhausted. No replay; use independent inspection/execute-pending/resume.");
                    }
                }
            }
            Err(error) => {
                self.blocked = true;
                ui.complete_input(false);
                ui.set_enabled(false);
                let _ = ui.push_history(Role::Error, &refusal_message(error.as_ref()));
            }
        }
        Ok(())
    }
}
// Input arriving during synchronous inference must not silently become a next approved event.
fn discard_queued(
    mut read: impl FnMut() -> std::io::Result<Option<crossterm::event::Event>>,
    cap: u64,
) -> std::io::Result<(bool, bool)> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let mut exit = false;
    for _ in 0..cap {
        let Some(event) = read()? else {
            return Ok((exit, false));
        };
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Release
                && (key.code == KeyCode::Esc
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')))
            {
                exit = true;
            }
        }
    }
    Ok((exit, true))
}
pub fn run(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    grants: &Path,
    pin_file: &Path,
    world_digest: &str,
) -> Result<String> {
    run_with(
        config,
        definition,
        module,
        grants,
        pin_file,
        world_digest,
        |request| advance::execute_native(config, definition, module, request),
    )
}
pub(super) fn run_with(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    grants: &Path,
    pin_file: &Path,
    world_digest: &str,
    mut drive: impl FnMut(NativeRequest) -> Result<Accepted>,
) -> Result<String> {
    let plan = GrantPlan::load(grants)?;
    let paired = WorldRecoveryPin {
        source: selected_pin(pin_file, config, definition)?,
        world_digest: id(world_digest)?,
    };
    verify_initial(config, definition, module, paired)?;
    let caller = Limits::default();
    let mut ui = TerminalUi::new(UiLimits {
        prompt_bytes: usize::try_from(caller.max_observation_bytes.saturating_sub(16))?,
        history_bytes: usize::try_from(caller.max_context_bytes)?,
        history_entries: plan.max_events * 3 + 4,
    })?;
    ui.push_history(Role::Status,"No transcript restored. Enter: finite native ActorWake/pin approval. LOCAL TEST; no real effects. Esc after bounded inference; queued typing discarded.")?;
    let mut controller = Controller {
        paired,
        remaining: plan.max_events,
        blocked: false,
    };
    let mut terminal = TerminalSession::open()?;
    loop {
        terminal.draw(&mut ui)?;
        let Some(event) = terminal.read(Duration::from_millis(200))? else {
            continue;
        };
        match ui.handle(event) {
            Ok(Signal::Exit) => break,
            Ok(Signal::Submit) => {
                let text = ui.prompt().to_owned();
                let _ = ui.push_history(Role::Operator, &text);
                terminal.draw(&mut ui)?;
                match random_ids() {
                    Ok(ids) => controller.submit(&mut ui, &plan, ids, &mut drive)?,
                    Err(_) => {
                        ui.complete_input(false);
                        ui.set_enabled(false);
                        controller.blocked = true;
                        let _ = ui.push_history(
                            Role::Error,
                            "Fresh native identities unavailable; no event admitted.",
                        );
                    }
                }
                let (exit, overflow) = discard_queued(
                    || terminal.read(Duration::ZERO),
                    caller.max_collection_entries,
                )?;
                if overflow {
                    controller.blocked = true;
                    ui.set_enabled(false);
                    let _ = ui.push_history(
                        Role::Error,
                        "Queued-input cap reached; prompt blocked. No extra event admitted.",
                    );
                }
                if exit {
                    break;
                }
            }
            Ok(Signal::None) => {}
            Err(_) => {
                let _ = ui.push_history(
                    Role::Error,
                    "Presentation/input bound. Text preserved; no event submitted.",
                );
            }
        }
    }
    drop(terminal);
    Ok("interactive=closed; native outcomes require independently selected checkpoint inspection\n".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_input_during_inference_never_becomes_approval_and_exit_is_deferred_not_lost() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut events = std::collections::VecDeque::from([
            Event::Paste("not approved".into()),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        ]);
        assert_eq!(
            discard_queued(|| Ok(events.pop_front()), 4).unwrap(),
            (true, false)
        );
        let mut events = std::collections::VecDeque::from([
            Event::Paste("flood".into()),
            Event::Paste("flood".into()),
        ]);
        assert_eq!(
            discard_queued(|| Ok(events.pop_front()), 1).unwrap(),
            (false, true)
        );
        assert_eq!(events.len(), 1);
    }
    #[test]
    fn refusal_presentation_formats_only_closed_native_diagnostics_not_raw_errors() {
        use raiv_agent_host::inference::http::ClientError;
        for error in [
            ClientError::Timeout,
            ClientError::Status(429),
            ClientError::Response,
            ClientError::Stream(raiv_agent_host::inference::sse::StreamError::Frame),
            ClientError::CredentialEcho,
        ] {
            let message = refusal_message(&advance::NativeFailure::provider(error));
            assert!(message.contains(&format!("provider: {error}")));
            assert!(message.contains("Never auto-refund/relaunch/replay"));
        }
        let message = refusal_message(&advance::NativeFailure::stage("reply_mode"));
        assert!(message.contains("native stage: reply_mode"));
        let unknown = refusal_message(&advance::NativeFailure::stage("SECRET_SENTINEL"));
        assert!(!unknown.contains("SECRET_SENTINEL"));
        assert!(unknown.contains("guest_refusal"));
        let raw = fail("SECRET_SENTINEL raw provider body/reasoning/arguments");
        assert!(!refusal_message(raw.as_ref()).contains("SECRET_SENTINEL"));
        assert!(refusal_message(raw.as_ref()).contains("Details withheld"));
    }
    #[test]
    fn finite_plan_is_closed_exact_text_and_actor_input_contains_only_current_prompt() {
        let source="activation_units=\"100\"\nremaining_episode_units=\"100\"\nremaining_model_calls=\"16\"\nremaining_actions=\"4\"\nmax_events=\"2\"\n";
        let plan = GrantPlan::parse(source).unwrap();
        assert_eq!(plan.max_events, 2);
        for bad in [
            source.replace("\"2\"", "\"0\""),
            source.replace("\"2\"", "\"1025\""),
            source.replace("\"100\"", "\"01\""),
            source.replace("\"100\"", "100"),
            source.to_owned() + "secret=\"never-echo\"\n",
        ] {
            assert!(GrantPlan::parse(&bad).is_err());
        }
    }
}
