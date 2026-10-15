//! Explicit local administrative adoption, never automatic trust from decoded data.

use crate::{
    admission::{AdmissionBook, AdmissionError, AdmittedInput},
    checkpoint::{CheckpointError, CheckpointManifest, ManifestLimits},
    disk::{DiskBlockStore, DiskError},
    registry::{Anchor, RegisteredAgent, Registry, RegistryError},
    run::ProgramBinding,
    VmLimits,
};
use raiv::{
    format::{DaslCid, DaslCodec, Id32},
    limits::Limits,
};
use std::fmt;

mod reservation;

pub use reservation::resume_reserved;

#[cfg(test)]
mod tests;

/// Metadata supplied by the local administrator through the MUTABLE registry/book port.
/// Constructing these fields or hashing a file is not itself approval. Obtain pins from
/// the accepting local policy; never let imported/guest/provider payloads choose them.
#[derive(Clone, Copy, Debug)]
pub struct LocalRecoveryPin {
    pub manifest_digest: Id32,
    pub anchor: Anchor,
    pub program: ProgramBinding,
    pub resources: VmLimits,
}

impl LocalRecoveryPin {
    /// Native data binding only; matching never selects or approves a current source.
    pub(crate) fn matches_registered(&self, registered: &RegisteredAgent) -> bool {
        self.anchor == registered.anchor()
            && self.program == registered.program_binding()
            && self.resources.memory_bytes == registered.resources().memory_bytes
            && self.resources.fuel_per_step == registered.resources().fuel_per_step
    }
}

#[derive(Debug)]
pub enum RecoveryError {
    MissingManifest,
    PinMismatch,
    Disk(DiskError),
    Checkpoint(CheckpointError),
    Registry(RegistryError),
    Admission(AdmissionError),
    Manifest(crate::disk::manifest::ManifestError),
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingManifest => "local recovery manifest is absent",
            Self::PinMismatch => "local manifest differs from explicit administrative pins",
            Self::Disk(e) => return e.fmt(f),
            Self::Checkpoint(e) => return e.fmt(f),
            Self::Registry(e) => return e.fmt(f),
            Self::Admission(e) => return e.fmt(f),
            Self::Manifest(e) => return e.fmt(f),
        })
    }
}

impl std::error::Error for RecoveryError {}

/// Read the fixed locked local file, bind the WHOLE metadata digest and explicit anchor/
/// module/resource pins, restore all dependencies, validate module/exports/resources,
/// stage registry and admission capacity, then publish both without fallible work.
/// Unresolved records retain status/attempts but no token or originating Run: fail closed.
/// No effects, refund, inference, manifest rewrite or authority outside this local registry.
pub fn adopt_local(
    wasm: &[u8],
    disk: &mut DiskBlockStore,
    registry: &mut Registry,
    book: &mut AdmissionBook,
    pin: LocalRecoveryPin,
    manifest_limits: &ManifestLimits,
    caller: &Limits,
) -> Result<Anchor, RecoveryError> {
    let bytes = disk
        .manifest(manifest_limits.wire)
        .read()
        .map_err(RecoveryError::Disk)?
        .ok_or(RecoveryError::MissingManifest)?;
    if DaslCid::for_block(DaslCodec::Raw, &bytes).digest() != pin.manifest_digest {
        return Err(RecoveryError::PinMismatch);
    }
    let manifest =
        CheckpointManifest::decode(&bytes, manifest_limits).map_err(RecoveryError::Checkpoint)?;
    let resources = manifest.resources();
    if manifest.anchor() != pin.anchor
        || manifest.program_binding() != pin.program
        || resources.memory_bytes != pin.resources.memory_bytes
        || resources.fuel_per_step != pin.resources.fuel_per_step
    {
        return Err(RecoveryError::PinMismatch);
    }
    let snapshot = manifest
        .restore(disk, caller)
        .map_err(RecoveryError::Checkpoint)?;
    let entry = registry
        .stage_approved(wasm, pin.program, snapshot, pin.resources)
        .map_err(RecoveryError::Registry)?;
    let staged = book
        .stage_recovered(&entry, manifest.admissions())
        .map_err(RecoveryError::Admission)?;
    // Re-establish current file/block durability before exposing an adopted decision;
    // an unacknowledged rename may have left complete bytes visible in page cache.
    disk.place(entry.snapshot()).map_err(RecoveryError::Disk)?;
    disk.manifest(manifest_limits.wire)
        .replace(Some(&bytes), &bytes)
        .map_err(RecoveryError::Manifest)?;
    registry.publish_approved(entry);
    book.publish_recovered(staged);
    Ok(pin.anchor)
}
