//! Explicit ActorWake or world-ledger ActionResult approval -> one actual-WASM boundary, no execution.
use super::{
    bounds,
    config::{id, Config},
    execution::pending,
    fail, read_bounded,
    world::selected_pin,
    Result,
};
use raiv::{
    activation::ModelReply,
    budget::BudgetAdmission,
    definition::Definition,
    format::{validate_drisl, DecodeLimits},
    input::{ActivationInput, Cause},
    limits::Limits,
};
use raiv_agent_host::{
    acceptance::accept_local,
    admission::AdmissionBook,
    disk::DiskBlockStore,
    inference::{
        http::{ChatCompletionsClient, ClientError, ProviderConfig},
        request::{check_chat_reply_mode, encode_chat_request, ReasoningSession},
        NormalizedReply,
    },
    launch::{launch_local, record_reserved},
    recovery::adopt_local,
    registry::Registry,
    run::Phase,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{LedgerActionResult, ScriptedWorld, WorldRecoveryPin},
    },
};
use std::path::Path;
pub struct RequestFiles<'a> {
    pub event: &'a Path,
    pub grant: &'a Path,
    pub replies: Option<&'a Path>,
    pub pin: &'a Path,
    pub world_digest: &'a str,
}
fn script<'a>(bytes: &'a [u8], caller: &Limits) -> Result<Vec<&'a [u8]>> {
    let cap = caller
        .max_model_reply_bytes
        .checked_mul(caller.max_model_calls_per_activation)
        .ok_or_else(|| fail("script byte overflow"))?;
    validate_drisl(
        bytes,
        &DecodeLimits {
            max_block_bytes: cap,
            max_depth: 2,
            max_collection_entries: caller.max_model_calls_per_activation,
        },
    )?;
    let mut decoder = minicbor::Decoder::new(bytes);
    let count = decoder
        .array()?
        .ok_or_else(|| fail("script requires a definite list of normalized-reply bytes"))?;
    if count == 0 {
        return Err(fail("empty script cannot create an implicit boundary"));
    }
    let mut frames = Vec::new();
    frames.try_reserve_exact(usize::try_from(count)?)?;
    for index in 0..count {
        let reply = decoder.bytes()?;
        let parsed = ModelReply::decode(reply, caller)?;
        if parsed.request_id() != index + 1 {
            return Err(fail(
                "script request IDs must match fresh VM sequence; no remapping",
            ));
        }
        frames.push(reply);
    }
    Ok(frames)
}
/// Native transport port only. DATA/pins/phase/freshness are checked before prepare;
/// prepare precedes durable Reserved/Attempted/Launched, complete follows Launched.
pub(super) trait ProviderPort {
    fn prepare(&mut self, definition: &Definition, input: &ActivationInput) -> Result<()>;
    fn complete(&mut self, id: u64, payload: &[u8], limits: &Limits) -> Result<NormalizedReply>;
    /// Optional native diagnostics receive finite labels, never reply/guest error strings.
    fn guest_refused(&mut self, _category: &'static str) {}
}
#[derive(Clone, Copy)]
pub(super) struct NativeFailure(FailureKind);
#[derive(Clone, Copy)]
enum FailureKind {
    Provider(ClientError),
    Stage(&'static str),
}
impl NativeFailure {
    pub(super) fn provider(error: ClientError) -> Self {
        Self(FailureKind::Provider(error))
    }
    pub(super) fn stage(category: &'static str) -> Self {
        let label = match category {
            "request_mapping"
            | "reply_mode"
            | "reasoning_continuation"
            | "program_binding"
            | "input_binding"
            | "request_correlation"
            | "terminal_phase"
            | "guest_protocol"
            | "guest_failure"
            | "native_origin"
            | "frame_bound"
            | "allocation"
            | "normalized_reply"
            | "vm_bridge"
            | "candidate_verification"
            | "guest_activation_failed"
            | "guest_candidate_failed"
            | "guest_wrong_request"
            | "guest_inference_failed"
            | "guest_other_error" => category,
            _ => "guest_refusal",
        };
        Self(FailureKind::Stage(label))
    }
}
impl std::fmt::Debug for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::fmt::Display for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            FailureKind::Provider(error) => write!(f, "provider: {error}"),
            FailureKind::Stage(label) => write!(f, "native stage: {label}"),
        }
    }
}
impl std::error::Error for NativeFailure {}
struct LiveProvider<'a> {
    config: &'a ProviderConfig,
    client: Option<ChatCompletionsClient>,
    failure: Option<NativeFailure>,
}
impl ProviderPort for LiveProvider<'_> {
    fn prepare(&mut self, definition: &Definition, input: &ActivationInput) -> Result<()> {
        // One context/Activation owns the route; all retained infer turns reuse this native label.
        let session = input
            .event_id()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let prepared = (|| {
            let client =
                ChatCompletionsClient::new(self.config.clone())?.with_session_id(&session)?;
            client.guard_credential_echo(definition.as_bytes())?;
            client.guard_credential_echo(input.as_bytes())?;
            Ok::<_, ClientError>(client)
        })();
        match prepared {
            Ok(client) => {
                self.client = Some(client);
                Ok(())
            }
            Err(error) => {
                self.failure = Some(NativeFailure::provider(error));
                Err(error.into())
            }
        }
    }
    fn complete(&mut self, id: u64, payload: &[u8], limits: &Limits) -> Result<NormalizedReply> {
        let result = self
            .client
            .as_ref()
            .ok_or_else(|| fail("live provider not prepared"))?
            .complete(id, payload, limits);
        result.map_err(|error| {
            self.failure = Some(NativeFailure::provider(error));
            error.into()
        })
    }
    fn guest_refused(&mut self, category: &'static str) {
        self.failure = Some(NativeFailure::stage(category));
    }
}
pub fn execute(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    files: RequestFiles<'_>,
    cause: Cause,
) -> Result<String> {
    if let Some(selected) = &config.live {
        let mut provider = LiveProvider {
            config: selected,
            client: None,
            failure: None,
        };
        execute_using(
            config,
            definition,
            module,
            files,
            cause,
            Some(&mut provider),
        )
    } else {
        execute_using(config, definition, module, files, cause, None)
    }
}
fn execute_using(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    files: RequestFiles<'_>,
    cause: Cause,
    provider: Option<&mut dyn ProviderPort>,
) -> Result<String> {
    let caller = Limits::default();
    let cap = caller
        .max_model_reply_bytes
        .checked_mul(caller.max_model_calls_per_activation)
        .ok_or_else(|| fail("script byte overflow"))?;
    let cap = usize::try_from(cap)?;
    let request = NativeRequest {
        paired: WorldRecoveryPin {
            source: selected_pin(files.pin, config, definition)?,
            world_digest: id(files.world_digest)?,
        },
        input: ActivationInput::decode(
            &read_bounded(files.event, usize::try_from(caller.max_block_bytes)?)?,
            &caller,
        )?,
        grant: BudgetAdmission::decode(&read_bounded(files.grant, 4096)?, &caller.decode_limits())?,
        cause,
        script: files
            .replies
            .map(|path| read_bounded(path, cap))
            .transpose()?,
        capture_continuation: false,
    };
    Ok(execute_request(config, definition, module, request, provider)?.display)
}
/// Native administrative DATA, never provider-selected authority. No transcript/temporary input file.
pub(super) struct NativeRequest {
    pub paired: WorldRecoveryPin,
    pub input: ActivationInput,
    pub grant: BudgetAdmission,
    pub cause: Cause,
    pub script: Option<Vec<u8>>,
    pub capture_continuation: bool,
}
pub(super) struct Accepted {
    pub display: String,
    pub next: Option<WorldRecoveryPin>,
    pub await_actor: bool,
    pub model_text: Option<String>,
}
pub(super) fn execute_native(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    request: NativeRequest,
) -> Result<Accepted> {
    let selected = config
        .live
        .as_ref()
        .ok_or_else(|| fail("interactive command requires selected live inference"))?;
    let mut provider = LiveProvider {
        config: selected,
        client: None,
        failure: None,
    };
    execute_request(config, definition, module, request, Some(&mut provider)).map_err(|error| {
        match provider.failure {
            Some(failure) => Box::new(failure) as Box<dyn std::error::Error>,
            None => error,
        }
    })
}
pub(super) fn execute_request(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    request: NativeRequest,
    mut provider: Option<&mut dyn ProviderPort>,
) -> Result<Accepted> {
    let NativeRequest {
        paired,
        input,
        grant,
        cause,
        script: replies,
        capture_continuation,
    } = request;
    if config.live.is_some() != provider.is_some() || replies.is_some() == provider.is_some() {
        return Err(fail(
            "inference mode mismatch; scripted replies cannot bypass the live profile",
        ));
    }
    if config.definition()?.cid() != definition.cid() {
        return Err(fail(
            "Definition differs from selected configuration; no rebinding",
        ));
    }
    let selected = config
        .world
        .as_ref()
        .ok_or_else(|| fail("provision independent scripted world before activation"))?;
    let caller = Limits::default();
    let mut reasoning = config
        .reasoning_effort
        .map(|_| ReasoningSession::new(definition, caller))
        .transpose()?;
    let source = paired.source;
    let world_digest = paired.world_digest;
    if input.cause() != cause {
        return Err(fail(
            "input cause differs from explicitly selected wake/resume command; no phase bypass",
        ));
    }
    if input.budget_admission() != grant {
        return Err(fail(
            "input grant differs from separately approved native grant",
        ));
    }
    let frames = if let Some(raw) = &replies {
        script(raw, &caller)?
    } else {
        Vec::new()
    };
    let mut model_text = None;
    let limits = LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: caller.decode_limits(),
            max_actions: selected.capacity as u64,
        },
        manifest: bounds(),
        caller,
    };
    let mut agent = DiskBlockStore::open_existing(&config.directory, caller.decode_limits())?;
    let mut world_store = DiskBlockStore::open_existing(&selected.directory, limits.executor.wire)?;
    let mut registry = Registry::new(1)?;
    let mut admissions = AdmissionBook::new(1024)?;
    adopt_local(
        module,
        &mut agent,
        &mut registry,
        &mut admissions,
        source,
        &limits.manifest,
        &caller,
    )?;
    let reg = registry
        .agent(config.agent)
        .ok_or_else(|| fail("approved source unavailable"))?;
    let mut world = ScriptedWorld::adopt(
        reg,
        &admissions,
        &mut agent,
        &mut world_store,
        selected.capacity,
        &WorldRecoveryPin {
            source,
            world_digest,
        },
        &limits,
    )?;
    if world.state().world_id() != selected.id || world.state().initial_units() != selected.units {
        return Err(fail(
            "world differs from separately selected configuration; no reseed",
        ));
    }
    let token = match cause {
        Cause::ActorWake => admissions.admit_actor_wake(reg, input, grant)?,
        Cause::ActionResult => {
            let command = pending(reg.snapshot(), &caller)?;
            let installed=world.state().ledger().receipt_bytes(command.action_id())
                .ok_or_else(||fail("resume requires installed Terminal world Receipt; imported DATA is not provenance"))?;
            if input.receipt_bytes() != Some(installed) {
                return Err(fail(
                    "ActionResult differs from world's exact installed pending Receipt",
                ));
            }
            // Correlate only an already checked installed slot; never create Ready for a forgery.
            let ticket = world.authorize_pending(reg, &admissions, &mut agent, &limits)?;
            world.admit_action_result(
                ticket,
                reg,
                &mut admissions,
                &mut agent,
                LedgerActionResult {
                    input,
                    approved_grant: grant,
                },
                &limits,
            )?
        }
    };
    // A failed credential/transport preparation leaves no durable reservation or attempt.
    if let Some(port) = &mut provider {
        port.prepare(definition, token.packet().input())?;
    }
    record_reserved(
        reg,
        &admissions,
        &token,
        &mut agent,
        &limits.manifest,
        &caller,
    )?;
    let mut run = launch_local(
        module,
        reg,
        &mut admissions,
        &token,
        &mut agent,
        &limits.manifest,
        &caller,
    )?;
    let mut count = 0;
    if let Some(port) = &mut provider {
        for _ in 0..caller.max_model_calls_per_activation {
            let Some((ticket, request)) = run.pending() else {
                break;
            };
            let result = (|| -> Result<NormalizedReply> {
                // Compiled demo limits equal native caller policy; emitted DATA cannot widen it.
                if request.limits() != caller {
                    return Err(fail(
                        "guest inference limits differ from captured demo policy",
                    ));
                }
                let selected = config.live.as_ref().expect("checked live mode");
                let payload = match &mut reasoning {
                    Some(carrier) => carrier.encode(definition, ticket, request, selected),
                    None => encode_chat_request(definition, request, selected, &caller),
                }
                .inspect_err(|_| {
                    port.guest_refused("request_mapping");
                })?;
                let reply = port.complete(ticket.request_id(), &payload, &caller)?;
                check_chat_reply_mode(definition, &reply).map_err(|_| {
                    port.guest_refused("reply_mode");
                    fail("provider reply mode differs from selected continuation contract")
                })?;
                Ok(reply)
            })();
            let reply = match result {
                Ok(value) => value,
                Err(_) => {
                    let _ = run.inference_error(ticket, "native_provider_failed");
                    return Err(fail("live inference failed; provider details withheld; Launched attempt unresolved, never implicit Finish/refund/relaunch"));
                }
            };
            if capture_continuation {
                let parsed = ModelReply::decode(reply.as_bytes(), &caller)?;
                let mut text = String::new();
                if !parsed.text().is_empty() && text.try_reserve_exact(parsed.text().len()).is_ok()
                {
                    text.push_str(parsed.text());
                    model_text = Some(text);
                }
            }
            if let Err(error) = run.reply(ticket, reply.as_bytes()) {
                use raiv_agent_host::run::RunError;
                let category = match error {
                    RunError::ProgramBinding => "program_binding",
                    RunError::InputBinding => "input_binding",
                    RunError::Correlation => "request_correlation",
                    RunError::Phase => "terminal_phase",
                    RunError::Protocol => "guest_protocol",
                    RunError::GuestFailure => "guest_failure",
                    RunError::OriginExhausted => "native_origin",
                    RunError::Frame => "frame_bound",
                    RunError::Allocation => "allocation",
                    RunError::Reply(_) => "normalized_reply",
                    RunError::Bridge(_) => "vm_bridge",
                    RunError::Candidate(_) => "candidate_verification",
                };
                port.guest_refused(category);
                if let Some(raiv_agent_host::output::Output::Error { code }) = run.output() {
                    let label = match code {
                        "activation_failed" => "guest_activation_failed",
                        "candidate_failed" => "guest_candidate_failed",
                        "wrong_inference_request" => "guest_wrong_request",
                        "inference_failed" => "guest_inference_failed",
                        _ => "guest_other_error",
                    };
                    port.guest_refused(label);
                }
                let _ = run.inference_error(ticket, "native_reply_failed");
                return Err(fail("live reply failed guest routing; Launched attempt unresolved, never implicit Finish/refund/relaunch"));
            }
            // Only a retained next inference needs a carrier entry. Final thought is dropped,
            // not journaled or needlessly retained across known boundary acceptance.
            if run.phase() == Phase::Pending {
                if let Some(carrier) = &mut reasoning {
                    carrier.record(definition, ticket, reply).map_err(|_| {
                        port.guest_refused("reasoning_continuation");
                        fail("live reasoning continuation failed; details withheld; Launched attempt unresolved, never retry/refund/implicit Finish")
                    })?;
                }
            }
            count += 1;
        }
    } else {
        for frame in &frames {
            let (ticket, _request) = run.pending().ok_or_else(|| {
                fail("script contains unused replies after a boundary; no extra decision")
            })?;
            run.reply(ticket, frame)?;
            count += 1;
        }
    }
    drop(reasoning);
    if run.phase() != Phase::Boundary {
        return Err(fail(if provider.is_some() {
            "live inference exhausted without a boundary; attempt remains unresolved, never implicit Finish/relaunch"
        } else {
            "script exhausted without a boundary; attempt remains unresolved, never implicit Finish/relaunch"
        }));
    }
    let mut sealed = run.into_sealed()?;
    accept_local(
        &mut registry,
        &mut admissions,
        &token,
        &mut sealed,
        &mut agent,
        &limits.manifest,
        &caller,
    )?;
    // A failed continuation export can only block the UI, never disguise known acceptance.
    let accepted = registry
        .agent(config.agent)
        .expect("known accepted registered agent");
    let mode = if provider.is_some() {
        "live-accepted-boundary"
    } else {
        "scripted-accepted-boundary"
    };
    let display=format!("mode={mode}\nroot={}\nrevision={}\nphase={:?}\nmodel_replies={}\nvirtual_units_available={}\nledger_rows={}\nexecution=not-performed; use separate pending execution\nnext-pin=requires-independent-current-main-and-world-selection\n",
        accepted.anchor().root,accepted.anchor().revision,accepted.snapshot().state().continuation(),count,world.state().available_units(),world.state().ledger().records().count());
    let await_actor = matches!(
        accepted.snapshot().state().continuation(),
        raiv::state::Continuation::AwaitActor
    );
    let next = if capture_continuation {
        world
            .current_pin(accepted, &admissions, &mut agent, &limits)
            .ok()
    } else {
        None
    };
    Ok(Accepted {
        display,
        next,
        await_actor,
        model_text,
    })
}
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_script_preflight_rejects_empty_wrong_type_duplicate_and_oversized_frames_without_a_vm()
    {
        let caller = Limits::default();
        for bytes in [
            vec![0x80],
            vec![0x81, 0x01],
            vec![0x81, 0x41, 0xa0],
            b"\x81\x4d\xa2\x64text\x60\x64text\x60".to_vec(),
            vec![0xff],
        ] {
            assert!(script(&bytes, &caller).is_err());
        }
        let bytes = vec![
            0;
            caller.max_model_reply_bytes as usize
                * caller.max_model_calls_per_activation as usize
                + 1
        ];
        assert!(script(&bytes, &caller).is_err());
    }
}
