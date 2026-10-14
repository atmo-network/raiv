//! Owns sparse semantic Memory blocks and atomic object patches, not host authority.

mod arguments;
mod patch;
pub use arguments::{PatchArguments, SkippedPlane};
pub use patch::PatchError;

use crate::format::{
    validate_drisl, CodecError, DaslCid, DaslCodec, DecodeLimits, ModelValue, ValueRef, ValueView,
};
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    Codec(CodecError),
    WrongKind,
    UnsupportedVersion,
    InvalidSchema,
}
impl From<CodecError> for MemoryError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}
impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::WrongKind => f.write_str("block is not a Memory record"),
            Self::UnsupportedVersion => f.write_str("unsupported Memory version"),
            Self::InvalidSchema => f.write_str(
                "Memory requires normalized null-free sparse intents/strategy/tactics objects",
            ),
        }
    }
}
impl core::error::Error for MemoryError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// Canonical Memory v1, with optional object planes intents, strategy and tactics.
/// Enabled empty memory is {}, never a disabled profile. No null or empty object
/// fields are materialized; empty objects in array positions remain. Decode rejects
/// incompatible bytes, never repairs them or translates aliases.
#[derive(Debug, Eq, PartialEq)]
pub struct Memory {
    block: ModelValue,
    value_span: Range<usize>,
}
impl Memory {
    /// Encode an already normalized sparse semantic object. Empty planes/fields
    /// are rejected, not silently removed from authored persistent state.
    pub fn encode(
        value: &[(&str, ValueRef<'_>)],
        limits: &DecodeLimits,
    ) -> Result<Self, MemoryError> {
        let record = [
            ("kind", ValueRef::Text("memory")),
            ("version", ValueRef::Integer(1)),
            ("value", ValueRef::Map(value)),
        ];
        let block = ModelValue::encode(ValueRef::Map(&record), limits)?;
        let value_span = schema(block.as_bytes())?;
        Ok(Self { block, value_span })
    }
    pub fn decode(bytes: &[u8], limits: &DecodeLimits) -> Result<Self, MemoryError> {
        validate_drisl(bytes, limits)?;
        let value_span = schema(bytes)?;
        let block = ModelValue::decode(bytes, limits)?;
        Ok(Self { block, value_span })
    }
    pub fn as_bytes(&self) -> &[u8] {
        self.block.as_bytes()
    }
    /// Complete compact sparse value, without the structural record envelope.
    pub fn value_bytes(&self) -> &[u8] {
        &self.block.as_bytes()[self.value_span.clone()]
    }
    pub fn cid(&self) -> DaslCid {
        DaslCid::for_block(DaslCodec::Drisl, self.as_bytes())
    }
}

pub(super) fn plane(key: &str) -> bool {
    matches!(key, "intents" | "strategy" | "tactics")
}

fn schema(bytes: &[u8]) -> Result<Range<usize>, MemoryError> {
    let [kind, version, value] = ValueView::preflighted(bytes)
        .fields(["kind", "version", "value"])
        .map_err(|_| MemoryError::InvalidSchema)?;
    let kind = kind.text().map_err(|_| MemoryError::InvalidSchema)?;
    let version = version.unsigned().map_err(|_| MemoryError::InvalidSchema)?;
    if kind != "memory" {
        return Err(MemoryError::WrongKind);
    }
    if version != 1 {
        return Err(MemoryError::UnsupportedVersion);
    }
    for entry in value.map().map_err(|_| MemoryError::InvalidSchema)? {
        let (key, child) = entry?;
        if !plane(key) || child.map().is_err() {
            return Err(MemoryError::InvalidSchema);
        }
    }
    // Flat borrowed traversal is bounded by preflighted input bytes, including arrays.
    let mut stack = Vec::new();
    push(&mut stack, (value, false))?;
    while let Some((value, field)) = stack.pop() {
        if value.is_null() {
            return Err(MemoryError::InvalidSchema);
        }
        match value.encoded()[0] >> 5 {
            5 => {
                let map = value.map()?;
                if field && map.len() == 0 {
                    return Err(MemoryError::InvalidSchema);
                }
                for entry in map {
                    push(&mut stack, (entry?.1, true))?;
                }
            }
            4 => {
                for child in value.list()? {
                    push(&mut stack, (child?, false))?;
                }
            }
            _ => {}
        }
    }
    Ok(value.span())
}
pub(super) fn push<T>(stack: &mut Vec<T>, value: T) -> Result<(), CodecError> {
    stack
        .try_reserve(1)
        .map_err(|_| CodecError::AllocationFailure)?;
    stack.push(value);
    Ok(())
}

#[cfg(test)]
mod tests;
