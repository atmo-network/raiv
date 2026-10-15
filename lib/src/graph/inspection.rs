//! Typed required-closure discovery; deliberately excludes accepted lineage and host authority.

use super::{
    read, AgentState, Budget, Caps, Definition, GraphError, Limits, MaterializedBlocks, Memory,
};
use crate::boundary::Boundary;
use crate::episode::Episode;
use crate::format::{validate_drisl, CodecError, DaslCid};
use crate::input::ActivationInput;
use alloc::vec::Vec;
use core::fmt;

/// Only Episode placement/position/exit-shape failures, not full lineage validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpisodeRoleError {
    RevisionZeroWithEpisodes,
    MissingCompletedHistory,
    RepeatedEpisode(DaslCid),
    RepeatedBoundary(DaslCid),
    CurrentEpisodeClosed(DaslCid),
    HistoryEpisodeOpen(DaslCid),
    EmptyEpisode(DaslCid),
    UnexpectedExit(DaslCid),
}

impl fmt::Display for EpisodeRoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionZeroWithEpisodes => f.write_str("revision zero cannot contain Episodes"),
            Self::MissingCompletedHistory => {
                f.write_str("non-genesis AwaitActor requires completed history")
            }
            Self::RepeatedEpisode(cid) => write!(f, "repeated semantic Episode position: {cid}"),
            Self::RepeatedBoundary(cid) => write!(f, "repeated semantic Boundary position: {cid}"),
            Self::CurrentEpisodeClosed(cid) => write!(f, "current Episode is closed: {cid}"),
            Self::HistoryEpisodeOpen(cid) => write!(f, "history Episode is open: {cid}"),
            Self::EmptyEpisode(cid) => write!(f, "accepted Episode has no Boundary: {cid}"),
            Self::UnexpectedExit(cid) => write!(
                f,
                "Boundary exit disagrees with Episode role/position: {cid}"
            ),
        }
    }
}

impl core::error::Error for EpisodeRoleError {}

/// Identity binding within inspected bytes, not host provenance or authorized registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    TriggerEventMismatch(DaslCid),
    EpisodeIdMismatch(DaslCid),
    ActionIdMismatch(DaslCid),
    PendingActionMismatch(DaslCid),
    ActionIndexOverflow(DaslCid),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TriggerEventMismatch(cid) => write!(
                f,
                "Episode trigger differs from first activation event: {cid}"
            ),
            Self::EpisodeIdMismatch(cid) => write!(
                f,
                "Episode ID differs from domain/agent/trigger derivation: {cid}"
            ),
            Self::ActionIdMismatch(cid) => write!(
                f,
                "Command ID differs from logical action-slot derivation: {cid}"
            ),
            Self::PendingActionMismatch(cid) => write!(
                f,
                "root pending action ID differs from final Command: {cid}"
            ),
            Self::ActionIndexOverflow(cid) => {
                write!(f, "Episode action index overflows u64: {cid}")
            }
        }
    }
}

impl core::error::Error for IdentityError {}

/// Episode ordering/cause/receipt pairing errors, not host provenance or full policy checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionError {
    OrdinalMismatch(DaslCid),
    OrdinalOverflow(DaslCid),
    FirstInputCause(DaslCid),
    SubsequentInputCause(DaslCid),
    ReceiptMismatch(DaslCid),
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrdinalMismatch(cid) => {
                write!(f, "Boundary ordinal is not contiguous from zero: {cid}")
            }
            Self::OrdinalOverflow(cid) => {
                write!(f, "Episode Boundary ordinal overflows u64: {cid}")
            }
            Self::FirstInputCause(cid) => write!(
                f,
                "first Boundary input is not ActorWake without receipt: {cid}"
            ),
            Self::SubsequentInputCause(cid) => write!(
                f,
                "subsequent Boundary input is not ActionResult with receipt: {cid}"
            ),
            Self::ReceiptMismatch(cid) => write!(
                f,
                "Receipt does not bind the preceding exact Command: {cid}"
            ),
        }
    }
}

impl core::error::Error for TransitionError {}

