//! Volatile local simulator intent/terminal book, NOT a durable/live dispatch permit.
//! No effects, virtual spending, callbacks, automatic retries, pricing or ledger recovery.
use crate::{
    admission::{AdmissionBook, AdmissionStatus},
    checkpoint::ManifestLimits,
    disk::DiskBlockStore,
    policy::{current_pin, PolicyError},
    registry::{Anchor, RegisteredAgent},
    run::ProgramBinding,
};
use raiv::{
    boundary::Exit,
    command::{Command, CommandError},
    format::Id32,
    limits::Limits,
    receipt::{Receipt, ReceiptError, ReceiptView},
    state::Continuation,
};
use std::{fmt, sync::atomic::AtomicU64};
static NEXT_BOOK: AtomicU64 = AtomicU64::new(1);
pub mod checkpoint;
pub mod local;
pub mod recovery;
#[cfg(test)]
mod tests;
pub mod world;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ready,
    Submitted,
    Terminal,
}
/// Process-local correlation only; cannot be reconstructed from command/receipt bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionTicket {
    origin: u64,
    index: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionRecord {
    pub anchor: Anchor,
    pub action_id: Id32,
    pub command_digest: Id32,
    pub status: Status,
    pub submission_intents: u64,
}
struct Entry {
    anchor: Anchor,
    program: ProgramBinding,
    command: Command,
    limits: Limits,
    intents: u64,
    receipt: Option<Receipt>,
}
impl Entry {
    fn same_binding(&self, other: &Self) -> bool {
        self.anchor == other.anchor
            && self.program == other.program
            && self.command.as_bytes() == other.command.as_bytes()
            && self.limits == other.limits
    }
    fn record(&self) -> ExecutionRecord {
        ExecutionRecord {
            anchor: self.anchor,
            action_id: self.command.action_id(),
            command_digest: raiv::identity::command_digest(
                self.program.execution_domain,
                &self.command,
            ),
            status: if self.receipt.is_some() {
                Status::Terminal
            } else if self.intents == 1 {
                Status::Submitted
            } else {
                Status::Ready
            },
            submission_intents: self.intents,
        }
    }
}
#[derive(Debug)]
pub enum SimulatorError {
    Capacity,
    OriginExhausted,
    Allocation,
    WrongPhase,
    Busy,
    Binding,
    ForeignTicket,
    AlreadySubmitted,
    NotSubmitted,
    ReceiptMismatch,
    TerminalConflict,
    Command(CommandError),
    Receipt(ReceiptError),
    Policy(PolicyError),
    Admission(crate::admission::AdmissionError),
}
impl fmt::Display for SimulatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Capacity => "simulator execution capacity must be positive and available",
            Self::OriginExhausted => "process simulator identity exhausted",
            Self::Allocation => "simulator metadata allocation failure",
            Self::WrongPhase => "simulator intent requires a committed pending Command",
            Self::Busy => "agent has unresolved admission or simulator execution",
            Self::Binding => "ActionId conflicts with recorded anchor/program/command",
            Self::ForeignTicket => "action ticket belongs to another simulator book",
            Self::AlreadySubmitted => {
                "submission intent already recorded; unknown is not retry permission"
            }
            Self::NotSubmitted => "terminal receipt requires a prior simulator submission intent",
            Self::ReceiptMismatch => "terminal receipt does not match recorded exact Command",
            Self::TerminalConflict => {
                "terminal receipt is immutable and conflicts with prior outcome"
            }
            Self::Command(e) => return e.fmt(f),
            Self::Receipt(e) => return e.fmt(f),
            Self::Policy(e) => return e.fmt(f),
            Self::Admission(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for SimulatorError {}
/// Mutable possession is the explicit LOCAL simulator administrative port. Never expose
/// it to provider/guest payloads. Drop/restart loses this entire volatile book: no live
/// backend may use it without a durable, world-coupled ledger and recovery protocol.
pub struct ExecutorBook {
    origin: u64,
    capacity: usize,
    entries: Vec<Entry>,
}
impl ExecutorBook {
    pub fn new(capacity: usize) -> Result<Self, SimulatorError> {
        if capacity == 0 {
            return Err(SimulatorError::Capacity);
        }
        let origin =
            crate::atomic::next_u64(&NEXT_BOOK).map_err(|_| SimulatorError::OriginExhausted)?;
        Ok(Self {
            origin,
            capacity,
            entries: Vec::new(),
        })
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Native current approval + exact file/full closure agreement, not supplied candidate
    /// or a caller-selected Command. An unresolved inference/receipt admission blocks this
    /// gate. Identical pending slots reuse their ticket/status, never reset intent/outcome.
    pub fn authorize_pending(
        &mut self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        disk: &mut DiskBlockStore,
        manifest_limits: &ManifestLimits,
        caller: &Limits,
    ) -> Result<ActionTicket, SimulatorError> {
        let entry = Self::pending_entry(registered, admissions, caller)?;
        // Preflight collision/capacity before any durability acknowledgement or publication.
        let index = self.stage(&entry)?;
        current_pin(registered, admissions, disk, manifest_limits, caller)
            .map_err(SimulatorError::Policy)?;
        Ok(self.publish(index, entry))
    }
    fn pending_entry(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        caller: &Limits,
    ) -> Result<Entry, SimulatorError> {
        let source = registered.snapshot();
        if !matches!(
            source.state().continuation(),
            Continuation::AwaitAction { .. }
        ) {
            return Err(SimulatorError::WrongPhase);
        }
        if admissions
            .records_for(registered)
            .map_err(SimulatorError::Admission)?
            .any(|r| r.status != AdmissionStatus::Consumed)
        {
            return Err(SimulatorError::Busy);
        }
        let episode = source.current_episode().expect("restored pending Episode");
        let boundary = source
            .boundary(
                episode
                    .boundaries()
                    .last()
                    .expect("restored pending boundary"),
            )
            .expect("restored boundary");
        let Exit::AwaitAction(bytes) = boundary.exit() else {
            return Err(SimulatorError::WrongPhase);
        };
        let limits = Self::execution_limits(registered, caller);
        let command = Command::decode(bytes, &limits).map_err(SimulatorError::Command)?;
        Ok(Entry {
            anchor: registered.anchor(),
            program: registered.program_binding(),
            command,
            limits,
            intents: 0,
            receipt: None,
        })
    }
    fn execution_limits(registered: &RegisteredAgent, caller: &Limits) -> Limits {
        let d = registered.snapshot().definition().limits();
        Limits {
            max_command_bytes: caller.max_command_bytes.min(d.max_command_bytes),
            max_receipt_bytes: caller.max_receipt_bytes.min(d.max_receipt_bytes),
            max_block_bytes: caller.max_block_bytes.min(d.max_block_bytes),
            max_depth: caller.max_depth.min(d.max_depth),
            max_collection_entries: caller.max_collection_entries.min(d.max_collection_entries),
            ..*caller
        }
    }
    fn stage(&mut self, entry: &Entry) -> Result<usize, SimulatorError> {
        if let Some(index) = self.entries.iter().position(|e| {
            e.anchor.agent_id == entry.anchor.agent_id
                && e.program.execution_domain == entry.program.execution_domain
                && e.command.action_id() == entry.command.action_id()
        }) {
            let old = &self.entries[index];
            if !old.same_binding(entry) {
                return Err(SimulatorError::Binding);
            }
            return Ok(index);
        }
        if self
            .entries
            .iter()
            .any(|e| e.anchor.agent_id == entry.anchor.agent_id && e.receipt.is_none())
        {
            return Err(SimulatorError::Busy);
        }
        if self.entries.len() >= self.capacity {
            return Err(SimulatorError::Capacity);
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| SimulatorError::Allocation)?;
        Ok(self.entries.len())
    }
    fn publish(&mut self, index: usize, entry: Entry) -> ActionTicket {
        if index == self.entries.len() {
            self.entries.push(entry);
        }
        ActionTicket {
            origin: self.origin,
            index,
        }
    }
    fn index(&self, ticket: ActionTicket) -> Result<usize, SimulatorError> {
        if ticket.origin != self.origin || ticket.index >= self.entries.len() {
            return Err(SimulatorError::ForeignTicket);
        }
        Ok(ticket.index)
    }
    pub fn record(&self, ticket: ActionTicket) -> Result<ExecutionRecord, SimulatorError> {
        Ok(self.entries[self.index(ticket)?].record())
    }
    /// Recheck LOCAL current head/admission/Command/policy and exact disk readiness just
    /// before recording the RAM intent. A captured ticket alone is not current approval.
    /// Still no durable effect ledger, backend call, pricing or live dispatch permit.
    pub fn begin_current(
        &mut self,
        ticket: ActionTicket,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        disk: &mut DiskBlockStore,
        manifest_limits: &ManifestLimits,
        caller: &Limits,
    ) -> Result<&Command, SimulatorError> {
        let index = self.ready_current(ticket, registered, admissions, caller)?;
        current_pin(registered, admissions, disk, manifest_limits, caller)
            .map_err(SimulatorError::Policy)?;
        Ok(self.mark_submitted(index))
    }
    fn ready_current(
        &self,
        ticket: ActionTicket,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        caller: &Limits,
    ) -> Result<usize, SimulatorError> {
        let index = self.index(ticket)?;
        let old = &self.entries[index];
        if old.intents != 0 || old.receipt.is_some() {
            return Err(SimulatorError::AlreadySubmitted);
        }
        let current = Self::pending_entry(registered, admissions, caller)?;
        if !old.same_binding(&current) {
            return Err(SimulatorError::Binding);
        }
        Ok(index)
    }
    fn mark_submitted(&mut self, index: usize) -> &Command {
        let entry = &mut self.entries[index];
        entry.intents = 1;
        &entry.command
    }
    /// LOW-LEVEL volatile bookkeeping only: no current-head/admission/storage recheck.
    /// One RAM intent, not a backend call or charge. Unknown remains Submitted; there
    /// is no timeout->failure/refund/retry method. Exact accepted Command bytes survive.
    pub fn begin(&mut self, ticket: ActionTicket) -> Result<&Command, SimulatorError> {
        let index = self.index(ticket)?;
        let entry = &mut self.entries[index];
        if entry.intents != 0 || entry.receipt.is_some() {
            return Err(SimulatorError::AlreadySubmitted);
        }
        Ok(self.mark_submitted(index))
    }
    /// Explicit local terminal DATA approval. Matching proves no external effect/finality.
    /// Bound full receipt validation precedes ownership; duplicate exact outcome is known,
    /// conflicting outcome refuses, and a terminal slot never becomes submitted again.
    pub fn resolve(
        &mut self,
        ticket: ActionTicket,
        receipt: Receipt,
    ) -> Result<ExecutionRecord, SimulatorError> {
        let (index, known) = self.terminal_candidate(ticket, &receipt)?;
        if !known {
            self.entries[index].receipt = Some(receipt);
        }
        Ok(self.entries[index].record())
    }
    fn terminal_candidate(
        &self,
        ticket: ActionTicket,
        receipt: &Receipt,
    ) -> Result<(usize, bool), SimulatorError> {
        let index = self.index(ticket)?;
        let entry = &self.entries[index];
        ReceiptView::decode(receipt.as_bytes(), &entry.limits).map_err(SimulatorError::Receipt)?;
        if !receipt.matches_command(entry.program.execution_domain, &entry.command) {
            return Err(SimulatorError::ReceiptMismatch);
        }
        if let Some(old) = &entry.receipt {
            return if old.as_bytes() == receipt.as_bytes() {
                Ok((index, true))
            } else {
                Err(SimulatorError::TerminalConflict)
            };
        }
        if entry.intents != 1 {
            return Err(SimulatorError::NotSubmitted);
        }
        Ok((index, false))
    }
    pub fn receipt(&self, ticket: ActionTicket) -> Result<Option<&Receipt>, SimulatorError> {
        Ok(self.entries[self.index(ticket)?].receipt.as_ref())
    }
}
