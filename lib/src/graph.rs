//! Owns bounded typed checkpoint inspection/restoration, not I/O or host authority.

mod restored;

pub use restored::RestoredAgent;

mod inspection;

pub use inspection::{
    AdmissionGrantError, EpisodeRoleError, HistoryConsistencyError, IdentityError,
    InspectedClosure, MemoryConsistencyError, ToolBindingError, TransitionError, UsageLimitError,
};

use crate::definition::{Definition, DefinitionError, MemoryProfile};
use crate::format::{validate_drisl, CodecError, DaslCid, DecodeLimits, Id32};
use crate::limits::Limits;
use crate::memory::{Memory, MemoryError};
use crate::state::{AgentState, AgentStateSpec, Continuation, StateError};
use core::fmt;

/// Already materialized exact bytes. Implementations MUST NOT hydrate, perform I/O,
/// reevaluate events or substitute another block. Absence is reported as None.
pub trait MaterializedBlocks {
    fn get(&self, cid: DaslCid) -> Option<&[u8]>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphError {
    Codec(CodecError),
    State(StateError),
    Definition(DefinitionError),
    Memory(MemoryError),
    Episode(crate::episode::EpisodeError),
    Boundary(crate::boundary::BoundaryError),
    Input(crate::input::InputError),
    TargetKindMismatch(DaslCid),
    EpisodeRole(EpisodeRoleError),
    Identity(IdentityError),
    Transition(TransitionError),
    MemoryConsistency(MemoryConsistencyError),
    HistoryConsistency(HistoryConsistencyError),
    UsageLimit(UsageLimitError),
    AdmissionGrant(AdmissionGrantError),
    ToolBinding(ToolBindingError),
    InputUnavailable(DaslCid),
    IntegrityFailure(DaslCid),
    NotGenesis,
    MemoryProfileMismatch,
    MaterializedByteLimit,
    BlockCountLimit,
}

impl From<CodecError> for GraphError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<StateError> for GraphError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<DefinitionError> for GraphError {
    fn from(error: DefinitionError) -> Self {
        Self::Definition(error)
    }
}

impl From<MemoryError> for GraphError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<crate::episode::EpisodeError> for GraphError {
    fn from(error: crate::episode::EpisodeError) -> Self {
        Self::Episode(error)
    }
}

impl From<crate::boundary::BoundaryError> for GraphError {
    fn from(error: crate::boundary::BoundaryError) -> Self {
        Self::Boundary(error)
    }
}

impl From<crate::input::InputError> for GraphError {
    fn from(error: crate::input::InputError) -> Self {
        Self::Input(error)
    }
}

impl From<EpisodeRoleError> for GraphError {
    fn from(error: EpisodeRoleError) -> Self {
        Self::EpisodeRole(error)
    }
}

impl From<IdentityError> for GraphError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<TransitionError> for GraphError {
    fn from(error: TransitionError) -> Self {
        Self::Transition(error)
    }
}

impl From<MemoryConsistencyError> for GraphError {
    fn from(error: MemoryConsistencyError) -> Self {
        Self::MemoryConsistency(error)
    }
}

impl From<HistoryConsistencyError> for GraphError {
    fn from(error: HistoryConsistencyError) -> Self {
        Self::HistoryConsistency(error)
    }
}

impl From<UsageLimitError> for GraphError {
    fn from(error: UsageLimitError) -> Self {
        Self::UsageLimit(error)
    }
}

impl From<AdmissionGrantError> for GraphError {
    fn from(error: AdmissionGrantError) -> Self {
        Self::AdmissionGrant(error)
    }
}

impl From<ToolBindingError> for GraphError {
    fn from(error: ToolBindingError) -> Self {
        Self::ToolBinding(error)
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::State(error) => error.fmt(f),
            Self::Definition(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
            Self::Episode(error) => error.fmt(f),
            Self::Boundary(error) => error.fmt(f),
            Self::Input(error) => error.fmt(f),
            Self::EpisodeRole(error) => error.fmt(f),
            Self::Identity(error) => error.fmt(f),
            Self::Transition(error) => error.fmt(f),
            Self::MemoryConsistency(error) => error.fmt(f),
            Self::HistoryConsistency(error) => error.fmt(f),
            Self::UsageLimit(error) => error.fmt(f),
            Self::AdmissionGrant(error) => error.fmt(f),
            Self::ToolBinding(error) => error.fmt(f),
            Self::TargetKindMismatch(cid) => {
                write!(f, "CID is referenced under incompatible typed roles: {cid}")
            }
            Self::InputUnavailable(cid) => write!(f, "required block is unavailable: {cid}"),
            Self::IntegrityFailure(cid) => write!(f, "block bytes do not match CID: {cid}"),
            Self::NotGenesis => {
                f.write_str("root is not revision-zero AwaitActor with empty history")
            }
            Self::MemoryProfileMismatch => {
                f.write_str("genesis memory presence disagrees with Definition profile")
            }
            Self::MaterializedByteLimit => {
                f.write_str("required closure exceeds aggregate encoded-byte limit")
            }
            Self::BlockCountLimit => f.write_str("required closure exceeds block-count limit"),
        }
    }
}

impl core::error::Error for GraphError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::State(e) => Some(e),
            Self::Definition(e) => Some(e),
            Self::Memory(e) => Some(e),
            Self::Episode(e) => Some(e),
            Self::Boundary(e) => Some(e),
            Self::Input(e) => Some(e),
            Self::EpisodeRole(e) => Some(e),
            Self::Identity(e) => Some(e),
            Self::Transition(e) => Some(e),
            Self::MemoryConsistency(e) => Some(e),
            Self::HistoryConsistency(e) => Some(e),
            Self::UsageLimit(e) => Some(e),
            Self::AdmissionGrant(e) => Some(e),
            Self::ToolBinding(e) => Some(e),
            _ => None,
        }
    }
}