/// Declared memory-link consistency, not provenance, patch correctness or full restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryConsistencyError {
    ProfileMismatch(DaslCid),
    EntryMismatch(DaslCid),
    RootMismatch(DaslCid),
}

impl fmt::Display for MemoryConsistencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProfileMismatch(cid) => write!(
                f,
                "memory presence disagrees with Definition profile: {cid}"
            ),
            Self::EntryMismatch(cid) => write!(
                f,
                "Episode entry memory differs from preceding final memory: {cid}"
            ),
            Self::RootMismatch(cid) => {
                write!(f, "root memory differs from latest Boundary memory: {cid}")
            }
        }
    }
}

impl core::error::Error for MemoryConsistencyError {}

/// Declared revision/retention consistency, not accepted lineage or lifetime accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryConsistencyError {
    CallerWindowLimit(DaslCid),
    DefinitionWindowLimit(DaslCid),
    BoundaryCountOverflow(DaslCid),
    RevisionTooSmall(DaslCid),
    IncompleteWindow(DaslCid),
}

impl fmt::Display for HistoryConsistencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallerWindowLimit(cid) => {
                write!(f, "history exceeds caller retention capacity: {cid}")
            }
            Self::DefinitionWindowLimit(cid) => {
                write!(f, "history exceeds Definition retention window: {cid}")
            }
            Self::BoundaryCountOverflow(cid) => {
                write!(f, "retained Boundary count overflows u64: {cid}")
            }
            Self::RevisionTooSmall(cid) => {
                write!(f, "revision is smaller than retained Boundary count: {cid}")
            }
            Self::IncompleteWindow(cid) => write!(
                f,
                "evicted prefix requires the full Definition history window: {cid}"
            ),
        }
    }
}

impl core::error::Error for HistoryConsistencyError {}

/// Accepted counter ceilings only; not cost conversion, admission or lifetime accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageLimitError {
    ActivationModelCalls(DaslCid),
    ActivationLocalTools(DaslCid),
    EpisodeModelCalls(DaslCid),
    EpisodeActions(DaslCid),
}

impl fmt::Display for UsageLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivationModelCalls(cid) => write!(
                f,
                "Boundary exceeds per-activation model-call ceiling: {cid}"
            ),
            Self::ActivationLocalTools(cid) => write!(
                f,
                "Boundary exceeds per-activation local-tool ceiling: {cid}"
            ),
            Self::EpisodeModelCalls(cid) => write!(
                f,
                "Boundary exceeds remaining Episode model-call ceiling: {cid}"
            ),
            Self::EpisodeActions(cid) => write!(
                f,
                "Boundary exceeds remaining Episode action ceiling: {cid}"
            ),
        }
    }
}

impl core::error::Error for UsageLimitError {}

/// Declared per-input call/action grants, not admission provenance, units or freshness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionGrantError {
    ModelCalls(DaslCid),
    Action(DaslCid),
}

impl fmt::Display for AdmissionGrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelCalls(cid) => write!(
                f,
                "Boundary model usage exceeds its admitted remaining-call grant: {cid}"
            ),
            Self::Action(cid) => write!(
                f,
                "Boundary yields an action without a remaining-action grant: {cid}"
            ),
        }
    }
}

impl core::error::Error for AdmissionGrantError {}

/// Command/declared-descriptor equality, not authorized registration or argument validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolBindingError {
    UnknownTool(DaslCid),
    NotYield(DaslCid),
    VersionMismatch(DaslCid),
    AuthorityMismatch(DaslCid),
}

impl fmt::Display for ToolBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool(cid) => write!(f, "Command tool is absent from Definition: {cid}"),
            Self::NotYield(cid) => write!(f, "Command descriptor control is not Yield: {cid}"),
            Self::VersionMismatch(cid) => {
                write!(f, "Command tool version differs from Definition: {cid}")
            }
            Self::AuthorityMismatch(cid) => write!(
                f,
                "Command authority binding differs from Definition: {cid}"
            ),
        }
    }
}

impl core::error::Error for ToolBindingError {}

