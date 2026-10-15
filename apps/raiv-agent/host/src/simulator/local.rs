//! File-backed LOCAL intent exposure composition. No backend, world mutation, spending,
//! terminal/world transaction, executor adoption or live effect permission is provided.

use super::checkpoint::{
    file::{ExecutorCheckpointFile, ExecutorFileError},
    ExecutorCheckpoint, ExecutorCheckpointError, ExecutorLimits,
};
use super::*;
use crate::disk::manifest::{ManifestError, Publication};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct LocalExecutorLimits {
    pub executor: ExecutorLimits,
    pub manifest: ManifestLimits,
    pub caller: Limits,
}

#[derive(Debug)]
pub enum LocalExecutorError {
    Simulator(SimulatorError),
    Checkpoint(ExecutorCheckpointError),
    File(ExecutorFileError),
    MissingCheckpoint,
    NotGenesis,
    NonEmptyBaseline,
    Conflict,
}

impl fmt::Display for LocalExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Simulator(e) => return e.fmt(f),
            Self::Checkpoint(e) => return e.fmt(f),
            Self::File(e) => return e.fmt(f),
            Self::MissingCheckpoint => {
                "executor checkpoint missing; explicit initialization/recovery required"
            }
            Self::NotGenesis => {
                "executor bootstrap requires approved revision-zero AwaitActor with empty books"
            }
            Self::NonEmptyBaseline => {
                "executor checkpoint absent amid non-baseline files; explicit recovery required"
            }
            Self::Conflict => {
                "executor checkpoint disagrees with recorded local state; no replay or overwrite"
            }
        })
    }
}

impl std::error::Error for LocalExecutorError {}

/// Explicit bootstrap of EMPTY executor DATA beside an already initialized main genesis.
/// Independently selected source pin + no admissions/actions + exact source-only namespace.
/// No late/missing-ledger fallback, orphan cleanup, world creation or capability issuance.
pub fn initialize_genesis(
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    book: &ExecutorBook,
    disk: &mut DiskBlockStore,
    approved_source: &crate::recovery::LocalRecoveryPin,
    limits: &LocalExecutorLimits,
) -> Result<Publication, LocalExecutorError> {
    let state = registered.snapshot().state();
    if state.revision() != 0
        || state.continuation() != Continuation::AwaitActor
        || state.current_episode_link().is_some()
        || state.history_count() != 0
        || !book.is_empty()
        || admissions
            .records_for(registered)
            .map_err(|e| LocalExecutorError::Simulator(SimulatorError::Admission(e)))?
            .next()
            .is_some()
    {
        return Err(LocalExecutorError::NotGenesis);
    }
    if !approved_source.matches_registered(registered) {
        return Err(LocalExecutorError::Conflict);
    }
    let empty = ExecutorCheckpoint::capture(registered, admissions, book, &limits.executor)
        .map_err(LocalExecutorError::Checkpoint)?;
    let stored = ExecutorCheckpointFile::new(disk, limits.executor)
        .read()
        .map_err(LocalExecutorError::File)?;
    match &stored {
        Some(data) if data.as_bytes() == empty.as_bytes() => {}
        Some(_) => return Err(LocalExecutorError::Conflict),
        None => {
            if !disk
                .directory_is_source_baseline(registered.snapshot())
                .map_err(|e| LocalExecutorError::File(ExecutorFileError::Disk(e)))?
            {
                return Err(LocalExecutorError::NonEmptyBaseline);
            }
        }
    }
    let source = current_pin(
        registered,
        admissions,
        disk,
        &limits.manifest,
        &limits.caller,
    )
    .map_err(|e| LocalExecutorError::Simulator(SimulatorError::Policy(e)))?;
    if source.manifest_digest != approved_source.manifest_digest {
        return Err(LocalExecutorError::Conflict);
    }
    ExecutorCheckpointFile::new(disk, limits.executor)
        .replace(stored.as_ref(), &empty)
        .map_err(LocalExecutorError::File)
}

fn file_current(
    disk: &mut DiskBlockStore,
    limits: &LocalExecutorLimits,
) -> Result<ExecutorCheckpoint, LocalExecutorError> {
    ExecutorCheckpointFile::new(disk, limits.executor)
        .read()
        .map_err(LocalExecutorError::File)?
        .ok_or(LocalExecutorError::MissingCheckpoint)
}

fn exact_row(a: &ExecutorCheckpoint, b: &ExecutorCheckpoint, record: ExecutionRecord) -> bool {
    let id = record.action_id;
    b.records().any(|r| r == record)
        && a.command_bytes(id) == b.command_bytes(id)
        && a.receipt_bytes(id) == b.receipt_bytes(id)
        && a.recorded_limits(id) == b.recorded_limits(id)
}

fn ready_extension(old: &ExecutorCheckpoint, next: &ExecutorCheckpoint, action: Id32) -> bool {
    old.agent_id() == next.agent_id()
        && old.program_binding() == next.program_binding()
        && old.command_bytes(action).is_none()
        && old.records().count().checked_add(1) == Some(next.records().count())
        && old
            .records()
            .all(|r| r.status == Status::Terminal && exact_row(old, next, r))
        && next.records().any(|r| {
            r.action_id == action && r.status == Status::Ready && r.submission_intents == 0
        })
}

