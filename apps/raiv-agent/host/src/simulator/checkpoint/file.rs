//! Typed executor checkpoint FILE DATA. Byte CAS is not an FSM/world transaction,
//! authentication, current selection, native capability recreation or dispatch permission.
use super::{wire, ExecutorCheckpoint, ExecutorCheckpointError, ExecutorLimits};
use crate::disk::{
    manifest::{ManifestError, ManifestFile, Publication},
    DiskBlockStore, DiskError,
};
use std::fmt;
#[cfg(test)]
mod tests;
#[derive(Debug)]
pub enum ExecutorFileError {
    Checkpoint(ExecutorCheckpointError),
    Disk(DiskError),
    Manifest(ManifestError),
}
impl fmt::Display for ExecutorFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Checkpoint(e) => e.fmt(f),
            Self::Disk(e) => e.fmt(f),
            Self::Manifest(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for ExecutorFileError {}
/// Explicit mutable local administrative file port on the held cooperative lock.
/// It writes only raiv-executor.cbor. No caller-selected path or imported filename exists.
/// It cannot update the independent main checkpoint file or any RAM executor state.
pub struct ExecutorCheckpointFile<'a> {
    file: ManifestFile<'a>,
    limits: ExecutorLimits,
}
impl<'a> ExecutorCheckpointFile<'a> {
    pub fn new(store: &'a mut DiskBlockStore, limits: ExecutorLimits) -> Self {
        Self {
            file: ManifestFile::executor(store, limits.wire),
            limits,
        }
    }
    /// Only ENOENT means absence. Existing corrupt, foreign-schema or over-cap data refuses.
    pub fn read(&self) -> Result<Option<ExecutorCheckpoint>, ExecutorFileError> {
        let Some(bytes) = self.file.read().map_err(ExecutorFileError::Disk)? else {
            return Ok(None);
        };
        ExecutorCheckpoint::from_frame(bytes, &self.limits)
            .map(Some)
            .map_err(ExecutorFileError::Checkpoint)
    }
    /// Preflight caller data and existing typed file before exact-byte file CAS/fsync.
    /// Identical next is acknowledged without replacement, not fresh execution permission.
    /// This port intentionally does NOT enforce FSM transitions or couple world/HEAD changes.
    pub fn replace(
        &mut self,
        expected: Option<&ExecutorCheckpoint>,
        next: &ExecutorCheckpoint,
    ) -> Result<Publication, ExecutorFileError> {
        wire::inspect(next.as_bytes(), &self.limits).map_err(ExecutorFileError::Checkpoint)?;
        if let Some(expected) = expected {
            wire::inspect(expected.as_bytes(), &self.limits)
                .map_err(ExecutorFileError::Checkpoint)?;
        }
        let _ = self.read()?;
        self.file
            .replace(expected.map(ExecutorCheckpoint::as_bytes), next.as_bytes())
            .map_err(ExecutorFileError::Manifest)
    }
}
