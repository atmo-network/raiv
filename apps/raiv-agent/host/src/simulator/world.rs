//! Primary one-file SCRIPTED virtual-unit world + exact executor/Receipt ledger.
//! Synthetic quote=1, no money/market/portfolio/wake/backend/provenance or live permit.

use super::{
    checkpoint::{ExecutorCheckpoint, ExecutorCheckpointError, ExecutorLimits},
    local::LocalExecutorLimits,
    *,
};
use crate::{
    disk::{
        manifest::{ManifestError, ManifestFile, Publication},
        DiskError,
    },
    recovery::LocalRecoveryPin,
};
use raiv::{
    format::{CodecError, DecodeLimits},
    position::Position,
    receipt::{Outcome, ReceiptSpec},
};

mod admission;

pub use admission::LedgerActionResult;

mod recovery;

pub use recovery::WorldRecoveryPin;

#[cfg(test)]
mod tests;
mod wire;

#[derive(Debug)]
pub enum WorldError {
    Schema,
    Limit,
    Allocation,
    Binding,
    Missing,
    NotTerminal,
    Conflict,
    Exhausted,
    NotGenesis,
    NonEmptyDirectory,
    Simulator(SimulatorError),
    Checkpoint(ExecutorCheckpointError),
    Codec(CodecError),
    Disk(DiskError),
    Manifest(ManifestError),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Schema => "invalid scripted world schema or virtual-unit costs",
            Self::Limit => "scripted world frame capacity exceeded",
            Self::Allocation => "scripted world allocation failure",
            Self::Binding => "scripted world/native source binding disagrees",
            Self::Missing => "scripted world missing; explicit pinned recovery required",
            Self::NotTerminal => {
                "scripted world terminal receipt not installed; no ActionResult admission"
            }
            Self::Conflict => {
                "scripted world differs from exact old/next; never rollback or replay"
            }
            Self::Exhausted => "scripted virtual-unit capacity exhausted; no reservation",
            Self::NotGenesis => "scripted world bootstrap requires approved empty main genesis",
            Self::NonEmptyDirectory => {
                "scripted world absent in nonempty directory; explicit recovery required"
            }
            Self::Simulator(e) => return e.fmt(f),
            Self::Checkpoint(e) => return e.fmt(f),
            Self::Codec(e) => return e.fmt(f),
            Self::Disk(e) => return e.fmt(f),
            Self::Manifest(e) => return e.fmt(f),
        })
    }
}

impl std::error::Error for WorldError {}

/// DATA only on reading: independent world identity/seed must be selected by native policy.
/// Available = initial - known virtual debits - unresolved one-unit reservations.
/// The exact ledger contains each terminal debit in closed virtual_units costs; unknown
/// Submitted retains one reserved unit, never an inferred zero cost or refund.
pub struct WorldState {
    id: Id32,
    initial: u64,
    available: u64,
    ledger: ExecutorCheckpoint,
}

impl WorldState {
    fn compose(
        id: Id32,
        initial: u64,
        ledger: ExecutorCheckpoint,
        limits: &ExecutorLimits,
    ) -> Result<Self, WorldError> {
        let view = ExecutorCheckpoint::borrowed(ledger.as_bytes(), limits)
            .map_err(WorldError::Checkpoint)?;
        let used = wire::used(&view, &limits.wire, id)?;
        let available = initial.checked_sub(used).ok_or(WorldError::Exhausted)?;
        Ok(Self {
            id,
            initial,
            available,
            ledger,
        })
    }

    pub fn decode(bytes: &[u8], limits: &ExecutorLimits) -> Result<Self, WorldError> {
        wire::decode(bytes, limits)
    }

    pub fn encode(&self, limits: &ExecutorLimits) -> Result<Vec<u8>, WorldError> {
        wire::encode(self, limits)
    }

    pub fn world_id(&self) -> Id32 {
        self.id
    }

    pub fn initial_units(&self) -> u64 {
        self.initial
    }

    pub fn available_units(&self) -> u64 {
        self.available
    }

    pub fn ledger(&self) -> &ExecutorCheckpoint {
        &self.ledger
    }

    pub fn read(store: &mut DiskBlockStore, limits: &ExecutorLimits) -> Result<Self, WorldError> {
        let bytes = ManifestFile::world(store, limits.wire)
            .read()
            .map_err(WorldError::Disk)?
            .ok_or(WorldError::Missing)?;
        Self::decode(&bytes, limits)
    }
}

/// Explicit native seed approval; hash/pin construction is not authentication/currentness.
pub struct WorldGenesis {
    pub world_id: Id32,
    pub virtual_units: u64,
    pub capacity: usize,
    pub source: LocalRecoveryPin,
}

/// Scripted native terminal plan, not chosen by guest/provider arguments. Debit0/1 is a
/// known virtual result bounded by the fixed synthetic quote, not an actual monetary bill.
#[derive(Clone, Copy)]
pub struct ScriptedResult<'a> {
    pub outcome: Outcome,
    pub result: &'a [u8],
    pub debit: u64,
}