/// Complete hash/schema-checked required block set, NOT a restored runnable agent.
/// Does not establish derived IDs, receipt pairing, ordinals, phase/history/memory
/// consistency, Definition authorization, accepted HEAD, freshness or dispatch permission.
/// Counts unique encoded bytes, not allocator/RAM high-water usage. Duplicate semantic
/// positions remain in the typed records; check_episode_roles rejects a bounded subset
/// separately, without turning an inspection into a fully restored agent.
#[derive(Debug, Eq, PartialEq)]
pub struct InspectedClosure {
    nodes: Vec<Node>,
    encoded_bytes: u64,
}

impl InspectedClosure {
    /// Inspect materialized exact bytes only; failure returns no partial successful object.
    /// Caller/Definition wire, unique-closure, memory and embedded-record caps only tighten.
    /// Caller history count (including zero) is capped before Episode reads without truncation.
    /// Definition retention/revision and coupled lifetime/policy still need semantic validation.
    /// No hydration, model execution or fallback.
    pub fn inspect(
        root: DaslCid,
        store: &impl MaterializedBlocks,
        limits: &Limits,
    ) -> Result<Self, GraphError> {
        root.require_state().map_err(|_| CodecError::InvalidLink)?;
        let mut budget = Budget {
            caps: Caps::from_limits(limits),
            bytes: 0,
            blocks: 0,
        };
        let mut nodes = Vec::new();
        load(
            &mut nodes,
            Target(root, Kind::State),
            store,
            limits,
            &mut budget,
        )?;
        let definition = match &nodes[0].block {
            Block::State(state) => state.definition_link(),
            _ => unreachable!("first inspected block is the root"),
        };
        if root == definition {
            return Err(GraphError::TargetKindMismatch(root));
        }
        load(
            &mut nodes,
            Target(definition, Kind::Definition),
            store,
            limits,
            &mut budget,
        )?;
        let effective = match &nodes[1].block {
            Block::Definition(definition) => tightened(limits, definition.limits()),
            _ => unreachable!("second inspected block is Definition"),
        };
        budget.caps = Caps::from_limits(&effective);
        budget.check_totals()?;
        validate_drisl(nodes[0].bytes(), &budget.caps.wire)?;
        validate_drisl(nodes[1].bytes(), &budget.caps.wire)?;
        if let Block::State(state) = &nodes[0].block {
            if state.history_count() > limits.history_episodes {
                return Err(HistoryConsistencyError::CallerWindowLimit(root).into());
            }
        }
        let mut pending = Vec::new();
        children(&nodes[0], &nodes, &mut pending, budget.caps.blocks)?;
        while let Some(target) = pending.pop() {
            load(&mut nodes, target, store, &effective, &mut budget)?;
            children(
                nodes.last().expect("loaded one node"),
                &nodes,
                &mut pending,
                budget.caps.blocks,
            )?;
        }
        Ok(Self {
            nodes,
            encoded_bytes: budget.bytes,
        })
    }

    /// Check a bounded semantic subset without I/O or another payload copy.
    /// History must be closed, current open, with nonempty and unique semantic positions.
    /// Only a completed Episode's last Boundary may exit AwaitActor; all others await action.
    /// Does NOT prove derived IDs, pending action-ID binding, contiguous ordinals, no_op,
    /// receipt pairing, memory consistency, history windows or accepted authority.
    pub fn check_episode_roles(&self) -> Result<(), GraphError> {
        let state = self.state();
        if state.revision() == 0 {
            return if state.history_count() == 0 && state.current_episode_link().is_none() {
                Ok(())
            } else {
                Err(EpisodeRoleError::RevisionZeroWithEpisodes.into())
            };
        }
        if state.current_episode_link().is_none() && state.history_count() == 0 {
            return Err(EpisodeRoleError::MissingCompletedHistory.into());
        }
        let mut episodes = Vec::new();
        let mut boundaries = Vec::new();
        for cid in state.history() {
            self.check_role(cid, false, &mut episodes, &mut boundaries)?;
        }
        if let Some(cid) = state.current_episode_link() {
            self.check_role(cid, true, &mut episodes, &mut boundaries)?;
        }
        Ok(())
    }

