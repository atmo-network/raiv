//! Local scripted-ledger Receipt approval, not authenticated external Actor/executor,
//! durable input reservation, lost-attempt accounting or world/Core-loss reconciliation.

use super::*;
use crate::admission::{AdmissionError, AdmittedInput};
use raiv::{budget::BudgetAdmission, input::ActivationInput};

/// Input/observation/event and this exact grant are separately native approved. A caller
/// cannot choose the approved Receipt: it comes ONLY from the session's terminal ledger.
pub struct LedgerActionResult {
    pub input: ActivationInput,
    pub approved_grant: BudgetAdmission,
}

fn error(e: AdmissionError) -> WorldError {
    WorldError::Simulator(SimulatorError::Admission(e))
}

impl ScriptedWorld<'_> {
    /// Whole pending binding/Receipt/caller/input/grant/replay/capacity preflight and
    /// bounded native staging BEFORE exact primary + current main/closure fsync.
    /// Only then publish one RAM reservation. No ledger/balance/HEAD mutation, VM or
    /// backend call; record_reserved/launch_local separately supply durable exposure.
    pub fn admit_action_result(
        &mut self,
        ticket: ActionTicket,
        registered: &RegisteredAgent,
        admissions: &mut AdmissionBook,
        agent_store: &mut DiskBlockStore,
        request: LedgerActionResult,
        limits: &LocalExecutorLimits,
    ) -> Result<AdmittedInput, WorldError> {
        self.check_scope(registered)?;
        let index = self.book.index(ticket).map_err(WorldError::Simulator)?;
        let entry = &self.book.entries[index];
        if entry.anchor != registered.anchor()
            || entry.program != registered.program_binding()
            || entry.limits != ExecutorBook::execution_limits(registered, &limits.caller)
        {
            return Err(WorldError::Binding);
        }
        let receipt = entry.receipt.as_ref().ok_or(WorldError::NotTerminal)?;
        // Preserve ordinary admission's default bound as well as independent caller caps.
        AdmissionBook::validate_input(registered, &request.input, &limits.caller).map_err(error)?;
        let staged = admissions
            .stage_action_result(registered, request.input, receipt, request.approved_grant)
            .map_err(error)?;
        self.current_pin(registered, admissions, agent_store, limits)?;
        Ok(admissions.publish_reservation(staged))
    }
}
