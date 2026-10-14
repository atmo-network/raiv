//! Owns count-admitted inference correlation/control transitions, not reply normalization,
//! unit/host approval, callback execution or checkpoint construction/acceptance.

use super::{CallBudget, CallBudgetError, UnitBudget, UnitError};
use crate::{definition::ToolControl, format::Id32};
use core::fmt;

/// Transport correlation, not persistent Episode/Action/provider identity.
/// Instance labels are supplied by the runtime; this type does not prove their provenance
/// or uniqueness. Host and guest must bind the originating live instance/run independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestBinding {
    pub instance_id: Id32,
    pub request_id: u64,
}
/// One sequence owned by a live instance, carried across its successive Activations.
/// Create once for a fresh instance; do not recreate/reset it with the same instance label.
/// Sealed/failed sessions return the non-Clone sequence, preserving the last issued ID.
#[derive(Debug, Eq, PartialEq)]
pub struct RequestSequence {
    instance: Id32,
    last: u64,
}
impl RequestSequence {
    pub fn new(instance_id: Id32) -> Self {
        Self {
            instance: instance_id,
            last: 0,
        }
    }
    pub fn instance_id(&self) -> Id32 {
        self.instance
    }
    pub fn last_id(&self) -> u64 {
        self.last
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InferencePhase {
    Ready,
    Awaiting(RequestBinding),
    ReplyReady,
    Executing(ToolControl),
    Sealed,
    Failed,
    Released,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InferenceError {
    WrongPhase,
    WrongInstance,
    WrongRequest,
    RequestIdOverflow,
    Counts(CallBudgetError),
    Units(UnitError),
}
impl From<CallBudgetError> for InferenceError {
    fn from(error: CallBudgetError) -> Self {
        Self::Counts(error)
    }
}
impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WrongPhase => "inference operation does not match the live session phase",
            Self::WrongInstance => "inference response belongs to another instance",
            Self::WrongRequest => "inference response does not match the pending request",
            Self::RequestIdOverflow => "live instance request ID overflow",
            Self::Counts(error) => return error.fmt(f),
            Self::Units(error) => return error.fmt(f),
        })
    }
}
impl core::error::Error for InferenceError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Counts(error) => Some(error),
            Self::Units(error) => Some(error),
            _ => None,
        }
    }
}