    fn check_role(
        &self,
        cid: DaslCid,
        current: bool,
        episodes: &mut Vec<(DaslCid, crate::format::Id32)>,
        boundaries: &mut Vec<(DaslCid, crate::format::Id32, u64)>,
    ) -> Result<(), GraphError> {
        let episode = self.episode(cid);
        if episodes
            .iter()
            .any(|(seen, id)| *seen == cid || *id == episode.id())
        {
            return Err(EpisodeRoleError::RepeatedEpisode(cid).into());
        }
        if episode.closed() == current {
            return Err(if current {
                EpisodeRoleError::CurrentEpisodeClosed(cid)
            } else {
                EpisodeRoleError::HistoryEpisodeOpen(cid)
            }
            .into());
        }
        let mut remaining = episode.boundary_count();
        if remaining == 0 {
            return Err(EpisodeRoleError::EmptyEpisode(cid).into());
        }
        episodes
            .try_reserve(1)
            .map_err(|_| CodecError::AllocationFailure)?;
        episodes.push((cid, episode.id()));
        for boundary_cid in episode.boundaries() {
            let boundary = self.boundary(boundary_cid);
            if boundaries.iter().any(|(seen, id, ordinal)| {
                *seen == boundary_cid || (*id == episode.id() && *ordinal == boundary.ordinal())
            }) {
                return Err(EpisodeRoleError::RepeatedBoundary(boundary_cid).into());
            }
            remaining -= 1;
            let actor_exit = matches!(boundary.exit(), crate::boundary::Exit::AwaitActor);
            if actor_exit != (!current && remaining == 0) {
                return Err(EpisodeRoleError::UnexpectedExit(boundary_cid).into());
            }
            boundaries
                .try_reserve(1)
                .map_err(|_| CodecError::AllocationFailure)?;
            boundaries.push((boundary_cid, episode.id(), boundary.ordinal()));
        }
        Ok(())
    }

    /// Also checks Episode roles. Bind all declared IDs to these exact graph records.
    /// Action indexes count preceding AwaitAction boundaries, NOT Boundary ordinals.
    /// No store read, Command payload copy or host-registration/provenance claim.
    /// Receipt pairing, contiguous ordinals, memory/history/coupled policy remain unproven.
    pub fn check_identities(&self) -> Result<(), GraphError> {
        self.check_episode_roles()?;
        let state = self.state();
        let domain = self.definition().execution_domain();
        for cid in state.history().chain(state.current_episode_link()) {
            let episode = self.episode(cid);
            let first = episode
                .boundaries()
                .next()
                .expect("role-checked nonempty Episode");
            let input_cid = self.boundary(first).input_link();
            let input = self.input(input_cid);
            if episode.trigger_event_id() != input.event_id() {
                return Err(IdentityError::TriggerEventMismatch(cid).into());
            }
            if episode.id()
                != crate::identity::episode_id(domain, state.agent_id(), episode.trigger_event_id())
            {
                return Err(IdentityError::EpisodeIdMismatch(cid).into());
            }
            let mut action_index = 0u64;
            for boundary_cid in episode.boundaries() {
                if let crate::boundary::Exit::AwaitAction(command) =
                    self.boundary(boundary_cid).exit()
                {
                    let expected = crate::identity::action_id(
                        domain,
                        state.agent_id(),
                        episode.id(),
                        action_index,
                    );
                    if crate::command::Command::action_id_in_validated(command) != expected {
                        return Err(IdentityError::ActionIdMismatch(boundary_cid).into());
                    }
                    action_index = action_index
                        .checked_add(1)
                        .ok_or(IdentityError::ActionIndexOverflow(cid))?;
                }
            }
        }
        if let crate::state::Continuation::AwaitAction { action_id } = state.continuation() {
            let cid = state
                .current_episode_link()
                .expect("typed continuation presence");
            let last = self
                .episode(cid)
                .boundaries()
                .last()
                .expect("role-checked nonempty Episode");
            let crate::boundary::Exit::AwaitAction(command) = self.boundary(last).exit() else {
                unreachable!("role-checked current tail awaits action")
            };
            if action_id != crate::command::Command::action_id_in_validated(command) {
                return Err(IdentityError::PendingActionMismatch(last).into());
            }
        }
        Ok(())
    }

