//! Owns RAM Activation/request composition, not durable CommitCandidate construction,
//! provider execution or trusted host/program/grant/quote provenance.

use super::{
    CallBudget, CallBudgetError, InferenceError, InferenceSession, RequestBinding, RequestSequence,
    UnitBudget,
};
use crate::{
    definition::{ToolControl, ToolDescriptorView},
    format::{CodecError, DecodeLimits, ModelValue},
    graph::RestoredAgent,
    limits::Limits,
    memory::{Memory, MemoryError},
    preparation::{PrepareError, PreparedInput},
    projection::{InitialProjection, ProjectionError},
    tools::ToolSet,
};
use core::fmt;

/// Trusted host-profile upper-bound quotes, never token estimates or model-authored data.
/// These conservative reservations are not the nullable exact bills in Usage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionQuotes {
    pub model: u64,
    pub continue_tool: u64,
    pub yield_tool: u64,
    pub finish: u64,
}
impl ExecutionQuotes {
    pub(super) fn control(self, control: ToolControl) -> u64 {
        match control {
            ToolControl::Continue => self.continue_tool,
            ToolControl::Yield => self.yield_tool,
            ToolControl::Finish => self.finish,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunPolicy {
    pub caller: Limits,
    pub quotes: ExecutionQuotes,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationError {
    Prepared(PrepareError),
    Inference(InferenceError),
    Projection(ProjectionError),
    Memory(MemoryError),
    Codec(CodecError),
    RevisionOverflow,
    HistoryCapacity,
    NoEnabledControl,
    Reply(super::ReplyError),
    Patch(crate::memory::PatchError),
    Tool(crate::tools::ToolFailure),
    Command(crate::command::CommandError),
    UsageOverflow,
}
impl From<PrepareError> for ActivationError {
    fn from(e: PrepareError) -> Self {
        Self::Prepared(e)
    }
}
impl From<InferenceError> for ActivationError {
    fn from(e: InferenceError) -> Self {
        Self::Inference(e)
    }
}
impl From<ProjectionError> for ActivationError {
    fn from(e: ProjectionError) -> Self {
        Self::Projection(e)
    }
}
impl From<MemoryError> for ActivationError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}
impl From<CodecError> for ActivationError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}
impl fmt::Display for ActivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Prepared(e) => return e.fmt(f),
            Self::Inference(e) => return e.fmt(f),
            Self::Projection(e) => return e.fmt(f),
            Self::Memory(e) => return e.fmt(f),
            Self::Codec(e) => return e.fmt(f),
            Self::RevisionOverflow => "accepted revision cannot increment",
            Self::HistoryCapacity => {
                "caller capacity cannot retain the next promised completed history window"
            }
            Self::Reply(e) => return e.fmt(f),
            Self::Patch(e) => return e.fmt(f),
            Self::Tool(e) => return e.fmt(f),
            Self::Command(e) => return e.fmt(f),
            Self::UsageOverflow => "known reported cost/token sum overflows u64",
            Self::NoEnabledControl => {
                "no registered count/unit-enabled control remains after model reservation"
            }
        })
    }
}
impl core::error::Error for ActivationError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Prepared(e) => Some(e),
            Self::Inference(e) => Some(e),
            Self::Projection(e) => Some(e),
            Self::Memory(e) => Some(e),
            Self::Codec(e) => Some(e),
            Self::Reply(e) => Some(e),
            Self::Patch(e) => Some(e),
            Self::Tool(e) => Some(e),
            Self::Command(e) => Some(e),
            _ => None,
        }
    }
}
/// Preparation failure returns the unspent instance sequence, never resets its last ID.
/// The host retains canonical input/blocks/anchor for retry; no candidate is produced.
#[derive(Debug, Eq, PartialEq)]
pub struct ActivationPrepareError {
    pub reason: ActivationError,
    pub sequence: RequestSequence,
}