/// Hold the independent world's mutable FD/lock; no externally mutable executor book.
/// Its native identity/tickets are RAM-only. There is no auto-adoption or late bootstrap.
pub struct ScriptedWorld<'a> {
    store: &'a mut DiskBlockStore,
    state: WorldState,
    book: ExecutorBook,
}

impl<'a> ScriptedWorld<'a> {
    pub fn bootstrap(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        world_store: &'a mut DiskBlockStore,
        genesis: &WorldGenesis,
        limits: &LocalExecutorLimits,
    ) -> Result<Self, WorldError> {
        let state = registered.snapshot().state();
        if state.revision() != 0
            || state.continuation() != Continuation::AwaitActor
            || state.current_episode_link().is_some()
            || state.history_count() != 0
            || admissions
                .records_for(registered)
                .map_err(|e| WorldError::Simulator(SimulatorError::Admission(e)))?
                .next()
                .is_some()
        {
            return Err(WorldError::NotGenesis);
        }
        if !genesis.source.matches_registered(registered) {
            return Err(WorldError::Binding);
        }
        let book = ExecutorBook::new(genesis.capacity).map_err(WorldError::Simulator)?;
        let ledger = ExecutorCheckpoint::capture(registered, admissions, &book, &limits.executor)
            .map_err(WorldError::Checkpoint)?;
        let state = WorldState::compose(
            genesis.world_id,
            genesis.virtual_units,
            ledger,
            &limits.executor,
        )?;
        let bytes = state.encode(&limits.executor)?;
        let current = ManifestFile::world(world_store, limits.executor.wire)
            .read()
            .map_err(WorldError::Disk)?;
        match current.as_deref() {
            Some(data) if data == bytes => {}
            Some(_) => return Err(WorldError::Conflict),
            None => {
                if !world_store.directory_is_empty().map_err(WorldError::Disk)? {
                    return Err(WorldError::NonEmptyDirectory);
                }
            }
        }
        let source = current_pin(
            registered,
            admissions,
            agent_store,
            &limits.manifest,
            &limits.caller,
        )
        .map_err(|e| WorldError::Simulator(SimulatorError::Policy(e)))?;
        if source.manifest_digest != genesis.source.manifest_digest {
            return Err(WorldError::Binding);
        }
        ManifestFile::world(world_store, limits.executor.wire)
            .replace(current.as_deref(), &bytes)
            .map_err(WorldError::Manifest)?;
        Ok(Self {
            store: world_store,
            state,
            book,
        })
    }

    #[cfg(test)]
    pub(crate) fn fail_after_publish(&mut self, fail: bool) {
        self.store.fail_manifest_after_publish = fail;
    }

    pub fn state(&self) -> &WorldState {
        &self.state
    }

    pub fn record(&self, ticket: ActionTicket) -> Result<ExecutionRecord, WorldError> {
        self.book.record(ticket).map_err(WorldError::Simulator)
    }

    pub fn receipt(&self, ticket: ActionTicket) -> Result<Option<&Receipt>, WorldError> {
        self.book.receipt(ticket).map_err(WorldError::Simulator)
    }

    fn check_scope(&self, registered: &RegisteredAgent) -> Result<(), WorldError> {
        if self.state.ledger.agent_id() != registered.anchor().agent_id
            || self.state.ledger.program_binding() != registered.program_binding()
        {
            return Err(WorldError::Binding);
        }
        Ok(())
    }

    fn source_ready(
        &self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        limits: &LocalExecutorLimits,
    ) -> Result<(), WorldError> {
        current_pin(
            registered,
            admissions,
            agent_store,
            &limits.manifest,
            &limits.caller,
        )
        .map_err(|e| WorldError::Simulator(SimulatorError::Policy(e)))?;
        Ok(())
    }

    fn publish(
        &mut self,
        next: &WorldState,
        limits: &ExecutorLimits,
    ) -> Result<Publication, WorldError> {
        let old = self.state.encode(limits)?;
        let bytes = next.encode(limits)?;
        // Complete typed preflight before write; unknown keeps exact old RAM for retry.
        let stored = WorldState::read(self.store, limits)?.encode(limits)?;
        if stored != old && stored != bytes {
            return Err(WorldError::Conflict);
        }
        ManifestFile::world(self.store, limits.wire)
            .replace(Some(&old), &bytes)
            .map_err(WorldError::Manifest)
    }

    pub fn authorize_pending(
        &mut self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        limits: &LocalExecutorLimits,
    ) -> Result<ActionTicket, WorldError> {
        self.check_scope(registered)?;
        if self.state.available == 0 {
            let pending = ExecutorBook::pending_entry(registered, admissions, &limits.caller)
                .map_err(WorldError::Simulator)?;
            if self.book.stage(&pending).map_err(WorldError::Simulator)? == self.book.entries.len()
            {
                return Err(WorldError::Exhausted);
            }
        }
        let ticket = self
            .book
            .authorize_pending(
                registered,
                admissions,
                agent_store,
                &limits.manifest,
                &limits.caller,
            )
            .map_err(WorldError::Simulator)?;
        let ledger =
            ExecutorCheckpoint::capture(registered, admissions, &self.book, &limits.executor)
                .map_err(WorldError::Checkpoint)?;
        let next =
            WorldState::compose(self.state.id, self.state.initial, ledger, &limits.executor)?;
        self.publish(&next, &limits.executor)?;
        self.state = next;
        Ok(ticket)
    }