    /// Includes roles/identities; then checks ordinal/cause/terminal-pair consistency.
    /// Uses exact borrowed Command/Receipt bytes without I/O or new payload/preimage copies.
    /// Does NOT prove memory/history/coupled budgets, finality, source provenance or authority.
    pub fn check_transitions(&self) -> Result<(), GraphError> {
        self.check_identities()?;
        let domain = self.definition().execution_domain();
        for cid in self
            .state()
            .history()
            .chain(self.state().current_episode_link())
        {
            let episode = self.episode(cid);
            let mut remaining = episode.boundary_count();
            let mut ordinal = 0u64;
            let mut previous_command = None;
            for boundary_cid in episode.boundaries() {
                let boundary = self.boundary(boundary_cid);
                if boundary.ordinal() != ordinal {
                    return Err(TransitionError::OrdinalMismatch(boundary_cid).into());
                }
                let input = self.input(boundary.input_link());
                if ordinal == 0 {
                    if input.cause() != crate::input::Cause::ActorWake
                        || input.receipt_bytes().is_some()
                    {
                        return Err(TransitionError::FirstInputCause(boundary_cid).into());
                    }
                } else {
                    if input.cause() != crate::input::Cause::ActionResult {
                        return Err(TransitionError::SubsequentInputCause(boundary_cid).into());
                    }
                    let receipt = input
                        .receipt_bytes()
                        .expect("typed ActionResult has terminal Receipt");
                    let command =
                        previous_command.expect("role-checked nonfinal Boundary awaits action");
                    if !crate::receipt::Receipt::matches_in_validated(receipt, domain, command) {
                        return Err(TransitionError::ReceiptMismatch(boundary_cid).into());
                    }
                }
                previous_command = match boundary.exit() {
                    crate::boundary::Exit::AwaitAction(command) => Some(command),
                    crate::boundary::Exit::AwaitActor => None,
                };
                remaining -= 1;
                if remaining != 0 {
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or(TransitionError::OrdinalOverflow(cid))?;
                }
            }
        }
        Ok(())
    }

    /// Includes transitions; check all memory presences and retained Episode/root joins.
    /// Earliest retained entry has no invented genesis/evicted-history anchor.
    /// No I/O or additional payload copies; does NOT prove patches, revision/accounting,
    /// history windows, coupled policy, authorized lineage or runnable restoration.
    pub fn check_memory_consistency(&self) -> Result<(), GraphError> {
        self.check_transitions()?;
        let enabled = self.definition().memory_profile()
            == crate::definition::MemoryProfile::IntentsStrategyTactics;
        let state = self.state();
        let check_presence = |cid, memory: Option<DaslCid>| {
            if memory.is_some() == enabled {
                Ok(())
            } else {
                Err(GraphError::from(MemoryConsistencyError::ProfileMismatch(
                    cid,
                )))
            }
        };
        check_presence(self.root(), state.memory_link())?;
        let mut previous = None;
        for cid in state.history().chain(state.current_episode_link()) {
            let episode = self.episode(cid);
            check_presence(cid, episode.entry_memory_link())?;
            if let Some(final_memory) = previous {
                if episode.entry_memory_link() != final_memory {
                    return Err(MemoryConsistencyError::EntryMismatch(cid).into());
                }
            }
            for boundary_cid in episode.boundaries() {
                let memory = self.boundary(boundary_cid).memory_link();
                check_presence(boundary_cid, memory)?;
                previous = Some(memory);
            }
        }
        if let Some(final_memory) = previous {
            if state.memory_link() != final_memory {
                return Err(MemoryConsistencyError::RootMismatch(self.root()).into());
            }
        }
        Ok(())
    }

