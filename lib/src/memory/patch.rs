//! Atomic working-Memory object merge; no registration, admission or persistence.

use super::{schema, Memory, PatchArguments};
use crate::{
    format::{merge_map_field, CodecError, DecodeLimits, ModelValue, ValueView},
    limits::Limits,
};
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchError {
    Codec(CodecError),
    EntryLimit,
    InvalidArguments,
    PlaneNotMap,
    NullInArray,
}

impl From<CodecError> for PatchError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Codec(error) => return error.fmt(f),
            Self::EntryLimit => "patch exceeds aggregate entry limit",
            Self::InvalidArguments => "patch arguments must be the object patch itself",
            Self::PlaneNotMap => "plane patch must be an object or null",
            Self::NullInArray => "replacement array data contains null",
        })
    }
}

impl PatchError {
    /// Bounded model-facing correction: what to change, never which data to guess.
    /// Transient trajectory text only; it is not persisted or part of any hash domain.
    pub fn feedback(&self) -> &'static str {
        match self {
            Self::PlaneNotMap => "invalid_patch: each supported plane (intents, strategy, tactics) must be a JSON object of named entries (not a string containing JSON), or null to delete it; e.g. {\"intents\":{\"goal\":\"ship the release\"}}",
            Self::NullInArray => "invalid_patch: replacement arrays must not contain null at any depth; delete object fields with null instead",
            Self::InvalidArguments => "invalid_patch: arguments must be the patch object itself, e.g. {\"tactics\":{\"next\":\"check the receipt\"}}",
            Self::EntryLimit => "invalid_patch: too many entries in one patch; send a smaller patch",
            Self::Codec(_) => "invalid_patch: patch is malformed or exceeds model-data or Memory bounds; send a smaller well-formed object",
        }
    }
}

impl core::error::Error for PatchError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl Memory {
    /// Apply canonical model-CBOR representing the JSON object patch itself.
    /// Null deletes object fields; arrays replace; empty object fields normalize.
    /// Failure preserves bytes/CID/assignment. This does not debit work, register
    /// a capability, authorize a disabled profile or create a checkpoint.
    ///
    /// ```
    /// use raiv::{format::DecodeLimits, limits::Limits, memory::Memory};
    /// let mut memory = Memory::encode(&[], &DecodeLimits::default())?;
    /// let complete = memory.apply_patch(&[0xa0], &Limits::default())?;
    /// assert_eq!(complete, &[0xa0]); // Enabled empty memory, not Finish.
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn apply_patch(&mut self, bytes: &[u8], limits: &Limits) -> Result<&[u8], PatchError> {
        PatchArguments::decode(bytes, limits)?.apply(self)
    }

    pub(super) fn patched(
        &self,
        patch: ValueView<'_>,
        limits: &Limits,
    ) -> Result<Self, PatchError> {
        let bounds = DecodeLimits {
            max_block_bytes: limits.max_block_bytes.min(limits.max_memory_bytes),
            max_depth: limits.max_depth,
            max_collection_entries: limits.max_collection_entries,
        };
        crate::format::validate_model_value(self.as_bytes(), &bounds)?;
        let bytes = merge_map_field(
            ValueView::preflighted(self.as_bytes()),
            "value",
            patch,
            &bounds,
            super::plane,
        )?;
        let block = ModelValue::from_encoded(bytes, &bounds)?;
        let value_span = schema(block.as_bytes()).map_err(|error| match error {
            super::MemoryError::Codec(error) => error,
            _ => CodecError::Malformed,
        })?;
        Ok(Self { block, value_span })
    }
}

#[cfg(test)]
mod tests;
