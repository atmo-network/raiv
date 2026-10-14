//! Native RAM/local-file boundary transaction roots. Neither grants a dispatch permit.
use crate::{
    admission::{AdmissionBook, AdmissionError, AdmissionStatus, AdmittedInput},
    registry::{Anchor, Registry},
    run::SealedRun,
    store::{MemoryBlockStore, StoreError},
};
use raiv::limits::Limits;
use std::fmt;
#[cfg(target_os = "linux")]
mod local;
#[cfg(target_os = "linux")]
pub use local::{accept_local, LocalAcceptanceError};
#[derive(Debug)]
pub enum AcceptanceError {
    UnknownAgent,
    Program,
    StaleAnchor,
    MissingSnapshot,
    Admission(AdmissionError),
    Readback(StoreError),
}
impl fmt::Display for AcceptanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownAgent => "agent is not registered",
            Self::Program => "sealed program is not locally approved",
            Self::StaleAnchor => "checkpoint revision/root CAS conflict",
            Self::MissingSnapshot => "sealed checkpoint ownership already transferred",
            Self::Admission(e) => return e.fmt(f),
            Self::Readback(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for AcceptanceError {}
/// Only for isolated local RAM computation. Complete readback binds the required bytes;
/// it is not the durable readiness policy required before real effect dispatch.
/// Exclusive mutable registry/book access makes the following in-memory assignments
/// coherent. No fallible work or external calls occur after assignment starts.
pub fn accept_ram(
    registry: &mut Registry,
    book: &mut AdmissionBook,
    token: &AdmittedInput,
    sealed: &mut SealedRun,
    store: &MemoryBlockStore,
) -> Result<Anchor, AcceptanceError> {
    let (index, record, registry_origin) = book
        .sealed_entry(token, sealed)
        .map_err(AcceptanceError::Admission)?;
    let registered = registry
        .agent_mut(sealed.binding.agent_id)
        .ok_or(AcceptanceError::UnknownAgent)?;
    if registered.registry_origin() != registry_origin
        || registered.program_binding() != sealed.program
    {
        return Err(AcceptanceError::Program);
    }
    let candidate = sealed.candidate();
    let root = candidate.new_root();
    if record.status == AdmissionStatus::Consumed {
        if record.accepted_root != Some(root) {
            return Err(AcceptanceError::StaleAnchor);
        }
        return Ok(Anchor {
            agent_id: sealed.binding.agent_id,
            revision: sealed
                .binding
                .expected_revision
                .checked_add(1)
                .ok_or(AcceptanceError::StaleAnchor)?,
            root,
        });
    }
    let anchor = registered.anchor();
    if anchor.root != sealed.binding.expected_root
        || anchor.revision != sealed.binding.expected_revision
    {
        return Err(AcceptanceError::StaleAnchor);
    }
    let snapshot = sealed
        .snapshot
        .as_ref()
        .ok_or(AcceptanceError::MissingSnapshot)?;
    store
        .readback(snapshot, &Limits::default())
        .map_err(AcceptanceError::Readback)?;
    let snapshot = sealed
        .snapshot
        .take()
        .expect("checked owning sealed snapshot");
    registered.install_ram(snapshot);
    book.consume_ram(index, root);
    Ok(registered.anchor())
}