    /// Includes memory/transition checks; validates declared revision and retained history.
    /// Caller history capacity is already enforced by inspection without policy truncation.
    /// No invented evicted blocks; does NOT prove revision provenance, lifetime accounting,
    /// coupled policy or runnable restoration.
    pub fn check_history_consistency(&self) -> Result<(), GraphError> {
        self.check_memory_consistency()?;
        let state = self.state();
        let window = self.definition().limits().history_episodes.max(1);
        if state.history_count() > window {
            return Err(HistoryConsistencyError::DefinitionWindowLimit(self.root()).into());
        }
        let mut retained = 0u64;
        for cid in state.history().chain(state.current_episode_link()) {
            retained = retained
                .checked_add(self.episode(cid).boundary_count())
                .ok_or(HistoryConsistencyError::BoundaryCountOverflow(self.root()))?;
        }
        if retained > state.revision() {
            return Err(HistoryConsistencyError::RevisionTooSmall(self.root()).into());
        }
        if retained < state.revision() && state.history_count() != window {
            return Err(HistoryConsistencyError::IncompleteWindow(self.root()).into());
        }
        Ok(())
    }

    /// Includes history consistency; apply Definition ceilings to accepted Usage/actions.
    /// Capacity subtraction avoids overflowing u64 totals and resets only between Episodes.
    /// Nullable costs are untouched. Does NOT prove admission freshness/remaining ceilings,
    /// paid lost attempts, units, lifetime accounting, run provenance or full restoration.
    pub fn check_usage_limits(&self) -> Result<(), GraphError> {
        self.check_history_consistency()?;
        let limits = self.definition().limits();
        for cid in self
            .state()
            .history()
            .chain(self.state().current_episode_link())
        {
            let mut models = limits.max_model_calls_per_episode;
            let mut actions = limits.max_actions_per_episode;
            for boundary_cid in self.episode(cid).boundaries() {
                let boundary = self.boundary(boundary_cid);
                let usage = boundary.usage();
                if usage.model_calls > limits.max_model_calls_per_activation {
                    return Err(UsageLimitError::ActivationModelCalls(boundary_cid).into());
                }
                if usage.local_tool_calls > limits.max_local_tools_per_activation {
                    return Err(UsageLimitError::ActivationLocalTools(boundary_cid).into());
                }
                models = models
                    .checked_sub(usage.model_calls)
                    .ok_or(UsageLimitError::EpisodeModelCalls(boundary_cid))?;
                if matches!(boundary.exit(), crate::boundary::Exit::AwaitAction(_)) {
                    actions = actions
                        .checked_sub(1)
                        .ok_or(UsageLimitError::EpisodeActions(boundary_cid))?;
                }
            }
        }
        Ok(())
    }

    /// Includes Definition Usage ceilings; bind each Boundary to its exact input grant.
    /// Host grants and hard policy ceilings remain separate; neither overrides the other.
    /// Does NOT prove units, grant top-up/freshness/provenance, paid lost attempts,
    /// caller operational policy, tool authority or full restoration.
    pub fn check_admission_grants(&self) -> Result<(), GraphError> {
        self.check_usage_limits()?;
        for cid in self
            .state()
            .history()
            .chain(self.state().current_episode_link())
        {
            for boundary_cid in self.episode(cid).boundaries() {
                let boundary = self.boundary(boundary_cid);
                let admission = self.input(boundary.input_link()).budget_admission();
                if boundary.usage().model_calls > admission.remaining_model_calls {
                    return Err(AdmissionGrantError::ModelCalls(boundary_cid).into());
                }
                if matches!(boundary.exit(), crate::boundary::Exit::AwaitAction(_))
                    && admission.remaining_actions == 0
                {
                    return Err(AdmissionGrantError::Action(boundary_cid).into());
                }
            }
        }
        Ok(())
    }

