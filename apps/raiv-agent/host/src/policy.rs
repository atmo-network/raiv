//! Local approved-source baseline/current-pin ports. Exported pins remain metadata.
use crate::{
    admission::AdmissionBook,
    checkpoint::{CheckpointError, CheckpointManifest, ManifestLimits},
    disk::{manifest::ManifestError, DiskBlockStore, DiskError},
    recovery::LocalRecoveryPin,
    registry::RegisteredAgent,
};
use raiv::{
    format::{DaslCid, DaslCodec},
    limits::Limits,
    state::Continuation,
};
use std::fmt;
#[cfg(test)]
mod tests;
#[derive(Debug)]
pub enum PolicyError {
    NotGenesis,
    AdmissionsPresent,
    NonEmptyUninitialized,
    MissingManifest,
    Conflict,
    Checkpoint(CheckpointError),
    Disk(DiskError),
    Manifest(ManifestError),
    Graph(raiv::graph::GraphError),
}
impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotGenesis => "baseline initialization requires approved revision-zero genesis",
            Self::AdmissionsPresent => "baseline initialization cannot erase admissions",
            Self::NonEmptyUninitialized => {
                "manifest absent in nonempty directory; explicit recovery required"
            }
            Self::MissingManifest => "approved current manifest is absent",
            Self::Conflict => "stored manifest differs from approved source/book",
            Self::Checkpoint(e) => return e.fmt(f),
            Self::Disk(e) => return e.fmt(f),
            Self::Manifest(e) => return e.fmt(f),
            Self::Graph(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for PolicyError {}
fn captured(
    registered: &RegisteredAgent,
    book: &AdmissionBook,
    limits: &ManifestLimits,
) -> Result<(CheckpointManifest, Vec<u8>, LocalRecoveryPin), PolicyError> {
    let data =
        CheckpointManifest::capture(registered, book, limits).map_err(PolicyError::Checkpoint)?;
    let bytes = data.encode(limits).map_err(PolicyError::Checkpoint)?;
    let pin = LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: data.anchor(),
        program: data.program_binding(),
        resources: data.resources(),
    };
    Ok((data, bytes, pin))
}
/// Native approved genesis -> one empty locked directory. Existing exact metadata can
/// be acknowledged; missing metadata amid ANY existing bytes/orphans is not genesis.
/// Partial pre-publication staging is fail-closed after process loss, never auto-cleaned.
pub fn initialize_local(
    registered: &RegisteredAgent,
    book: &AdmissionBook,
    disk: &mut DiskBlockStore,
    limits: &ManifestLimits,
    caller: &Limits,
) -> Result<LocalRecoveryPin, PolicyError> {
    let state = registered.snapshot().state();
    if state.revision() != 0
        || state.continuation() != Continuation::AwaitActor
        || state.current_episode_link().is_some()
        || state.history_count() != 0
    {
        return Err(PolicyError::NotGenesis);
    }
    let (data, bytes, pin) = captured(registered, book, limits)?;
    if !data.admissions().is_empty() {
        return Err(PolicyError::AdmissionsPresent);
    }
    let stored = disk
        .manifest(limits.wire)
        .read()
        .map_err(PolicyError::Disk)?;
    match stored.as_deref() {
        Some(current) if current == bytes => {}
        Some(_) => return Err(PolicyError::Conflict),
        None => {
            if !disk.directory_is_empty().map_err(PolicyError::Disk)? {
                return Err(PolicyError::NonEmptyUninitialized);
            }
        }
    }
    struct Source<'a>(&'a raiv::graph::RestoredAgent);
    impl raiv::graph::MaterializedBlocks for Source<'_> {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.0
                .blocks()
                .find(|(key, _)| *key == cid)
                .map(|(_, body)| body)
        }
    }
    raiv::graph::RestoredAgent::restore(
        registered.snapshot().root(),
        &Source(registered.snapshot()),
        caller,
    )
    .map_err(PolicyError::Graph)?;
    disk.place(registered.snapshot())
        .map_err(PolicyError::Disk)?;
    disk.readback(registered.snapshot(), caller)
        .map_err(PolicyError::Disk)?;
    disk.manifest(limits.wire)
        .replace(None, &bytes)
        .map_err(PolicyError::Manifest)?;
    Ok(pin)
}
/// Export metadata from the ALREADY APPROVED RAM head/book, not a decoded/imported head.
/// Exact file equality and whole required-closure readback/fsync precede return. This
/// neither stores a second authority file nor proves authenticity/global currentness:
/// the administrator owns safe retention/selection of the pin for later adoption.
pub fn current_pin(
    registered: &RegisteredAgent,
    book: &AdmissionBook,
    disk: &mut DiskBlockStore,
    limits: &ManifestLimits,
    caller: &Limits,
) -> Result<LocalRecoveryPin, PolicyError> {
    let (_, bytes, pin) = captured(registered, book, limits)?;
    let stored = disk
        .manifest(limits.wire)
        .read()
        .map_err(PolicyError::Disk)?
        .ok_or(PolicyError::MissingManifest)?;
    if stored != bytes {
        return Err(PolicyError::Conflict);
    }
    disk.readback(registered.snapshot(), caller)
        .map_err(PolicyError::Disk)?;
    disk.place(registered.snapshot())
        .map_err(PolicyError::Disk)?;
    disk.manifest(limits.wire)
        .replace(Some(&bytes), &bytes)
        .map_err(PolicyError::Manifest)?;
    Ok(pin)
}