/// Complete revision-zero typed closure, not registered identity or authoritative HEAD.
/// The selected Definition and optional initial Memory are retained exactly. No Episode
/// is invented, no transcript/model/tool is invoked, and unrelated store blocks are ignored.
/// Post-genesis roots require a future full restorer and are explicitly rejected here.
#[derive(Debug, Eq, PartialEq)]
pub struct Genesis {
    state: AgentState,
    definition: Definition,
    memory: Option<Memory>,
}

impl Genesis {
    /// Pure construction. Registration and refusing to replace an existing agent are host duties.
    /// Caller bounds and linked Definition resource limits both apply, never widen each other.
    ///
    /// ```
    /// use raiv::graph::Genesis;
    /// use raiv::definition::{Definition, DefinitionSpec, MemoryProfile};
    /// use raiv::format::DecodeLimits;
    /// use raiv::limits::Limits;
    /// let definition = Definition::encode(&DefinitionSpec {
    ///     execution_domain: [1; 32], policy: "fixture", mandate: "fixture",
    ///     memory_profile: MemoryProfile::None, tools: &[], inference_profile: &[0xa0],
    ///     limits: Limits::default(),
    /// }, &DecodeLimits::default())?;
    /// let genesis = Genesis::build([2; 32], definition, None, &Limits::default())?;
    /// assert_eq!(genesis.state().revision(), 0);
    /// assert_eq!(genesis.blocks().count(), 2);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn build(
        agent_id: Id32,
        definition: Definition,
        memory: Option<Memory>,
        limits: &Limits,
    ) -> Result<Self, GraphError> {
        let caps = Caps::from_limits(limits).tightened(definition.limits());
        check_memory_profile(definition.memory_profile(), memory.is_some())?;
        let count = if memory.is_some() { 3 } else { 2 };
        if count > caps.blocks {
            return Err(GraphError::BlockCountLimit);
        }
        validate_drisl(definition.as_bytes(), &caps.wire)?;
        let mut bytes = definition.as_bytes().len() as u64;
        if let Some(memory) = &memory {
            validate_drisl(memory.as_bytes(), &caps.memory_bounds())?;
            bytes = bytes
                .checked_add(memory.as_bytes().len() as u64)
                .ok_or(GraphError::MaterializedByteLimit)?;
        }
        let remaining = caps
            .bytes
            .checked_sub(bytes)
            .ok_or(GraphError::MaterializedByteLimit)?;
        let bounds = DecodeLimits {
            max_block_bytes: caps.wire.max_block_bytes.min(remaining),
            ..caps.wire
        };
        let state = AgentState::encode(
            &AgentStateSpec {
                agent_id,
                revision: 0,
                definition: definition.cid(),
                memory: memory.as_ref().map(Memory::cid),
                continuation: Continuation::AwaitActor,
                current_episode: None,
                history: &[],
            },
            &bounds,
        )
        .map_err(|error| {
            if error == StateError::Codec(CodecError::ByteLimit)
                && remaining < caps.wire.max_block_bytes
            {
                GraphError::MaterializedByteLimit
            } else {
                GraphError::State(error)
            }
        })?;
        Ok(Self {
            state,
            definition,
            memory,
        })
    }

    /// Restore only a complete genesis closure. A failed load/validation never returns
    /// partial success, guessed Memory or a replacement genesis. Aggregate limits count
    /// encoded required-block bytes, not measured allocator/RAM high-water usage.
    pub fn restore(
        root: DaslCid,
        blocks: &impl MaterializedBlocks,
        limits: &Limits,
    ) -> Result<Self, GraphError> {
        root.require_state().map_err(|_| CodecError::InvalidLink)?;
        let mut budget = Budget {
            caps: Caps::from_limits(limits),
            bytes: 0,
            blocks: 0,
        };
        let root_bytes = read(blocks, root, &mut budget)?;
        let state = AgentState::decode(root_bytes, &budget.caps.wire)?;
        if state.revision() != 0
            || state.continuation() != Continuation::AwaitActor
            || state.current_episode_link().is_some()
            || state.history_count() != 0
        {
            return Err(GraphError::NotGenesis);
        }
        let definition_bytes = read(blocks, state.definition_link(), &mut budget)?;
        let definition = Definition::decode(definition_bytes, &budget.caps.wire)?;
        budget.caps = budget.caps.tightened(definition.limits());
        budget.check_totals()?;
        validate_drisl(root_bytes, &budget.caps.wire)?;
        validate_drisl(definition_bytes, &budget.caps.wire)?;
        check_memory_profile(definition.memory_profile(), state.memory_link().is_some())?;
        let memory = match state.memory_link() {
            Some(cid) => Some(Memory::decode(
                read(blocks, cid, &mut budget)?,
                &budget.caps.memory_bounds(),
            )?),
            None => None,
        };
        Ok(Self {
            state,
            definition,
            memory,
        })
    }

    pub fn root(&self) -> DaslCid {
        self.state.cid()
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    pub fn definition(&self) -> &Definition {
        &self.definition
    }

    pub fn memory(&self) -> Option<&Memory> {
        self.memory.as_ref()
    }

    /// Exact required block set, no duplicate previous roots or hidden archive links.
    pub fn blocks(&self) -> impl Iterator<Item = (DaslCid, &[u8])> {
        [
            (self.state.cid(), self.state.as_bytes()),
            (self.definition.cid(), self.definition.as_bytes()),
        ]
        .into_iter()
        .chain(
            self.memory
                .as_ref()
                .map(|memory| (memory.cid(), memory.as_bytes())),
        )
    }
}

