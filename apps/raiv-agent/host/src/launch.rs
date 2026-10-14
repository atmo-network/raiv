//! Durable local reservation/launch exposure gate; no pricing/lost-unit/dispatch policy.
use crate::{
    admission::{AdmissionBook, AdmissionError, AdmissionStatus, AdmittedInput},
    checkpoint::{CheckpointError, CheckpointManifest, ManifestLimits},
    disk::{manifest::ManifestError, DiskBlockStore, DiskError},
    registry::RegisteredAgent,
    run::{Run, RunError},
};
use raiv::limits::Limits;
use std::fmt;
#[cfg(test)]
mod tests;
#[derive(Debug)]
pub enum LaunchError {
    Admission(AdmissionError),
    Checkpoint(CheckpointError),
    Disk(DiskError),
    Manifest(ManifestError),
    Run(RunError),
    MissingManifest,
    Conflict,
}
impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Admission(e) => return e.fmt(f),
            Self::Checkpoint(e) => return e.fmt(f),
            Self::Disk(e) => return e.fmt(f),
            Self::Manifest(e) => return e.fmt(f),
            Self::Run(e) => return e.fmt(f),
            Self::MissingManifest => "approved source manifest is absent",
            Self::Conflict => "local launch metadata CAS conflict",
        })
    }
}
impl std::error::Error for LaunchError {}
/// Persist a RAM-approved Reserved token's exact Input block and metadata. The prior
/// manifest must be the same approved checkpoint/book minus only this reservation.
/// Failure never frees the RAM reservation/token; it remains available for locked retry.
pub fn record_reserved(
    registered: &RegisteredAgent,
    book: &AdmissionBook,
    token: &AdmittedInput,
    disk: &mut DiskBlockStore,
    limits: &ManifestLimits,
    caller: &Limits,
) -> Result<(), LaunchError> {
    book.reserved_entry(registered, token)
        .map_err(LaunchError::Admission)?;
    raiv::input::ActivationInput::decode(token.packet().input().as_bytes(), caller)
        .map_err(|e| LaunchError::Admission(AdmissionError::Input(e)))?;
    let next =
        CheckpointManifest::capture(registered, book, limits).map_err(LaunchError::Checkpoint)?;
    let baseline = next
        .admission_stage(token.packet().binding(), None, limits)
        .map_err(LaunchError::Checkpoint)?;
    let old = baseline.encode(limits).map_err(LaunchError::Checkpoint)?;
    let new = next.encode(limits).map_err(LaunchError::Checkpoint)?;
    let current = disk
        .manifest(limits.wire)
        .read()
        .map_err(LaunchError::Disk)?
        .ok_or(LaunchError::MissingManifest)?;
    if current != old && current != new {
        return Err(LaunchError::Conflict);
    }
    disk.readback(registered.snapshot(), caller)
        .map_err(LaunchError::Disk)?;
    let input = token.packet().input();
    disk.put(input.cid(), input.as_bytes())
        .map_err(LaunchError::Disk)?;
    let stored = disk
        .load_capped(input.cid(), &caller.decode_limits())
        .map_err(LaunchError::Disk)?;
    if stored != input.as_bytes() {
        return Err(LaunchError::Conflict);
    }
    disk.manifest(limits.wire)
        .replace(Some(&old), &new)
        .map_err(LaunchError::Manifest)?;
    Ok(())
}
/// Attempted is fsynced before any guest VM is created. Launched is fsynced before a
/// Run/provider request can leave this function. Attempt-publication uncertainty
/// conservatively marks Attempted and blocks relaunch; existing Attempted file bytes
/// never authorize a fresh RAM token/VM. Any actual attempt stays unresolved.
/// A failed Launched write discards the unexposed VM; it never reruns/refunds that token.
pub fn launch_local<'a>(
    wasm: &[u8],
    registered: &'a RegisteredAgent,
    book: &mut AdmissionBook,
    token: &'a AdmittedInput,
    disk: &mut DiskBlockStore,
    limits: &ManifestLimits,
    caller: &Limits,
) -> Result<Run<'a>, LaunchError> {
    let index = book
        .reserved_entry(registered, token)
        .map_err(LaunchError::Admission)?;
    raiv::input::ActivationInput::decode(token.packet().input().as_bytes(), caller)
        .map_err(|e| LaunchError::Admission(AdmissionError::Input(e)))?;
    registered
        .program_binding()
        .check_module(wasm, registered.snapshot())
        .map_err(LaunchError::Run)?;
    let reserved =
        CheckpointManifest::capture(registered, book, limits).map_err(LaunchError::Checkpoint)?;
    let attempted = reserved
        .admission_stage(
            token.packet().binding(),
            Some(AdmissionStatus::Attempted),
            limits,
        )
        .map_err(LaunchError::Checkpoint)?;
    let launched = attempted
        .admission_stage(
            token.packet().binding(),
            Some(AdmissionStatus::Launched),
            limits,
        )
        .map_err(LaunchError::Checkpoint)?;
    let old = reserved.encode(limits).map_err(LaunchError::Checkpoint)?;
    let attempted = attempted.encode(limits).map_err(LaunchError::Checkpoint)?;
    let launched = launched.encode(limits).map_err(LaunchError::Checkpoint)?;
    let current = disk
        .manifest(limits.wire)
        .read()
        .map_err(LaunchError::Disk)?
        .ok_or(LaunchError::MissingManifest)?;
    // Attempted bytes may belong to a lost prior process/VM, not this RAM token.
    // Never treat them as permission to launch again, even with identical binding.
    if current != old {
        return Err(LaunchError::Conflict);
    }
    disk.readback(registered.snapshot(), caller)
        .map_err(LaunchError::Disk)?;
    let input = token.packet().input();
    if disk
        .load_capped(input.cid(), &caller.decode_limits())
        .map_err(LaunchError::Disk)?
        != input.as_bytes()
    {
        return Err(LaunchError::Conflict);
    }
    if let Err(error) = disk.manifest(limits.wire).replace(Some(&old), &attempted) {
        if matches!(error, ManifestError::UnknownOutcome(_)) {
            book.mark_attempt(index);
        }
        return Err(LaunchError::Manifest(error));
    }
    book.mark_attempt(index);
    let run = registered
        .launch(wasm, token.packet())
        .map_err(LaunchError::Run)?;
    book.mark_launched(index, run.origin());
    disk.manifest(limits.wire)
        .replace(Some(&attempted), &launched)
        .map_err(LaunchError::Manifest)?;
    Ok(run)
}