/// Require an explicitly provisioned prior typed file: no absent-ledger fallback.
/// Add only this current Ready slot to exact retained terminal history, or acknowledge
/// the identical whole frame. Existing Submitted/Terminal cannot be reset to Ready.
pub fn record_ready(
    ticket: ActionTicket,
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    book: &ExecutorBook,
    disk: &mut DiskBlockStore,
    limits: &LocalExecutorLimits,
) -> Result<Publication, LocalExecutorError> {
    let index = book
        .ready_current(ticket, registered, admissions, &limits.caller)
        .map_err(LocalExecutorError::Simulator)?;
    let action = book.entries[index].command.action_id();
    let next = ExecutorCheckpoint::capture(registered, admissions, book, &limits.executor)
        .map_err(LocalExecutorError::Checkpoint)?;
    let old = file_current(disk, limits)?;
    if old.as_bytes() != next.as_bytes() && !ready_extension(&old, &next, action) {
        return Err(LocalExecutorError::Conflict);
    }
    current_pin(
        registered,
        admissions,
        disk,
        &limits.manifest,
        &limits.caller,
    )
    .map_err(|e| LocalExecutorError::Simulator(SimulatorError::Policy(e)))?;
    ExecutorCheckpointFile::new(disk, limits.executor)
        .replace(Some(&old), &next)
        .map_err(LocalExecutorError::File)
}

/// Require exact Ready file/native-book equality and renewed current source readiness.
/// Stage everything fallible before replacing Ready→Submitted/count1; fsync before
/// Command exposure. Unknown publication consumes the RAM intent WITHOUT exposure.
/// Identical pre-existing Submitted bytes cannot bootstrap a fresh RAM attempt.
pub fn begin_local<'a>(
    ticket: ActionTicket,
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    book: &'a mut ExecutorBook,
    disk: &mut DiskBlockStore,
    limits: &LocalExecutorLimits,
) -> Result<&'a Command, LocalExecutorError> {
    let index = book
        .ready_current(ticket, registered, admissions, &limits.caller)
        .map_err(LocalExecutorError::Simulator)?;
    let old = ExecutorCheckpoint::capture(registered, admissions, book, &limits.executor)
        .map_err(LocalExecutorError::Checkpoint)?;
    let next = ExecutorCheckpoint::capture_stage(
        registered,
        admissions,
        book,
        &limits.executor,
        Some(book.entries[index].command.action_id()),
    )
    .map_err(LocalExecutorError::Checkpoint)?;
    let stored = file_current(disk, limits)?;
    if stored.as_bytes() != old.as_bytes() {
        return Err(LocalExecutorError::Conflict);
    }
    current_pin(
        registered,
        admissions,
        disk,
        &limits.manifest,
        &limits.caller,
    )
    .map_err(|e| LocalExecutorError::Simulator(SimulatorError::Policy(e)))?;
    let published = ExecutorCheckpointFile::new(disk, limits.executor).replace(Some(&old), &next);
    match published {
        Ok(Publication::Replaced) => Ok(book.mark_submitted(index)),
        Ok(Publication::AlreadyPresent) => {
            book.mark_submitted(index);
            Err(LocalExecutorError::Conflict)
        }
        Err(error @ ExecutorFileError::Manifest(ManifestError::UnknownOutcome(_))) => {
            book.mark_submitted(index);
            Err(LocalExecutorError::File(error))
        }
        Err(error) => Err(LocalExecutorError::File(error)),
    }
}

/// Explicit administrative terminal DATA approval; matching is NOT effect/provenance proof.
/// Persist/fsync exact terminal bytes before RAM installation. Unknown retains Submitted;
/// identical next is verified/acknowledged without re-execution; conflicts never replace.
/// Known historical duplicates require the current whole-book file/current source agreement,
/// not rollback to an earlier checkpoint. No world mutation or expense is fabricated here.
pub fn resolve_local_data(
    ticket: ActionTicket,
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    book: &mut ExecutorBook,
    receipt: Receipt,
    disk: &mut DiskBlockStore,
    limits: &LocalExecutorLimits,
) -> Result<ExecutionRecord, LocalExecutorError> {
    let (index, known) = book
        .terminal_candidate(ticket, &receipt)
        .map_err(LocalExecutorError::Simulator)?;
    if book.entries[index].limits != ExecutorBook::execution_limits(registered, &limits.caller) {
        return Err(LocalExecutorError::Simulator(SimulatorError::Binding));
    }
    let old = ExecutorCheckpoint::capture(registered, admissions, book, &limits.executor)
        .map_err(LocalExecutorError::Checkpoint)?;
    let staged = if known {
        None
    } else {
        Some(
            ExecutorCheckpoint::capture_terminal(
                registered,
                admissions,
                book,
                &limits.executor,
                book.entries[index].command.action_id(),
                &receipt,
            )
            .map_err(LocalExecutorError::Checkpoint)?,
        )
    };
    let next = staged.as_ref().unwrap_or(&old);
    let stored = file_current(disk, limits)?;
    if stored.as_bytes() != old.as_bytes() && stored.as_bytes() != next.as_bytes() {
        return Err(LocalExecutorError::Conflict);
    }
    current_pin(
        registered,
        admissions,
        disk,
        &limits.manifest,
        &limits.caller,
    )
    .map_err(|e| LocalExecutorError::Simulator(SimulatorError::Policy(e)))?;
    ExecutorCheckpointFile::new(disk, limits.executor)
        .replace(Some(&old), next)
        .map_err(LocalExecutorError::File)?;
    if !known {
        book.entries[index].receipt = Some(receipt);
    }
    Ok(book.entries[index].record())
}