    /// Includes grants; check all Commands against declared Yield descriptors.
    /// Reads borrowed immutable bytes; no copy or additional store read.
    /// Does NOT prove arguments against application code/schema, program registration,
    /// signing authority, dispatch permission, freshness/units or full restoration.
    pub fn check_tool_bindings(&self) -> Result<(), GraphError> {
        self.check_admission_grants()?;
        for cid in self
            .state()
            .history()
            .chain(self.state().current_episode_link())
        {
            for boundary_cid in self.episode(cid).boundaries() {
                if let crate::boundary::Exit::AwaitAction(command) =
                    self.boundary(boundary_cid).exit()
                {
                    let (id, version, authority) =
                        crate::command::Command::descriptor_in_validated(command);
                    let descriptor = self
                        .definition()
                        .tools()
                        .find(|tool| tool.id == id)
                        .ok_or(ToolBindingError::UnknownTool(boundary_cid))?;
                    if descriptor.control != crate::definition::ToolControl::Yield {
                        return Err(ToolBindingError::NotYield(boundary_cid).into());
                    }
                    if descriptor.version != version {
                        return Err(ToolBindingError::VersionMismatch(boundary_cid).into());
                    }
                    if descriptor.authority_binding != Some(authority) {
                        return Err(ToolBindingError::AuthorityMismatch(boundary_cid).into());
                    }
                }
            }
        }
        Ok(())
    }

    fn input(&self, cid: DaslCid) -> &ActivationInput {
        self.find_input(cid)
            .expect("immutable inspected Input target")
    }

    pub(super) fn find_input(&self, cid: DaslCid) -> Option<&ActivationInput> {
        self.nodes.iter().find_map(|node| match &node.block {
            Block::Input(value) if node.cid == cid => Some(value),
            _ => None,
        })
    }

    pub(super) fn episode(&self, cid: DaslCid) -> &Episode {
        self.nodes
            .iter()
            .find_map(|node| match &node.block {
                Block::Episode(value) if node.cid == cid => Some(value),
                _ => None,
            })
            .expect("immutable inspected Episode target")
    }

    fn boundary(&self, cid: DaslCid) -> &Boundary {
        self.find_boundary(cid)
            .expect("immutable inspected Boundary target")
    }

    pub(super) fn find_boundary(&self, cid: DaslCid) -> Option<&Boundary> {
        self.nodes.iter().find_map(|node| match &node.block {
            Block::Boundary(value) if node.cid == cid => Some(value),
            _ => None,
        })
    }

    pub(super) fn find_memory(&self, cid: DaslCid) -> Option<&Memory> {
        self.nodes.iter().find_map(|node| match &node.block {
            Block::Memory(value) if node.cid == cid => Some(value),
            _ => None,
        })
    }

    pub fn root(&self) -> DaslCid {
        self.nodes[0].cid
    }

    pub fn state(&self) -> &AgentState {
        match &self.nodes[0].block {
            Block::State(state) => state,
            _ => unreachable!("immutable inspected root"),
        }
    }

    pub fn definition(&self) -> &Definition {
        match &self.nodes[1].block {
            Block::Definition(definition) => definition,
            _ => unreachable!("immutable inspected Definition"),
        }
    }

    pub fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    pub fn block_count(&self) -> u64 {
        self.nodes.len() as u64
    }

    /// Exact unique required bytes; order is discovery order, not a CAR export order.
    pub fn blocks(&self) -> impl Iterator<Item = (DaslCid, &[u8])> {
        self.nodes.iter().map(|node| (node.cid, node.bytes()))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    State,
    Definition,
    Memory,
    Episode,
    Boundary,
    Input,
}

#[derive(Clone, Copy)]
struct Target(DaslCid, Kind);

#[derive(Debug, Eq, PartialEq)]
struct Node {
    cid: DaslCid,
    block: Block,
}

#[derive(Debug, Eq, PartialEq)]
enum Block {
    State(AgentState),
    Definition(Definition),
    Memory(Memory),
    Episode(Episode),
    Boundary(Boundary),
    Input(ActivationInput),
}

impl Node {
    fn kind(&self) -> Kind {
        match self.block {
            Block::State(_) => Kind::State,
            Block::Definition(_) => Kind::Definition,
            Block::Memory(_) => Kind::Memory,
            Block::Episode(_) => Kind::Episode,
            Block::Boundary(_) => Kind::Boundary,
            Block::Input(_) => Kind::Input,
        }
    }