fn check_memory_profile(profile: MemoryProfile, present: bool) -> Result<(), GraphError> {
    if (profile == MemoryProfile::IntentsStrategyTactics) == present {
        Ok(())
    } else {
        Err(GraphError::MemoryProfileMismatch)
    }
}

#[derive(Clone, Copy)]
struct Caps {
    wire: DecodeLimits,
    bytes: u64,
    blocks: u64,
    memory: u64,
}

impl Caps {
    fn from_limits(limits: &Limits) -> Self {
        Self {
            wire: limits.decode_limits(),
            bytes: limits.max_materialized_bytes,
            blocks: limits.max_blocks,
            memory: limits.max_memory_bytes,
        }
    }

    fn tightened(self, limits: &Limits) -> Self {
        Self {
            wire: DecodeLimits {
                max_block_bytes: self.wire.max_block_bytes.min(limits.max_block_bytes),
                max_depth: self.wire.max_depth.min(limits.max_depth),
                max_collection_entries: self
                    .wire
                    .max_collection_entries
                    .min(limits.max_collection_entries),
            },
            bytes: self.bytes.min(limits.max_materialized_bytes),
            blocks: self.blocks.min(limits.max_blocks),
            memory: self.memory.min(limits.max_memory_bytes),
        }
    }

    fn memory_bounds(self) -> DecodeLimits {
        DecodeLimits {
            max_block_bytes: self.wire.max_block_bytes.min(self.memory),
            ..self.wire
        }
    }
}

struct Budget {
    caps: Caps,
    bytes: u64,
    blocks: u64,
}

impl Budget {
    fn check_totals(&self) -> Result<(), GraphError> {
        if self.blocks > self.caps.blocks {
            return Err(GraphError::BlockCountLimit);
        }
        if self.bytes > self.caps.bytes {
            return Err(GraphError::MaterializedByteLimit);
        }
        Ok(())
    }

    fn admit(&mut self, bytes: &[u8]) -> Result<(), GraphError> {
        if bytes.len() as u64 > self.caps.wire.max_block_bytes {
            return Err(CodecError::ByteLimit.into());
        }
        self.blocks = self
            .blocks
            .checked_add(1)
            .ok_or(GraphError::BlockCountLimit)?;
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(GraphError::MaterializedByteLimit)?;
        self.check_totals()
    }
}

fn read<'a>(
    store: &'a impl MaterializedBlocks,
    cid: DaslCid,
    budget: &mut Budget,
) -> Result<&'a [u8], GraphError> {
    if budget.blocks >= budget.caps.blocks {
        return Err(GraphError::BlockCountLimit);
    }
    let bytes = store.get(cid).ok_or(GraphError::InputUnavailable(cid))?;
    budget.admit(bytes)?;
    cid.verify(bytes)
        .map_err(|_| GraphError::IntegrityFailure(cid))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
