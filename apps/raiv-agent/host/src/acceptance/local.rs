//! Local-only file-backed boundary commit; no effect/dispatch permit.

use super::*;
use crate::{
    checkpoint::{CheckpointError, CheckpointManifest, ManifestLimits},
    disk::{manifest::ManifestError, DiskBlockStore, DiskError},
};

#[derive(Debug)]
pub enum LocalAcceptanceError {
    Binding(AcceptanceError),
    Checkpoint(CheckpointError),
    Disk(DiskError),
    Manifest(ManifestError),
    MissingManifest,
    Conflict,
}

impl fmt::Display for LocalAcceptanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Binding(e) => return e.fmt(f),
            Self::Checkpoint(e) => return e.fmt(f),
            Self::Disk(e) => return e.fmt(f),
            Self::Manifest(e) => return e.fmt(f),
            Self::MissingManifest => "admitted source manifest is absent",
            Self::Conflict => "stored manifest differs from admitted source or exact next decision",
        })
    }
}

impl std::error::Error for LocalAcceptanceError {}

/// Requires an already durably recorded exact Launched source manifest. This port
/// commits a sealed boundary, not an admission-before-inference/accounting protocol.
/// Registered source + opaque book/run joins precede any placement/publication.
/// Entire new closure fsync/readback precedes the one typed anchor/consumption swap.
/// No fallible work follows RAM installation; unknown file outcome keeps RAM/handoff.
/// Retry uses the same sealed output, verifies/resyncs exact next bytes, never inference.
pub fn accept_local(
    registry: &mut Registry,
    book: &mut AdmissionBook,
    token: &AdmittedInput,
    sealed: &mut SealedRun,
    disk: &mut DiskBlockStore,
    manifest_limits: &ManifestLimits,
    caller: &Limits,
) -> Result<Anchor, LocalAcceptanceError> {
    let binding = LocalAcceptanceError::Binding;
    let (index, record, origin) = book
        .sealed_entry(token, sealed)
        .map_err(|e| binding(AcceptanceError::Admission(e)))?;
    let registered = registry
        .agent_mut(sealed.binding.agent_id)
        .ok_or(binding(AcceptanceError::UnknownAgent))?;
    if registered.registry_origin() != origin || registered.program_binding() != sealed.program {
        return Err(binding(AcceptanceError::Program));
    }
    let root = sealed.candidate().new_root();
    let source = CheckpointManifest::capture(registered, book, manifest_limits)
        .map_err(LocalAcceptanceError::Checkpoint)?;
    let source_bytes = source
        .encode(manifest_limits)
        .map_err(LocalAcceptanceError::Checkpoint)?;
    let stored = disk
        .manifest(manifest_limits.wire)
        .read()
        .map_err(LocalAcceptanceError::Disk)?
        .ok_or(LocalAcceptanceError::MissingManifest)?;
    if record.status == AdmissionStatus::Consumed {
        if record.accepted_root != Some(root) {
            return Err(binding(AcceptanceError::StaleAnchor));
        }
        if stored != source_bytes {
            return Err(LocalAcceptanceError::Conflict);
        }
        disk.readback(registered.snapshot(), caller)
            .map_err(LocalAcceptanceError::Disk)?;
        disk.manifest(manifest_limits.wire)
            .replace(Some(&source_bytes), &source_bytes)
            .map_err(LocalAcceptanceError::Manifest)?;
        return Ok(Anchor {
            agent_id: sealed.binding.agent_id,
            revision: sealed
                .binding
                .expected_revision
                .checked_add(1)
                .ok_or(binding(AcceptanceError::StaleAnchor))?,
            root,
        });
    }
    if registered.anchor().root != sealed.binding.expected_root
        || registered.anchor().revision != sealed.binding.expected_revision
    {
        return Err(binding(AcceptanceError::StaleAnchor));
    }
    let snapshot = sealed
        .snapshot
        .as_ref()
        .ok_or(binding(AcceptanceError::MissingSnapshot))?;
    let next = source
        .next(snapshot, sealed.binding, manifest_limits)
        .map_err(LocalAcceptanceError::Checkpoint)?;
    let next_bytes = next
        .encode(manifest_limits)
        .map_err(LocalAcceptanceError::Checkpoint)?;
    if stored != source_bytes && stored != next_bytes {
        return Err(LocalAcceptanceError::Conflict);
    }
    if stored == source_bytes {
        disk.readback(registered.snapshot(), caller)
            .map_err(LocalAcceptanceError::Disk)?;
    }
    disk.place(snapshot).map_err(LocalAcceptanceError::Disk)?;
    disk.readback(snapshot, caller)
        .map_err(LocalAcceptanceError::Disk)?;
    disk.manifest(manifest_limits.wire)
        .replace(Some(&source_bytes), &next_bytes)
        .map_err(LocalAcceptanceError::Manifest)?;
    let snapshot = sealed
        .snapshot
        .take()
        .expect("preflighted owning sealed snapshot");
    registered.install_ram(snapshot);
    book.consume_ram(index, root);
    Ok(registered.anchor())
}
