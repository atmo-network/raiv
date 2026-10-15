//! Owns retained compiled demo Activation, not host approval, provider work or checkpoint acceptance.

use super::{abi, profiles, AbiError, Guest};
use alloc::vec::Vec;
use raiv::{
    activation::{
        prepare, Activation, ActivationError, InferenceError, RamProgress, ReplyError,
        RequestBinding, RequestSequence,
    },
    graph::RestoredAgent,
    input::ActivationInput,
    preparation::{InputBinding, PreparedInput},
    tools::ToolSet,
};

impl Guest {
    pub(super) fn advance(&mut self, bytes: &[u8]) -> Result<Vec<u8>, AbiError> {
        match abi::decode(bytes)? {
            abi::Input::Initialize(input) => self.initialize(input),
            abi::Input::Activate(input) => self.activate(input),
            abi::Input::InferenceReply { request_id, reply } => {
                let run = self.live.as_mut().ok_or(AbiError::WrongRequest)?;
                let pending = run
                    .session()
                    .pending_request()
                    .ok_or(AbiError::WrongRequest)?;
                if request_id != pending.request_id {
                    return Err(AbiError::WrongRequest);
                }
                match run.advance_ram(pending, reply) {
                    Ok(RamProgress::InferAgain) => emit_infer(run),
                    Ok(RamProgress::Sealed) => {
                        let candidate = run.candidate().map_err(|_| AbiError::CandidateFailure)?;
                        abi::boundary(&candidate)
                    }
                    Err(ActivationError::Reply(ReplyError::Inference(
                        InferenceError::WrongRequest,
                    ))) => Err(AbiError::WrongRequest),
                    Err(error) => Err(run_failure(&error)),
                }
            }
            abi::Input::InferenceError { request_id } => {
                let run = self.live.as_mut().ok_or(AbiError::WrongRequest)?;
                let pending = run
                    .session()
                    .pending_request()
                    .ok_or(AbiError::WrongRequest)?;
                let binding = RequestBinding {
                    request_id,
                    ..pending
                };
                run.inference_error(binding)
                    .map_err(|_| AbiError::WrongRequest)?;
                // Host diagnostic codes never become guest state or a fake checkpoint.
                Err(AbiError::InferenceFailure)
            }
        }
    }

    fn activate(&mut self, frame: abi::Activate<'_>) -> Result<Vec<u8>, AbiError> {
        if self.live.is_some() {
            return Err(AbiError::Busy);
        }
        let policy = profiles::policy();
        let snapshot = RestoredAgent::restore(frame.root, &frame, &policy.caller)
            .map_err(|_| AbiError::InvalidActivation)?;
        let registrations = profiles::registrations(snapshot.definition())?;
        let tools = ToolSet::bind(snapshot.definition(), registrations, &policy.caller)
            .map_err(|_| AbiError::RegistryMismatch)?;
        let input = ActivationInput::decode(frame.input, &policy.caller)
            .map_err(|_| AbiError::InvalidActivation)?;
        let binding = InputBinding {
            agent_id: snapshot.state().agent_id(),
            execution_domain: snapshot.definition().execution_domain(),
            expected_root: frame.root,
            expected_revision: snapshot.state().revision(),
            admitted_event_id: input.event_id(),
            admitted_input_digest: input.cid().digest(),
        };
        // Equality/data checks do not authenticate this packet; native admission remains mandatory.
        let packet = PreparedInput::new(binding, input).map_err(|_| AbiError::InvalidActivation)?;
        // The zero internal label is local to this VM, not a globally unique run ID.
        // Native transport must separately bind the originating instance/run before routing.
        let sequence = self
            .sequence
            .take()
            .unwrap_or_else(|| RequestSequence::new([0; 32]));
        let run = match prepare(snapshot, packet, &tools, policy, sequence) {
            Ok(run) => run,
            Err(error) => {
                self.sequence = Some(error.sequence);
                return Err(AbiError::InvalidActivation);
            }
        };
        self.live = Some(run);
        emit_infer(self.live.as_mut().expect("stored Activation"))
    }
}

/// Closed category of a failed activation; no model text, arguments or provider data.
fn run_failure(error: &ActivationError) -> AbiError {
    use raiv::{activation::CallBudgetError as Budget, projection::ProjectionError};
    match error {
        ActivationError::Inference(InferenceError::Counts(Budget::CorrectiveTurnsExhausted)) => {
            AbiError::CorrectiveTurnsExhausted
        }
        ActivationError::Inference(InferenceError::Counts(
            Budget::ModelCallsExhausted | Budget::LocalToolsExhausted | Budget::ActionsExhausted,
        ))
        | ActivationError::Inference(InferenceError::Units(_)) => AbiError::BudgetExhausted,
        ActivationError::Projection(ProjectionError::ContextLimit) => AbiError::ContextLimit,
        _ => AbiError::RunFailure,
    }
}

fn emit_infer(run: &mut Activation<'_>) -> Result<Vec<u8>, AbiError> {
    let key = run.request().map_err(|_| AbiError::RunFailure)?.binding();
    match abi::infer(run.request().map_err(|_| AbiError::RunFailure)?) {
        Ok(output) => Ok(output),
        Err(error) => {
            let _ = run.inference_error(key);
            Err(error)
        }
    }
}
