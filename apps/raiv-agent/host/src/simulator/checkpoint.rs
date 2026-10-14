//! Bounded executor checkpoint DATA, not approval, ticket recreation, persistence or effects.
use super::*;
use raiv::format::{validate_drisl, CodecError, DaslCid, DecodeLimits};
use std::ops::Range;
pub mod file;
#[cfg(test)]
mod tests;
mod wire;
#[derive(Clone, Copy, Debug)]
pub struct ExecutorLimits {
    pub wire: DecodeLimits,
    pub max_actions: u64,
}
#[derive(Debug)]
pub enum ExecutorCheckpointError {
    Codec(CodecError),
    Schema,
    Limit,
    Allocation,
    Binding,
    Limits(raiv::limits::LimitsError),
    Simulator(SimulatorError),
}
impl fmt::Display for ExecutorCheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Codec(e) => return e.fmt(f),
            Self::Schema => "invalid executor checkpoint schema/state",
            Self::Limit => "executor checkpoint capacity exceeded",
            Self::Allocation => "executor checkpoint allocation failure",
            Self::Binding => "executor checkpoint/source/command/receipt bindings disagree",
            Self::Limits(e) => return e.fmt(f),
            Self::Simulator(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for ExecutorCheckpointError {}
#[derive(Clone, Copy)]
enum Change<'a> {
    Submitted(Id32),
    Terminal(Id32, &'a Receipt),
}
impl Change<'_> {
    fn action(self) -> Id32 {
        match self {
            Self::Submitted(id) | Self::Terminal(id, _) => id,
        }
    }
}
struct Stored {
    record: ExecutionRecord,
    limits: Limits,
    command: Range<usize>,
    receipt: Option<Range<usize>>,
}
/// Own one unchanged exact encoded frame only AFTER complete borrowed preflight.
/// Records/spans are bounded metadata; no payload tree or native capabilities are stored.
pub struct ExecutorCheckpoint {
    encoded: Vec<u8>,
    agent: Id32,
    program: ProgramBinding,
    rows: Vec<Stored>,
}
pub(super) struct ExecutorCheckpointView<'a> {
    bytes: &'a [u8],
    rows: Vec<Stored>,
}
impl ExecutorCheckpointView<'_> {
    pub(super) fn records(&self) -> impl Iterator<Item = (ExecutionRecord, Option<&[u8]>)> + '_ {
        self.rows.iter().map(|r| {
            (
                r.record,
                r.receipt.as_ref().map(|span| &self.bytes[span.clone()]),
            )
        })
    }
}
impl ExecutorCheckpoint {
    pub(super) fn borrowed<'a>(
        bytes: &'a [u8],
        limits: &ExecutorLimits,
    ) -> Result<ExecutorCheckpointView<'a>, ExecutorCheckpointError> {
        let (_, _, rows) = wire::inspect(bytes, limits)?;
        Ok(ExecutorCheckpointView { bytes, rows })
    }
    pub fn capture(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        book: &ExecutorBook,
        limits: &ExecutorLimits,
    ) -> Result<Self, ExecutorCheckpointError> {
        Self::capture_stage(registered, admissions, book, limits, None)
    }
    pub(super) fn capture_stage(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        book: &ExecutorBook,
        limits: &ExecutorLimits,
        submitted: Option<Id32>,
    ) -> Result<Self, ExecutorCheckpointError> {
        Self::capture_change(
            registered,
            admissions,
            book,
            limits,
            submitted.map(Change::Submitted),
        )
    }
    pub(super) fn capture_terminal(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        book: &ExecutorBook,
        limits: &ExecutorLimits,
        action: Id32,
        receipt: &Receipt,
    ) -> Result<Self, ExecutorCheckpointError> {
        Self::capture_change(
            registered,
            admissions,
            book,
            limits,
            Some(Change::Terminal(action, receipt)),
        )
    }
    fn capture_change(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        book: &ExecutorBook,
        limits: &ExecutorLimits,
        change: Option<Change<'_>>,
    ) -> Result<Self, ExecutorCheckpointError> {
        let id = registered.anchor().agent_id;
        let program = registered.program_binding();
        let mut rows = Vec::new();
        let _ = admissions
            .records_for(registered)
            .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Admission(e)))?;
        for e in book.entries.iter().filter(|e| e.anchor.agent_id == id) {
            if rows.len() as u64 >= limits.max_actions {
                return Err(ExecutorCheckpointError::Limit);
            }
            Self::check_entry(
                registered,
                admissions,
                e.anchor,
                e.program,
                e.command.as_bytes(),
            )?;
            rows.try_reserve(1)
                .map_err(|_| ExecutorCheckpointError::Allocation)?;
            rows.push(e);
        }
        rows.sort_unstable_by_key(|e| e.command.action_id());
        if let Some(change) = change {
            let entry = rows
                .iter()
                .find(|e| e.command.action_id() == change.action())
                .ok_or(ExecutorCheckpointError::Binding)?;
            if entry.receipt.is_some() {
                return Err(ExecutorCheckpointError::Schema);
            }
            match change {
                Change::Submitted(_) if entry.intents != 0 => {
                    return Err(ExecutorCheckpointError::Schema)
                }
                Change::Submitted(_) => {}
                Change::Terminal(_, receipt) => {
                    if entry.intents != 1 {
                        return Err(ExecutorCheckpointError::Schema);
                    }
                    ReceiptView::decode(receipt.as_bytes(), &entry.limits).map_err(|e| {
                        ExecutorCheckpointError::Simulator(SimulatorError::Receipt(e))
                    })?;
                    if !receipt.matches_command(entry.program.execution_domain, &entry.command) {
                        return Err(ExecutorCheckpointError::Binding);
                    }
                }
            }
        }
        let encoded = wire::encode_change(id, program, &rows, limits, change)?;
        Self::from_frame(encoded, limits)
    }
    pub(super) fn check_source(
        &self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
    ) -> Result<(), ExecutorCheckpointError> {
        if self.agent != registered.anchor().agent_id
            || self.program != registered.program_binding()
        {
            return Err(ExecutorCheckpointError::Binding);
        }
        let _ = admissions
            .records_for(registered)
            .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Admission(e)))?;
        for row in &self.rows {
            Self::check_entry(
                registered,
                admissions,
                row.record.anchor,
                self.program,
                &self.encoded[row.command.clone()],
            )?;
        }
        Ok(())
    }
    fn restore_bounds(
        saved: Limits,
        registered: &RegisteredAgent,
        caller: &Limits,
        wire: &DecodeLimits,
    ) -> Limits {
        let current = ExecutorBook::execution_limits(registered, caller);
        Limits {
            max_block_bytes: saved
                .max_block_bytes
                .min(current.max_block_bytes)
                .min(wire.max_block_bytes),
            max_command_bytes: saved
                .max_command_bytes
                .min(current.max_command_bytes)
                .min(wire.max_block_bytes),
            max_receipt_bytes: saved
                .max_receipt_bytes
                .min(current.max_receipt_bytes)
                .min(wire.max_block_bytes),
            max_depth: saved.max_depth.min(current.max_depth).min(wire.max_depth),
            max_collection_entries: saved
                .max_collection_entries
                .min(current.max_collection_entries)
                .min(wire.max_collection_entries),
            ..saved
        }
    }
    /// Shared sidecar/primary-world restoration: whole source/capacity and ALL borrowed
    /// caller-bound nested payload checks precede further Command/Receipt ownership.
    pub(super) fn preflight_restore(
        &self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        caller: &Limits,
        limits: &ExecutorLimits,
        capacity: usize,
    ) -> Result<(), ExecutorCheckpointError> {
        if self.rows.len() > capacity {
            return Err(ExecutorCheckpointError::Simulator(SimulatorError::Capacity));
        }
        self.check_source(registered, admissions)?;
        for row in &self.rows {
            let bound = Self::restore_bounds(row.limits, registered, caller, &limits.wire);
            raiv::command::CommandView::decode(&self.encoded[row.command.clone()], &bound)
                .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Command(e)))?;
            if let Some(span) = &row.receipt {
                ReceiptView::decode(&self.encoded[span.clone()], &bound)
                    .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Receipt(e)))?;
            }
        }
        Ok(())
    }
    pub(super) fn stage_restore(
        &self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        caller: &Limits,
        limits: &ExecutorLimits,
        capacity: usize,
    ) -> Result<Vec<Entry>, ExecutorCheckpointError> {
        self.preflight_restore(registered, admissions, caller, limits, capacity)?;
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(self.rows.len())
            .map_err(|_| ExecutorCheckpointError::Simulator(SimulatorError::Allocation))?;
        for row in &self.rows {
            let bound = Self::restore_bounds(row.limits, registered, caller, &limits.wire);
            let command = Command::decode(&self.encoded[row.command.clone()], &bound)
                .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Command(e)))?;
            let receipt = row
                .receipt
                .as_ref()
                .map(|span| Receipt::decode(&self.encoded[span.clone()], &bound))
                .transpose()
                .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Receipt(e)))?;
            staged.push(Entry {
                anchor: row.record.anchor,
                program: self.program,
                command,
                limits: row.limits,
                intents: row.record.submission_intents,
                receipt,
            });
        }
        Ok(staged)
    }
    fn check_entry(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        anchor: Anchor,
        program: ProgramBinding,
        command: &[u8],
    ) -> Result<(), ExecutorCheckpointError> {
        if anchor.agent_id != registered.anchor().agent_id
            || program != registered.program_binding()
            || anchor.revision > registered.anchor().revision
            || !admissions
                .records_for(registered)
                .map_err(|e| ExecutorCheckpointError::Simulator(SimulatorError::Admission(e)))?
                .any(|r| {
                    r.status == AdmissionStatus::Consumed
                        && r.accepted_root == Some(anchor.root)
                        && r.binding.expected_revision.checked_add(1) == Some(anchor.revision)
                })
        {
            return Err(ExecutorCheckpointError::Binding);
        }
        if anchor.revision == registered.anchor().revision {
            if anchor != registered.anchor() {
                return Err(ExecutorCheckpointError::Binding);
            }
            let episode = registered
                .snapshot()
                .current_episode()
                .ok_or(ExecutorCheckpointError::Binding)?;
            let last = registered
                .snapshot()
                .boundary(
                    episode
                        .boundaries()
                        .last()
                        .ok_or(ExecutorCheckpointError::Binding)?,
                )
                .ok_or(ExecutorCheckpointError::Binding)?;
            let Exit::AwaitAction(bytes) = last.exit() else {
                return Err(ExecutorCheckpointError::Binding);
            };
            if bytes != command {
                return Err(ExecutorCheckpointError::Binding);
            }
        }
        Ok(())
    }
    fn from_frame(
        encoded: Vec<u8>,
        limits: &ExecutorLimits,
    ) -> Result<Self, ExecutorCheckpointError> {
        let (agent, program, rows) = wire::inspect(&encoded, limits)?;
        Ok(Self {
            encoded,
            agent,
            program,
            rows,
        })
    }
    pub fn decode(bytes: &[u8], limits: &ExecutorLimits) -> Result<Self, ExecutorCheckpointError> {
        let (agent, program, rows) = wire::inspect(bytes, limits)?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(bytes.len())
            .map_err(|_| ExecutorCheckpointError::Allocation)?;
        encoded.extend_from_slice(bytes);
        Ok(Self {
            encoded,
            agent,
            program,
            rows,
        })
    }
    #[cfg(test)]
    pub(super) fn test_frame(
        agent: Id32,
        program: ProgramBinding,
        rows: &[&Entry],
        limits: &ExecutorLimits,
        submitted: Option<Id32>,
    ) -> Self {
        Self::from_frame(
            wire::encode_stage(agent, program, rows, limits, submitted).unwrap(),
            limits,
        )
        .unwrap()
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }
    pub fn agent_id(&self) -> Id32 {
        self.agent
    }
    pub fn program_binding(&self) -> ProgramBinding {
        self.program
    }
    pub fn records(&self) -> impl Iterator<Item = ExecutionRecord> + '_ {
        self.rows.iter().map(|r| r.record)
    }
    pub fn command_bytes(&self, action: Id32) -> Option<&[u8]> {
        self.row(action).map(|r| &self.encoded[r.command.clone()])
    }
    pub fn receipt_bytes(&self, action: Id32) -> Option<&[u8]> {
        self.row(action)
            .and_then(|r| r.receipt.as_ref())
            .map(|r| &self.encoded[r.clone()])
    }
    pub fn recorded_limits(&self, action: Id32) -> Option<Limits> {
        self.row(action).map(|r| r.limits)
    }
    fn row(&self, action: Id32) -> Option<&Stored> {
        self.rows
            .binary_search_by_key(&action, |r| r.record.action_id)
            .ok()
            .map(|i| &self.rows[i])
    }
}