    fn bytes(&self) -> &[u8] {
        match &self.block {
            Block::State(v) => v.as_bytes(),
            Block::Definition(v) => v.as_bytes(),
            Block::Memory(v) => v.as_bytes(),
            Block::Episode(v) => v.as_bytes(),
            Block::Boundary(v) => v.as_bytes(),
            Block::Input(v) => v.as_bytes(),
        }
    }
}

fn load(
    nodes: &mut Vec<Node>,
    target: Target,
    store: &impl MaterializedBlocks,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<(), GraphError> {
    let bytes = read(store, target.0, budget)?;
    nodes
        .try_reserve(1)
        .map_err(|_| CodecError::AllocationFailure)?;
    let block = match target.1 {
        Kind::State => Block::State(AgentState::decode(bytes, &budget.caps.wire)?),
        Kind::Definition => Block::Definition(Definition::decode(bytes, &budget.caps.wire)?),
        Kind::Memory => Block::Memory(Memory::decode(bytes, &budget.caps.memory_bounds())?),
        Kind::Episode => Block::Episode(Episode::decode(bytes, &budget.caps.wire)?),
        Kind::Boundary => Block::Boundary(Boundary::decode(bytes, limits)?),
        Kind::Input => Block::Input(ActivationInput::decode(bytes, limits)?),
    };
    nodes.push(Node {
        cid: target.0,
        block,
    });
    Ok(())
}

fn enqueue(
    target: Target,
    nodes: &[Node],
    pending: &mut Vec<Target>,
    cap: u64,
) -> Result<(), GraphError> {
    if let Some(kind) = nodes
        .iter()
        .find(|node| node.cid == target.0)
        .map(Node::kind)
        .or_else(|| {
            pending
                .iter()
                .find(|queued| queued.0 == target.0)
                .map(|queued| queued.1)
        })
    {
        return if kind == target.1 {
            Ok(())
        } else {
            Err(GraphError::TargetKindMismatch(target.0))
        };
    }
    if nodes.len() as u64 >= cap || pending.len() as u64 >= cap - nodes.len() as u64 {
        return Err(GraphError::BlockCountLimit);
    }
    pending
        .try_reserve(1)
        .map_err(|_| CodecError::AllocationFailure)?;
    pending.push(target);
    Ok(())
}

fn children(
    node: &Node,
    nodes: &[Node],
    pending: &mut Vec<Target>,
    cap: u64,
) -> Result<(), GraphError> {
    let mut add = |cid, kind| enqueue(Target(cid, kind), nodes, pending, cap);
    match &node.block {
        Block::State(state) => {
            if let Some(cid) = state.memory_link() {
                add(cid, Kind::Memory)?;
            }
            if let Some(cid) = state.current_episode_link() {
                add(cid, Kind::Episode)?;
            }
            for cid in state.history() {
                add(cid, Kind::Episode)?;
            }
        }
        Block::Episode(episode) => {
            if let Some(cid) = episode.entry_memory_link() {
                add(cid, Kind::Memory)?;
            }
            for cid in episode.boundaries() {
                add(cid, Kind::Boundary)?;
            }
        }
        Block::Boundary(boundary) => {
            add(boundary.input_link(), Kind::Input)?;
            if let Some(cid) = boundary.memory_link() {
                add(cid, Kind::Memory)?;
            }
        }
        // These owners expose no structural graph links. IDs/receipts/model maps are scalar data.
        Block::Definition(_) | Block::Memory(_) | Block::Input(_) => {}
    }
    Ok(())
}

fn tightened(caller: &Limits, definition: &Limits) -> Limits {
    Limits {
        max_block_bytes: caller.max_block_bytes.min(definition.max_block_bytes),
        max_depth: caller.max_depth.min(definition.max_depth),
        max_collection_entries: caller
            .max_collection_entries
            .min(definition.max_collection_entries),
        max_materialized_bytes: caller
            .max_materialized_bytes
            .min(definition.max_materialized_bytes),
        max_blocks: caller.max_blocks.min(definition.max_blocks),
        max_memory_bytes: caller.max_memory_bytes.min(definition.max_memory_bytes),
        max_observation_bytes: caller
            .max_observation_bytes
            .min(definition.max_observation_bytes),
        max_receipt_bytes: caller.max_receipt_bytes.min(definition.max_receipt_bytes),
        max_command_bytes: caller.max_command_bytes.min(definition.max_command_bytes),
        ..*caller
    }
}