    /// Persist one-unit reservation + Submitted BEFORE exposing even a submission record.
    /// Unknown conservatively installs that RAM reservation/intent without exposure; never
    /// treat already Submitted file bytes as fresh permission from a Ready RAM book.
    pub fn begin(
        &mut self,
        ticket: ActionTicket,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        limits: &LocalExecutorLimits,
    ) -> Result<ExecutionRecord, WorldError> {
        self.check_scope(registered)?;
        let index = self
            .book
            .ready_current(ticket, registered, admissions, &limits.caller)
            .map_err(WorldError::Simulator)?;
        if self.state.available == 0 {
            return Err(WorldError::Exhausted);
        }
        let old = ExecutorCheckpoint::capture(registered, admissions, &self.book, &limits.executor)
            .map_err(WorldError::Checkpoint)?;
        if old.as_bytes() != self.state.ledger.as_bytes() {
            return Err(WorldError::Conflict);
        }
        let ledger = ExecutorCheckpoint::capture_stage(
            registered,
            admissions,
            &self.book,
            &limits.executor,
            Some(self.book.entries[index].command.action_id()),
        )
        .map_err(WorldError::Checkpoint)?;
        let next =
            WorldState::compose(self.state.id, self.state.initial, ledger, &limits.executor)?;
        // Require exact old bytes, not a pre-existing Submitted acknowledgement.
        if WorldState::read(self.store, &limits.executor)?.encode(&limits.executor)?
            != self.state.encode(&limits.executor)?
        {
            return Err(WorldError::Conflict);
        }
        self.source_ready(registered, admissions, agent_store, limits)?;
        match self.publish(&next, &limits.executor) {
            Ok(Publication::Replaced) => {
                self.book.mark_submitted(index);
                self.state = next;
                Ok(self.book.entries[index].record())
            }
            Ok(Publication::AlreadyPresent) => {
                self.book.mark_submitted(index);
                self.state = next;
                Err(WorldError::Conflict)
            }
            Err(error @ WorldError::Manifest(ManifestError::UnknownOutcome(_))) => {
                self.book.mark_submitted(index);
                self.state = next;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// The virtual effect (known debit/release) and exact Receipt live in ONE atomic file.
    /// No budget mutation/Receipt installation before fsync; unknown retains Submitted and
    /// its reservation. Identical next acknowledges without spending twice; conflict refuses.
    pub fn settle(
        &mut self,
        ticket: ActionTicket,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        result: ScriptedResult<'_>,
        limits: &LocalExecutorLimits,
    ) -> Result<ExecutionRecord, WorldError> {
        self.check_scope(registered)?;
        let index = self.book.index(ticket).map_err(WorldError::Simulator)?;
        if result.debit > 1 || result.outcome == Outcome::NotExecuted && result.debit != 0 {
            return Err(WorldError::Schema);
        }
        let entry = &self.book.entries[index];
        if entry.limits != ExecutorBook::execution_limits(registered, &limits.caller) {
            return Err(WorldError::Binding);
        }
        let costs = wire::costs(result.debit)?;
        let receipt = Receipt::encode(
            &ReceiptSpec {
                action_id: entry.command.action_id(),
                command_digest: entry.record().command_digest,
                outcome: result.outcome,
                result: result.result,
                execution_reference: None,
                finalized_at: Position {
                    domain: self.state.id,
                    number: entry.anchor.revision,
                    reference: None,
                },
                costs: &costs,
            },
            &entry.limits,
        )
        .map_err(|e| WorldError::Simulator(SimulatorError::Receipt(e)))?;
        let (index, known) = self
            .book
            .terminal_candidate(ticket, &receipt)
            .map_err(WorldError::Simulator)?;
        let old = ExecutorCheckpoint::capture(registered, admissions, &self.book, &limits.executor)
            .map_err(WorldError::Checkpoint)?;
        if old.as_bytes() != self.state.ledger.as_bytes() {
            return Err(WorldError::Conflict);
        }
        let ledger = if known {
            old
        } else {
            ExecutorCheckpoint::capture_terminal(
                registered,
                admissions,
                &self.book,
                &limits.executor,
                entry.command.action_id(),
                &receipt,
            )
            .map_err(WorldError::Checkpoint)?
        };
        let next =
            WorldState::compose(self.state.id, self.state.initial, ledger, &limits.executor)?;
        self.source_ready(registered, admissions, agent_store, limits)?;
        self.publish(&next, &limits.executor)?;
        if !known {
            self.book.entries[index].receipt = Some(receipt);
        }
        self.state = next;
        Ok(self.book.entries[index].record())
    }
}
