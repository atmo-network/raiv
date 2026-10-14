//! Owns RAM call-count admission and tool eligibility, not the completed live lifecycle.
//! Count ceilings/debits do not authorize inference, host effects or execution-unit spend.

mod advance;
mod commit;
pub use commit::CandidateError;
mod inference;
mod reply;
mod runtime;
mod trajectory;
/// Sealed RAM decision retained by Activation, not host execution authority.
#[derive(Debug, Eq, PartialEq)]
pub enum RamDecision {
    Yield(crate::command::Command),
    Finish,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RamProgress {
    InferAgain,
    Sealed,
}
mod units;
pub use inference::{
    InferenceError, InferencePhase, InferenceSession, RequestBinding, RequestSequence,
};
pub use reply::{CallMeta, ModelReply, ReplyError, ToolCall};
pub use runtime::{
    prepare, Activation, ActivationError, ActivationPrepareError, ExecutionQuotes, ModelRequest,
    RunPolicy,
};
pub use units::{UnitBudget, UnitError};

use crate::{
    boundary::Exit,
    definition::{Definition, ToolControl},
    format::DaslCid,
    preparation::{InputBinding, ValidatedInput},
    tools::{BoundTool, ToolSet, ToolSetError},
};
use core::fmt;

/// Accepted counters for the current open Episode only, excluding completed history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpisodeCounts {
    pub model_calls: u64,
    pub actions: u64,
}
/// Per-Activation count capacities, live consumption, or remaining counts.
/// A Continue consumes local_tool_calls; Yield consumes actions; Finish consumes neither.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallCounts {
    pub model_calls: u64,
    pub local_tool_calls: u64,
    pub actions: u64,
    pub corrective_turns: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallBudgetError {
    CounterOverflow,
    ModelCallsExhausted,
    LocalToolsExhausted,
    ActionsExhausted,
    CorrectiveTurnsExhausted,
    DefinitionMismatch,
    Tools(ToolSetError),
}
impl fmt::Display for CallBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CounterOverflow => "Episode call counter overflow",
            Self::ModelCallsExhausted => "model-call count capacity exhausted",
            Self::LocalToolsExhausted => "Continue-tool count capacity exhausted",
            Self::ActionsExhausted => "Yield-action count capacity exhausted",
            Self::CorrectiveTurnsExhausted => {
                "corrective-turn count capacity exhausted after invalid tool decisions"
            }
            Self::DefinitionMismatch => "call budget was derived from another Definition",
            Self::Tools(error) => return error.fmt(f),
        })
    }
}
impl From<ToolSetError> for CallBudgetError {
    fn from(error: ToolSetError) -> Self {
        Self::Tools(error)
    }
}
impl core::error::Error for CallBudgetError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Tools(error) => Some(error),
            _ => None,
        }
    }
}

