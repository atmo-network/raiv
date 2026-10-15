//! Sealed RAM decision to bounded validated checkpoint data; no I/O or acceptance.

use super::{Activation, InferencePhase, RamDecision};
use crate::{
    boundary::{Boundary, BoundaryError, BoundarySpec, ExitSpec},
    commit::CommitCandidate,
    episode::{Episode, EpisodeError, EpisodeSpec},
    format::{CodecError, DaslCid},
    graph::{GraphError, MaterializedBlocks, RestoredAgent},
    identity::episode_id,
    state::{AgentState, AgentStateSpec, Continuation, StateError},
};
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateError {
    NotSealed,
    RevisionOverflow,
    Codec(CodecError),
    Boundary(BoundaryError),
    Episode(EpisodeError),
    State(StateError),
    Graph(GraphError),
}

impl fmt::Display for CandidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSealed => f.write_str("candidate requires a sealed successful RAM decision"),
            Self::RevisionOverflow => f.write_str("candidate revision overflows u64"),
            Self::Codec(error) => error.fmt(f),
            Self::Boundary(error) => error.fmt(f),
            Self::Episode(error) => error.fmt(f),
            Self::State(error) => error.fmt(f),
            Self::Graph(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for CandidateError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Boundary(error) => Some(error),
            Self::Episode(error) => Some(error),
            Self::State(error) => Some(error),
            Self::Graph(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CodecError> for CandidateError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl Activation<'_> {
    /// Build deterministic immutable candidate DATA after successful RAM sealing.
    /// Does not mutate the Activation, spend another request/tool slot, call any port,
    /// release request IDs, accept HEAD or authorize execution. Failure leaves the
    /// sealed run retryable; persist retries should reuse the returned candidate.
    /// All newly encoded records and the entire resulting closure obey captured caps.
    /// No prefix, trajectory, reply text or native call metadata is serialized.
    pub fn candidate(&self) -> Result<CommitCandidate, CandidateError> {
        if !matches!(
            self.session.phase(),
            InferencePhase::Sealed | InferencePhase::Released
        ) {
            return Err(CandidateError::NotSealed);
        }
        let decision = self
            .ram_decision
            .as_ref()
            .ok_or(CandidateError::NotSealed)?;
        let source = self.snapshot.state();
        let revision = source
            .revision()
            .checked_add(1)
            .ok_or(CandidateError::RevisionOverflow)?;
        let current = self.snapshot.current_episode();
        let ordinal = current.map_or(0, Episode::boundary_count);
        let memory = self.memory.as_ref().map(crate::memory::Memory::cid);
        let (exit, continuation, closed) = match decision {
            RamDecision::Yield(command) => (
                ExitSpec::AwaitAction(command),
                Continuation::AwaitAction {
                    action_id: command.action_id(),
                },
                false,
            ),
            RamDecision::Finish => (ExitSpec::AwaitActor, Continuation::AwaitActor, true),
        };
        let boundary = Boundary::encode(
            &BoundarySpec {
                ordinal,
                input: self.input.input().cid(),
                memory,
                exit,
                usage: self.usage(),
            },
            &self.effective,
        )
        .map_err(CandidateError::Boundary)?;
        let mut boundaries = Vec::new();
        let count = ordinal.checked_add(1).ok_or(CodecError::CollectionLimit)?;
        if count > self.effective.max_collection_entries {
            return Err(CodecError::CollectionLimit.into());
        }
        boundaries
            .try_reserve_exact(usize::try_from(count).map_err(|_| CodecError::AllocationFailure)?)
            .map_err(|_| CodecError::AllocationFailure)?;
        if let Some(current) = current {
            boundaries.extend(current.boundaries());
        }
        boundaries.push(boundary.cid());
        let trigger = current.map_or(self.input.input().event_id(), Episode::trigger_event_id);
        let id = if let Some(current) = current {
            current.id()
        } else {
            episode_id(
                self.snapshot.definition().execution_domain(),
                source.agent_id(),
                trigger,
            )
        };
        let entry_memory = current.map_or(source.memory_link(), Episode::entry_memory_link);
        let episode = Episode::encode(
            &EpisodeSpec {
                id,
                trigger_event_id: trigger,
                entry_memory,
                boundaries: &boundaries,
                closed,
            },
            &self.effective.decode_limits(),
        )
        .map_err(CandidateError::Episode)?;
        let mut history = Vec::new();
        let count = source
            .history_count()
            .checked_add(u64::from(closed))
            .ok_or(CodecError::CollectionLimit)?;
        history
            .try_reserve_exact(usize::try_from(count).map_err(|_| CodecError::AllocationFailure)?)
            .map_err(|_| CodecError::AllocationFailure)?;
        history.extend(source.history());
        if closed {
            history.push(episode.cid());
        }
        let window = self.snapshot.definition().limits().history_episodes.max(1);
        if history.len() as u64 > window {
            let retained = usize::try_from(window).map_err(|_| CodecError::AllocationFailure)?;
            history.drain(..history.len() - retained);
        }
        let state = AgentState::encode(
            &AgentStateSpec {
                agent_id: source.agent_id(),
                revision,
                definition: source.definition_link(),
                memory,
                continuation,
                current_episode: if closed { None } else { Some(episode.cid()) },
                history: &history,
            },
            &self.effective.decode_limits(),
        )
        .map_err(CandidateError::State)?;
        // Borrow encoded records and existing RAM only. Restore performs the bounded
        // ownership copy for the returned immutable closure; no second store read or
        // source mutation occurs. Temporary encoded records are dropped on return.
        let input = self.input.input();
        let additions = [
            (state.cid(), state.as_bytes()),
            (episode.cid(), episode.as_bytes()),
            (boundary.cid(), boundary.as_bytes()),
            (input.cid(), input.as_bytes()),
        ];
        let overlay = Overlay {
            source: &self.snapshot,
            additions: &additions,
            memory: self
                .memory
                .as_ref()
                .map(|memory| (memory.cid(), memory.as_bytes())),
        };
        let snapshot = RestoredAgent::restore(state.cid(), &overlay, &self.policy.caller)
            .map_err(CandidateError::Graph)?;
        CommitCandidate::from_validated(self.input.binding(), snapshot, &self.snapshot)
            .map_err(CandidateError::Codec)
    }
}

struct Overlay<'a> {
    source: &'a RestoredAgent,
    additions: &'a [(DaslCid, &'a [u8])],
    memory: Option<(DaslCid, &'a [u8])>,
}

impl MaterializedBlocks for Overlay<'_> {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.additions
            .iter()
            .find(|(key, _)| *key == cid)
            .map(|(_, bytes)| *bytes)
            .or_else(|| {
                self.memory
                    .filter(|(key, _)| *key == cid)
                    .map(|(_, bytes)| bytes)
            })
            .or_else(|| {
                self.source
                    .blocks()
                    .find(|(key, _)| *key == cid)
                    .map(|(_, bytes)| bytes)
            })
    }
}
