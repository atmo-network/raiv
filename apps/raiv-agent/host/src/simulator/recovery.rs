//! Explicit paired-pin recovery of LOCAL executor bookkeeping, not world/effect recovery,
//! global currentness, principal authentication or replay of a lost submission.
use super::{
    checkpoint::{
        file::{ExecutorCheckpointFile, ExecutorFileError},
        ExecutorCheckpoint, ExecutorCheckpointError,
    },
    local::LocalExecutorLimits,
    *,
};
use crate::{policy::current_pin as source_pin, recovery::LocalRecoveryPin};
use raiv::format::{DaslCid, DaslCodec};
#[cfg(test)]
mod tests;
/// Independently selected native administrative metadata. Import/provider/guest data
/// cannot select this pin. Hash equality itself is not external approval/currentness.
#[derive(Clone, Copy, Debug)]
pub struct ExecutorRecoveryPin {
    pub source: LocalRecoveryPin,
    pub checkpoint_digest: Id32,
}
#[derive(Debug)]
pub enum ExecutorRecoveryError {
    NotEmpty,
    MissingCheckpoint,
    PinMismatch,
    Simulator(SimulatorError),
    Checkpoint(ExecutorCheckpointError),
    File(ExecutorFileError),
    Policy(PolicyError),
}
impl fmt::Display for ExecutorRecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotEmpty => "executor adoption requires an empty native book; no merge/overwrite",
            Self::MissingCheckpoint => "executor checkpoint missing; no empty-ledger recovery",
            Self::PinMismatch => {
                "executor/source metadata differs from independently selected administrative pins"
            }
            Self::Simulator(e) => return e.fmt(f),
            Self::Checkpoint(e) => return e.fmt(f),
            Self::File(e) => return e.fmt(f),
            Self::Policy(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for ExecutorRecoveryError {}
fn file(
    disk: &mut DiskBlockStore,
    limits: &LocalExecutorLimits,
) -> Result<ExecutorCheckpoint, ExecutorRecoveryError> {
    ExecutorCheckpointFile::new(disk, limits.executor)
        .read()
        .map_err(ExecutorRecoveryError::File)?
        .ok_or(ExecutorRecoveryError::MissingCheckpoint)
}
fn digest(data: &ExecutorCheckpoint) -> Id32 {
    DaslCid::for_block(DaslCodec::Raw, data.as_bytes()).digest()
}
fn restore_error(error: ExecutorCheckpointError) -> ExecutorRecoveryError {
    match error {
        e @ ExecutorCheckpointError::Simulator(SimulatorError::Admission(_)) => {
            ExecutorRecoveryError::Checkpoint(e)
        }
        ExecutorCheckpointError::Simulator(e) => ExecutorRecoveryError::Simulator(e),
        e => ExecutorRecoveryError::Checkpoint(e),
    }
}
fn preflight(
    data: &ExecutorCheckpoint,
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    limits: &LocalExecutorLimits,
) -> Result<(), ExecutorRecoveryError> {
    data.preflight_restore(
        registered,
        admissions,
        &limits.caller,
        &limits.executor,
        usize::MAX,
    )
    .map_err(restore_error)
}
/// Approved RAM book + exact typed file/current source agreement, not imported selection.
/// Both files/required closure are re-acknowledged on the held lock; they are NOT one
/// world transaction. Export neither approves the pin nor proves pending-input retention.
pub fn current_pin(
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    book: &ExecutorBook,
    disk: &mut DiskBlockStore,
    limits: &LocalExecutorLimits,
) -> Result<ExecutorRecoveryPin, ExecutorRecoveryError> {
    let captured = ExecutorCheckpoint::capture(registered, admissions, book, &limits.executor)
        .map_err(ExecutorRecoveryError::Checkpoint)?;
    let stored = file(disk, limits)?;
    if captured.as_bytes() != stored.as_bytes() {
        return Err(ExecutorRecoveryError::PinMismatch);
    }
    preflight(&stored, registered, admissions, limits)?;
    let source = source_pin(
        registered,
        admissions,
        disk,
        &limits.manifest,
        &limits.caller,
    )
    .map_err(ExecutorRecoveryError::Policy)?;
    ExecutorCheckpointFile::new(disk, limits.executor)
        .replace(Some(&stored), &stored)
        .map_err(ExecutorRecoveryError::File)?;
    Ok(ExecutorRecoveryPin {
        source,
        checkpoint_digest: digest(&stored),
    })
}
/// Adopt metadata into an EMPTY independently created native book. Retain its NEW origin;
/// issue no ActionTicket and call no backend. Later authorize_pending explicitly obtains
/// new process-local correlation; Ready remains zero, Submitted/Terminal never reset.
/// All source/pin/capacity/borrowed-row checks precede payload copies; all staging and
/// exact file/closure re-acknowledgement precede one nonfallible entries assignment.
pub fn adopt_book(
    registered: &RegisteredAgent,
    admissions: &AdmissionBook,
    book: &mut ExecutorBook,
    disk: &mut DiskBlockStore,
    pin: &ExecutorRecoveryPin,
    limits: &LocalExecutorLimits,
) -> Result<(), ExecutorRecoveryError> {
    if !book.is_empty() {
        return Err(ExecutorRecoveryError::NotEmpty);
    }
    if !pin.source.matches_registered(registered) {
        return Err(ExecutorRecoveryError::PinMismatch);
    }
    let data = file(disk, limits)?;
    if digest(&data) != pin.checkpoint_digest {
        return Err(ExecutorRecoveryError::PinMismatch);
    }
    let count = data.records().count();
    if count > book.capacity {
        return Err(ExecutorRecoveryError::Simulator(SimulatorError::Capacity));
    }
    preflight(&data, registered, admissions, limits)?;
    let source = source_pin(
        registered,
        admissions,
        disk,
        &limits.manifest,
        &limits.caller,
    )
    .map_err(ExecutorRecoveryError::Policy)?;
    if source.manifest_digest != pin.source.manifest_digest {
        return Err(ExecutorRecoveryError::PinMismatch);
    }
    let staged = data
        .stage_restore(
            registered,
            admissions,
            &limits.caller,
            &limits.executor,
            book.capacity,
        )
        .map_err(restore_error)?;
    ExecutorCheckpointFile::new(disk, limits.executor)
        .replace(Some(&data), &data)
        .map_err(ExecutorRecoveryError::File)?;
    book.entries = staged;
    Ok(())
}