/// Owned initial RAM Activation. No store reads/provider calls/callbacks occur at prepare.
/// Root/prefix are immutable; the independent working Memory is the necessary mutable copy.
/// Registered advance_ram transitions and pure candidate construction live through
/// sibling behavior; persistence and trusted acceptance remain external.
/// The immutable ToolSet view is retained by value; only compiled registration metadata
/// is borrowed. A local registry wrapper may move/drop without self-referential storage.
/// Types/quotes alone prove no authorized host/run/program, principal, global freshness,
/// provider configuration, exact cost or accepted HEAD; those remain trusted host duties.
pub struct Activation<'tools> {
    pub(super) snapshot: RestoredAgent,
    pub(super) input: PreparedInput,
    pub(super) tools: ToolSet<'tools>,
    pub(super) policy: RunPolicy,
    pub(super) effective: Limits,
    pub(super) prefix: InitialProjection,
    pub(super) trajectory: ModelValue,
    pub(super) context: ModelValue,
    pub(super) memory: Option<Memory>,
    pub(super) session: InferenceSession,
    pub(super) units: UnitBudget,
    pub(super) usage_costs: crate::usage::Usage,
    pub(super) reported_costs: bool,
    pub(super) ram_decision: Option<super::RamDecision>,
}
/// All data preflights/allocations occur before the first count/nonce/unit reservation.
/// The registry must borrow independent compiled metadata, not fields of the snapshot
/// being moved here. The wrapper itself is not borrowed by the returned Activation.
/// Caller history remains capacity, never a truncated Definition policy.
pub fn prepare<'tools>(
    snapshot: RestoredAgent,
    input: PreparedInput,
    tools: &ToolSet<'tools>,
    policy: RunPolicy,
    sequence: RequestSequence,
) -> Result<Activation<'tools>, ActivationPrepareError> {
    let prepared = (|| -> Result<_, ActivationError> {
        let validated = input.validate(&snapshot, tools, &policy.caller)?;
        if snapshot.state().revision() == u64::MAX {
            return Err(ActivationError::RevisionOverflow);
        }
        let definition = snapshot.definition();
        let limits = definition.limits();
        let required = snapshot
            .state()
            .history_count()
            .saturating_add(1)
            .min(limits.history_episodes.max(1));
        if policy.caller.history_episodes < required {
            return Err(ActivationError::HistoryCapacity);
        }
        let budget = CallBudget::from_input(&validated).map_err(InferenceError::from)?;
        let units = UnitBudget::from_input(&validated);
        if budget.remaining().model_calls == 0 {
            return Err(InferenceError::Counts(CallBudgetError::ModelCallsExhausted).into());
        }
        sequence
            .last_id()
            .checked_add(1)
            .ok_or(InferenceError::RequestIdOverflow)?;
        units
            .check(budget.binding(), policy.quotes.model)
            .map_err(InferenceError::Units)?;
        let available = units
            .remaining_activation_units()
            .min(units.remaining_episode_units())
            - policy.quotes.model;
        if available == 0
            || (policy.quotes.finish > available
                && !budget
                    .count_enabled_tools(tools, definition)
                    .map_err(InferenceError::from)?
                    .any(|tool| policy.quotes.control(tool.descriptor().control) <= available))
        {
            return Err(ActivationError::NoEnabledControl);
        }
        let effective = effective_limits(limits, &policy.caller);
        let prefix = InitialProjection::build(&snapshot, input.input(), &policy.caller)?;
        let trajectory = ModelValue::decode(
            &[0x80],
            &DecodeLimits {
                max_block_bytes: effective.max_context_bytes,
                max_depth: effective.max_depth,
                max_collection_entries: effective.max_collection_entries,
            },
        )?;
        let context = prefix.context(&trajectory)?;
        let memory = snapshot
            .memory()
            .map(|memory| {
                Memory::decode(
                    memory.as_bytes(),
                    &DecodeLimits {
                        max_block_bytes: effective.max_memory_bytes.min(effective.max_block_bytes),
                        max_depth: effective.max_depth,
                        max_collection_entries: effective.max_collection_entries,
                    },
                )
            })
            .transpose()?;
        Ok((
            budget, units, effective, prefix, trajectory, context, memory,
        ))
    })();
    let (budget, mut units, effective, prefix, trajectory, context, memory) = match prepared {
        Ok(data) => data,
        Err(reason) => return Err(ActivationPrepareError { reason, sequence }),
    };
    let mut session = InferenceSession::new(sequence, budget);
    if let Err(error) = session.issue_with_units(&mut units, policy.quotes.model) {
        return Err(ActivationPrepareError {
            reason: error.into(),
            sequence: session
                .release_sequence()
                .expect("failed initial issue returns sequence"),
        });
    }
    Ok(Activation {
        snapshot,
        input,
        tools: *tools,
        policy,
        effective,
        prefix,
        trajectory,
        context,
        memory,
        session,
        units,
        usage_costs: crate::usage::Usage {
            model_calls: 0,
            local_tool_calls: 0,
            input_tokens: None,
            output_tokens: None,
            billed_units: None,
        },
        reported_costs: false,
        ram_decision: None,
    })
}
impl<'tools> Activation<'tools> {
    /// Borrow the same pending request repeatedly: no inference, allocation or new debit.
    pub fn request(&self) -> Result<ModelRequest<'_>, ActivationError> {
        let binding = self
            .session
            .pending_request()
            .ok_or(InferenceError::WrongPhase)?;
        Ok(ModelRequest {
            binding,
            context: self.context.as_bytes(),
            definition: self.snapshot.definition(),
            budget: self.session.budget(),
            units: &self.units,
            quotes: self.policy.quotes,
            limits: &self.effective,
            caller: &self.policy.caller,
        })
    }
    pub fn snapshot(&self) -> &RestoredAgent {
        &self.snapshot
    }
    pub fn input(&self) -> &PreparedInput {
        &self.input
    }
    pub fn working_memory(&self) -> Option<&Memory> {
        self.memory.as_ref()
    }
    pub fn initial_prefix(&self) -> &[u8] {
        self.prefix.as_bytes()
    }
    pub fn trajectory_bytes(&self) -> &[u8] {
        self.trajectory.as_bytes()
    }
    pub fn session(&self) -> &InferenceSession {
        &self.session
    }
    pub fn units(&self) -> &UnitBudget {
        &self.units
    }
    pub fn registered_tools(&self) -> &ToolSet<'tools> {
        &self.tools
    }
    /// Terminal extraction preserves instance IDs for retry/next admission; candidate
    /// acceptance must be resolved by the host before starting another Activation.
    pub fn release_sequence(&mut self) -> Result<RequestSequence, ActivationError> {
        Ok(self.session.release_sequence()?)
    }
    /// Matching failure spends no new slot/unit and produces no candidate. Wrong keys
    /// preserve the live request; actual failure seals against subsequent request views.
    pub fn inference_error(&mut self, binding: RequestBinding) -> Result<(), ActivationError> {
        self.session.inference_error(binding)?;
        Ok(())
    }
}
/// Borrowed initial model request under the supplied trusted policy. Provider JSON/SSE,
/// actual model/profile authorization and transport bounds are native adapter duties.
pub struct ModelRequest<'a> {
    binding: RequestBinding,
    context: &'a [u8],
    definition: &'a crate::definition::Definition,
    budget: &'a CallBudget,
    units: &'a UnitBudget,
    quotes: ExecutionQuotes,
    limits: &'a Limits,
    caller: &'a Limits,
}
impl<'a> ModelRequest<'a> {
    pub fn binding(&self) -> RequestBinding {
        self.binding
    }
    pub fn projection_version(&self) -> u64 {
        1
    }
    pub fn context_bytes(&self) -> &'a [u8] {
        self.context
    }
    /// Exact bounded canonical Definition model-parameter Map, never provider overrides.
    pub fn model_parameters_bytes(&self) -> &'a [u8] {
        self.definition.inference_profile_bytes()
    }
    /// Tightened operational ceilings; history_episodes retains Definition's policy.
    /// The distinct raw caller materialization/history capacity remains available below.
    pub fn limits(&self) -> &'a Limits {
        self.limits
    }
    pub fn caller_limits(&self) -> &'a Limits {
        self.caller
    }
    /// Metadata has already matched every registered implementation at preparation.
    /// Enabled views preserve Definition order; declared-prefix data is not a dispatch list.
    pub fn tools(&self) -> impl Iterator<Item = ToolDescriptorView<'a>> + 'a {
        enabled_controls(
            self.definition,
            self.budget.remaining(),
            self.units
                .remaining_activation_units()
                .min(self.units.remaining_episode_units()),
            self.quotes,
        )
    }
}
pub(super) fn enabled_controls(
    definition: &crate::definition::Definition,
    counts: super::CallCounts,
    available: u64,
    quotes: ExecutionQuotes,
) -> impl Iterator<Item = ToolDescriptorView<'_>> {
    definition.tools().filter(move |tool| {
        available > 0
            && quotes.control(tool.control) <= available
            && match tool.control {
                ToolControl::Continue => counts.local_tool_calls > 0,
                ToolControl::Yield => counts.actions > 0,
                ToolControl::Finish => true,
            }
    })
}
fn effective_limits(d: &Limits, c: &Limits) -> Limits {
    Limits {
        max_memory_bytes: d.max_memory_bytes.min(c.max_memory_bytes),
        max_observation_bytes: d.max_observation_bytes.min(c.max_observation_bytes),
        max_receipt_bytes: d.max_receipt_bytes.min(c.max_receipt_bytes),
        max_command_bytes: d.max_command_bytes.min(c.max_command_bytes),
        max_block_bytes: d.max_block_bytes.min(c.max_block_bytes),
        max_materialized_bytes: d.max_materialized_bytes.min(c.max_materialized_bytes),
        max_blocks: d.max_blocks.min(c.max_blocks),
        max_depth: d.max_depth.min(c.max_depth),
        max_collection_entries: d.max_collection_entries.min(c.max_collection_entries),
        max_context_bytes: d.max_context_bytes.min(c.max_context_bytes),
        max_model_reply_bytes: d.max_model_reply_bytes.min(c.max_model_reply_bytes),
        max_patch_entries: d.max_patch_entries.min(c.max_patch_entries),
        history_episodes: d.history_episodes,
        max_actions_per_episode: d.max_actions_per_episode.min(c.max_actions_per_episode),
        max_model_calls_per_activation: d
            .max_model_calls_per_activation
            .min(c.max_model_calls_per_activation),
        max_model_calls_per_episode: d
            .max_model_calls_per_episode
            .min(c.max_model_calls_per_episode),
        max_local_tools_per_activation: d
            .max_local_tools_per_activation
            .min(c.max_local_tools_per_activation),
        max_corrective_turns: d.max_corrective_turns.min(c.max_corrective_turns),
    }
}