/// A single Activation's count/correlation state. It owns the budget so issuing requests
/// cannot substitute/reset another budget between replies. Only explicit registered control
/// or corrective transitions move ReplyReady; a second/stale response cannot advance it.
///
/// This is NOT a complete Activation, ModelRequest or dispatch permission. The caller must
/// check unit reservations before issue, and complete bounded normalized reply/cardinality,
/// registered tool/schema/arguments, planner output and units before applying control.
/// No callback, memory patch, provider I/O or candidate construction happens here.
#[derive(Debug, Eq, PartialEq)]
pub struct InferenceSession {
    sequence: Option<RequestSequence>,
    budget: CallBudget,
    phase: InferencePhase,
}
impl InferenceSession {
    pub fn new(sequence: RequestSequence, budget: CallBudget) -> Self {
        Self {
            sequence: Some(sequence),
            budget,
            phase: InferencePhase::Ready,
        }
    }
    pub fn phase(&self) -> InferencePhase {
        self.phase
    }
    pub fn budget(&self) -> &CallBudget {
        &self.budget
    }
    /// Stable pending view. Reading it never consumes a count or generates another ID.
    pub fn pending_request(&self) -> Option<RequestBinding> {
        match self.phase {
            InferencePhase::Awaiting(key) => Some(key),
            _ => None,
        }
    }
    /// Count-admit exactly one request before inference dispatch. Wrong phase does not
    /// mutate state. Count exhaustion or ID overflow fails the run with no pending request.
    pub fn issue(&mut self) -> Result<RequestBinding, InferenceError> {
        if self.phase != InferencePhase::Ready {
            return Err(InferenceError::WrongPhase);
        }
        let sequence = self.sequence.as_mut().expect("unreleased Ready session");
        let Some(next) = sequence.last.checked_add(1) else {
            self.phase = InferencePhase::Failed;
            return Err(InferenceError::RequestIdOverflow);
        };
        if let Err(error) = self.budget.debit_model_call() {
            self.phase = InferencePhase::Failed;
            return Err(error.into());
        }
        let key = RequestBinding {
            instance_id: sequence.instance,
            request_id: next,
        };
        sequence.last = next;
        self.phase = InferencePhase::Awaiting(key);
        Ok(key)
    }
    /// Atomic count/nonce/unit admission for a trusted profile's upper-bound quote.
    /// Wrong phase/packet preserves all state; insufficient units fails without issuing,
    /// count/nonce refusal leaves units unspent. Reservation precedes external inference;
    /// unknown/failed billing cannot refund it or rewrite nullable Usage here.
    pub fn issue_with_units(
        &mut self,
        units: &mut UnitBudget,
        quote: u64,
    ) -> Result<RequestBinding, InferenceError> {
        if self.phase != InferencePhase::Ready {
            return Err(InferenceError::WrongPhase);
        }
        self.check_units(units, quote)?;
        let key = self.issue()?;
        units.commit(quote);
        Ok(key)
    }
    /// Pure correlation preflight. Wrong/stale/duplicate/other-instance keys preserve the
    /// exact pending request and spent model counts; they are not inference failures.
    pub fn check_response(&self, key: RequestBinding) -> Result<(), InferenceError> {
        let InferencePhase::Awaiting(expected) = self.phase else {
            return Err(InferenceError::WrongPhase);
        };
        if key.instance_id != expected.instance_id {
            return Err(InferenceError::WrongInstance);
        }
        if key.request_id != expected.request_id {
            return Err(InferenceError::WrongRequest);
        }
        Ok(())
    }
    /// Consume matching correlation once, only after complete normalized-reply preflight.
    /// This does not validate text, Usage, calls/cardinality or arguments itself.
    pub fn accept_reply(&mut self, key: RequestBinding) -> Result<(), InferenceError> {
        self.check_response(key)?;
        self.phase = InferencePhase::ReplyReady;
        Ok(())
    }
    /// A matching provider failure is fatal, not an action Receipt or implicit Finish.
    /// Its already issued model slot stays spent; wrong keys leave the request pending.
    pub fn inference_error(&mut self, key: RequestBinding) -> Result<(), InferenceError> {
        self.check_response(key)?;
        self.phase = InferencePhase::Failed;
        Ok(())
    }
    /// Apply an already validated single registered decision AFTER required local checks.
    /// A refused control debit leaves ReplyReady for bounded correction; successful terminal
    /// control seals this session, but does not prove or construct a CommitCandidate.
    pub fn control(&mut self, control: ToolControl) -> Result<(), InferenceError> {
        if self.phase != InferencePhase::ReplyReady {
            return Err(InferenceError::WrongPhase);
        }
        self.budget.debit_control(control)?;
        self.phase = match control {
            ToolControl::Continue => InferencePhase::Ready,
            ToolControl::Yield | ToolControl::Finish => InferencePhase::Sealed,
        };
        Ok(())
    }
    /// Atomic count/control/unit guard before an already validated registered operation.
    /// A count refusal leaves ReplyReady/units intact; unit exhaustion fails the run without
    /// sealing a candidate. Quote authenticity, arguments/output/context and actual effects
    /// remain the enclosing runtime's duties, including failures after this admitted debit.
    pub fn control_with_units(
        &mut self,
        control: ToolControl,
        units: &mut UnitBudget,
        quote: u64,
    ) -> Result<(), InferenceError> {
        if self.phase != InferencePhase::ReplyReady {
            return Err(InferenceError::WrongPhase);
        }
        self.check_units(units, quote)?;
        self.control(control)?;
        units.commit(quote);
        Ok(())
    }
    fn check_units(&mut self, units: &UnitBudget, quote: u64) -> Result<(), InferenceError> {
        if let Err(error) = units.check(self.budget.binding(), quote) {
            if error == UnitError::Exhausted {
                self.phase = InferencePhase::Failed;
            }
            return Err(InferenceError::Units(error));
        }
        Ok(())
    }
    /// Text/invalid-control recovery is bounded per Activation. Exhaustion fails the run
    /// without sealing a candidate or inventing Finish; model counts already spent remain.
    pub fn corrective_turn(&mut self) -> Result<(), InferenceError> {
        if self.phase != InferencePhase::ReplyReady {
            return Err(InferenceError::WrongPhase);
        }
        if let Err(error) = self.budget.debit_corrective_turn() {
            self.phase = InferencePhase::Failed;
            return Err(error.into());
        }
        self.phase = InferencePhase::Ready;
        Ok(())
    }
    /// Internal callback admission charges Continue before work; Yield action count waits
    /// for valid planned Command output. Unit quotes stay spent on failed admitted work.
    pub(crate) fn begin_execution(
        &mut self,
        control: ToolControl,
        units: &mut UnitBudget,
        quote: u64,
    ) -> Result<(), InferenceError> {
        if self.phase != InferencePhase::ReplyReady {
            return Err(InferenceError::WrongPhase);
        }
        let remaining = self.budget.remaining();
        if control == ToolControl::Yield && remaining.actions == 0 {
            return Err(InferenceError::Counts(CallBudgetError::ActionsExhausted));
        }
        if control == ToolControl::Continue && remaining.local_tool_calls == 0 {
            return Err(InferenceError::Counts(CallBudgetError::LocalToolsExhausted));
        }
        self.check_units(units, quote)?;
        if control == ToolControl::Continue {
            self.budget.debit_control(control)?;
        }
        units.commit(quote);
        self.phase = InferencePhase::Executing(control);
        Ok(())
    }
    pub(crate) fn complete_execution(&mut self) -> Result<(), InferenceError> {
        let InferencePhase::Executing(control) = self.phase else {
            return Err(InferenceError::WrongPhase);
        };
        if control == ToolControl::Yield {
            self.budget.debit_control(control)?;
        }
        self.phase = if control == ToolControl::Continue {
            InferencePhase::Ready
        } else {
            InferencePhase::Sealed
        };
        Ok(())
    }
    pub(crate) fn failed_execution(&mut self) -> Result<(), InferenceError> {
        if !matches!(self.phase, InferencePhase::Executing(_)) {
            return Err(InferenceError::WrongPhase);
        }
        self.phase = InferencePhase::ReplyReady;
        self.corrective_turn()
    }
    pub(crate) fn abort(&mut self) {
        self.phase = InferencePhase::Failed;
    }
    /// Return the unchanged live-instance sequence only after terminal control/failure.
    /// The runtime must resolve candidate acceptance before starting another Activation;
    /// this extraction does not acknowledge host acceptance. Nonterminal refusal leaves
    /// the exact session intact; success irreversibly marks it Released, with no allocation.
    pub fn release_sequence(&mut self) -> Result<RequestSequence, InferenceError> {
        if !matches!(self.phase, InferencePhase::Sealed | InferencePhase::Failed) {
            return Err(InferenceError::WrongPhase);
        }
        let sequence = self.sequence.take().expect("unreleased terminal session");
        self.phase = InferencePhase::Released;
        Ok(sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_u64_request_is_issued_once_and_overflow_does_not_debit_another_model_slot() {
        let root = crate::format::DaslCid::for_block(crate::format::DaslCodec::Drisl, &[0xa0]);
        let budget = CallBudget {
            binding: crate::preparation::InputBinding {
                agent_id: [0; 32],
                execution_domain: [0; 32],
                expected_root: root,
                expected_revision: 0,
                admitted_event_id: [0; 32],
                admitted_input_digest: [0; 32],
            },
            definition: root,
            accepted: super::super::EpisodeCounts {
                model_calls: 0,
                actions: 0,
            },
            capacity: super::super::CallCounts {
                model_calls: 2,
                local_tool_calls: 0,
                actions: 0,
                corrective_turns: 1,
            },
            used: super::super::CallCounts {
                model_calls: 0,
                local_tool_calls: 0,
                actions: 0,
                corrective_turns: 0,
            },
        };
        let mut session = InferenceSession::new(
            RequestSequence {
                instance: [9; 32],
                last: u64::MAX - 1,
            },
            budget,
        );
        let key = session.issue().unwrap();
        assert_eq!(key.request_id, u64::MAX);
        assert_eq!(key.instance_id, [9; 32]);
        session.accept_reply(key).unwrap();
        session.corrective_turn().unwrap();
        assert_eq!(session.issue(), Err(InferenceError::RequestIdOverflow));
        assert_eq!(session.budget().used().model_calls, 1);
        assert_eq!(session.phase(), InferencePhase::Failed);
        let sequence = session.release_sequence().unwrap();
        assert_eq!(sequence.last_id(), u64::MAX);
        assert_eq!(sequence.instance_id(), [9; 32]);
        assert_eq!(session.phase(), InferencePhase::Released);
        assert_eq!(session.release_sequence(), Err(InferenceError::WrongPhase));
    }
}