/// Private mutable counters derived from checked packet/snapshot data. Limits cannot widen.
/// ActionResult retains accepted Episode model/action counts; new ActorWake starts at zero.
/// Local/corrective counts reset per Activation, not Episode. Model debits represent admitted
/// requests (including attempts that later fail), never successful replies only.
///
/// These are count guards only: units/cost reservations, grant freshness, code/Actor/Receipt
/// authority, one live request/reply/control binding and boundary sealing remain required.
/// In particular, Finish eligibility here is not permission to finish an exhausted-unit run,
/// and exhausting any count does not synthesize Finish or mutate checkpoint data.
#[derive(Debug, Eq, PartialEq)]
pub struct CallBudget {
    binding: InputBinding,
    definition: DaslCid,
    accepted: EpisodeCounts,
    capacity: CallCounts,
    used: CallCounts,
}
impl CallBudget {
    pub fn from_input(input: &ValidatedInput<'_>) -> Result<Self, CallBudgetError> {
        let snapshot = input.snapshot();
        let definition = snapshot.definition();
        let policy = definition.limits();
        let caller = input.caller_limits();
        let grant = input.input().budget_admission();
        let mut accepted = EpisodeCounts {
            model_calls: 0,
            actions: 0,
        };
        if let Some(episode) = snapshot.current_episode() {
            for cid in episode.boundaries() {
                let boundary = snapshot.boundary(cid).expect("restored Boundary");
                accepted.model_calls = accepted
                    .model_calls
                    .checked_add(boundary.usage().model_calls)
                    .ok_or(CallBudgetError::CounterOverflow)?;
                if matches!(boundary.exit(), Exit::AwaitAction(_)) {
                    accepted.actions = accepted
                        .actions
                        .checked_add(1)
                        .ok_or(CallBudgetError::CounterOverflow)?;
                }
            }
        }
        let capacity = CallCounts {
            model_calls: policy
                .max_model_calls_per_activation
                .min(caller.max_model_calls_per_activation)
                .min(
                    policy
                        .max_model_calls_per_episode
                        .min(caller.max_model_calls_per_episode)
                        .saturating_sub(accepted.model_calls),
                )
                .min(grant.remaining_model_calls),
            actions: policy
                .max_actions_per_episode
                .min(caller.max_actions_per_episode)
                .saturating_sub(accepted.actions)
                .min(grant.remaining_actions),
            local_tool_calls: policy
                .max_local_tools_per_activation
                .min(caller.max_local_tools_per_activation),
            corrective_turns: policy.max_corrective_turns.min(caller.max_corrective_turns),
        };
        Ok(Self {
            binding: input.binding(),
            definition: definition.cid(),
            accepted,
            capacity,
            used: CallCounts {
                model_calls: 0,
                local_tool_calls: 0,
                actions: 0,
                corrective_turns: 0,
            },
        })
    }
    pub fn binding(&self) -> InputBinding {
        self.binding
    }
    pub fn accepted_episode_counts(&self) -> EpisodeCounts {
        self.accepted
    }
    pub fn capacity(&self) -> CallCounts {
        self.capacity
    }
    pub fn used(&self) -> CallCounts {
        self.used
    }
    pub fn remaining(&self) -> CallCounts {
        CallCounts {
            model_calls: self.capacity.model_calls - self.used.model_calls,
            local_tool_calls: self.capacity.local_tool_calls - self.used.local_tool_calls,
            actions: self.capacity.actions - self.used.actions,
            corrective_turns: self.capacity.corrective_turns - self.used.corrective_turns,
        }
    }
    pub fn episode_counts(&self) -> EpisodeCounts {
        // Capacities were subtracted from the effective Episode ceilings before any debit.
        EpisodeCounts {
            model_calls: self.accepted.model_calls + self.used.model_calls,
            actions: self.accepted.actions + self.used.actions,
        }
    }
    /// Debit once before external inference dispatch; repeated request views must not debit.
    pub fn debit_model_call(&mut self) -> Result<(), CallBudgetError> {
        debit(
            &mut self.used.model_calls,
            self.capacity.model_calls,
            CallBudgetError::ModelCallsExhausted,
        )
    }
    /// Debit only an already admitted single registered control decision. Cardinality,
    /// request/reply IDs, schema/profile/units checks and invocation ordering live above this.
    /// The last admitted model request can still choose Finish when no *new* call fits.
    pub fn debit_control(&mut self, control: ToolControl) -> Result<(), CallBudgetError> {
        match control {
            ToolControl::Continue => debit(
                &mut self.used.local_tool_calls,
                self.capacity.local_tool_calls,
                CallBudgetError::LocalToolsExhausted,
            ),
            ToolControl::Yield => debit(
                &mut self.used.actions,
                self.capacity.actions,
                CallBudgetError::ActionsExhausted,
            ),
            ToolControl::Finish => Ok(()),
        }
    }
    /// Charge a bounded corrective attempt, not an implicit Finish. Model-call debit is
    /// separate and stays spent even when this corrective-turn check refuses another try.
    pub fn debit_corrective_turn(&mut self) -> Result<(), CallBudgetError> {
        debit(
            &mut self.used.corrective_turns,
            self.capacity.corrective_turns,
            CallBudgetError::CorrectiveTurnsExhausted,
        )
    }
    /// Registered Definition-order views filtered by count capacity only. This iterator
    /// invokes no callbacks and grants no unit budget, model request or execution permit.
    pub fn count_enabled_tools<'a>(
        &'a self,
        tools: &'a ToolSet<'_>,
        definition: &'a Definition,
    ) -> Result<impl Iterator<Item = BoundTool<'a>> + 'a, CallBudgetError> {
        if self.definition != definition.cid() {
            return Err(CallBudgetError::DefinitionMismatch);
        }
        let remaining = self.remaining();
        Ok(tools
            .tools(definition)?
            .filter(move |tool| match tool.descriptor().control {
                ToolControl::Continue => remaining.local_tool_calls > 0,
                ToolControl::Yield => remaining.actions > 0,
                ToolControl::Finish => true,
            }))
    }
}
fn debit(used: &mut u64, capacity: u64, error: CallBudgetError) -> Result<(), CallBudgetError> {
    if *used == capacity {
        return Err(error);
    }
    // Private counters start at zero; a successful debit remains <= capacity <= u64::MAX.
    *used += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_width_debit_reaches_max_exactly_and_refuses_overflow_without_mutation() {
        let mut used = u64::MAX - 1;
        assert_eq!(
            debit(&mut used, u64::MAX, CallBudgetError::ModelCallsExhausted),
            Ok(())
        );
        assert_eq!(used, u64::MAX);
        assert_eq!(
            debit(&mut used, u64::MAX, CallBudgetError::ModelCallsExhausted),
            Err(CallBudgetError::ModelCallsExhausted)
        );
        assert_eq!(used, u64::MAX);
        let mut zero = 0;
        assert_eq!(
            debit(&mut zero, 0, CallBudgetError::ActionsExhausted),
            Err(CallBudgetError::ActionsExhausted)
        );
        assert_eq!(zero, 0);
    }
}
