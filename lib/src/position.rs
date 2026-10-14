//! Normalized host coordinates, not clocks, finality evidence, or state blocks.

use crate::format::{validate_drisl, CodecError, DecodeLimits, Id32, RecordEncoder, ValueView};
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PositionError {
    Codec(CodecError),
    InvalidSchema,
}

impl From<CodecError> for PositionError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::InvalidSchema => {
                f.write_str("Position requires exact domain/number/reference fields")
            }
        }
    }
}

impl core::error::Error for PositionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// A host-normalized coordinate. The number is full-width u64, not necessarily a block
/// number. The optional reference is a raw Id32, never a structural Link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub domain: Id32,
    pub number: u64,
    pub reference: Option<Id32>,
}

impl Position {
    pub fn encode(&self, bounds: &DecodeLimits) -> Result<Vec<u8>, PositionError> {
        if bounds.max_depth < 2 {
            return Err(CodecError::DepthLimit.into());
        }
        let mut encoder = RecordEncoder::new(bounds)?;
        self.write(&mut encoder)?;
        let bytes = encoder.finish();
        validate_drisl(&bytes, bounds)?;
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, PositionError> {
        validate_drisl(bytes, bounds)?;
        Self::from_view(ValueView::preflighted(bytes))
    }

    /// The enclosing typed owner has already preflighted the entire object.
    pub(crate) fn from_view(view: ValueView<'_>) -> Result<Self, PositionError> {
        let fields = view
            .fields(["domain", "number", "reference"])
            .map_err(|_| PositionError::InvalidSchema)?;
        let domain = fields[0]
            .bytes()
            .map_err(|_| PositionError::InvalidSchema)?
            .try_into()
            .map_err(|_| PositionError::InvalidSchema)?;
        let number = fields[1]
            .unsigned()
            .map_err(|_| PositionError::InvalidSchema)?;
        let reference = if fields[2].is_null() {
            None
        } else {
            Some(
                fields[2]
                    .bytes()
                    .map_err(|_| PositionError::InvalidSchema)?
                    .try_into()
                    .map_err(|_| PositionError::InvalidSchema)?,
            )
        };
        Ok(Self {
            domain,
            number,
            reference,
        })
    }

    pub(crate) fn write(&self, encoder: &mut RecordEncoder) -> Result<(), CodecError> {
        encoder.map(3)?;
        encoder.text("domain")?;
        encoder.bytes(&self.domain)?;
        encoder.text("number")?;
        encoder.unsigned(self.number)?;
        encoder.text("reference")?;
        match self.reference {
            Some(reference) => encoder.bytes(&reference)?,
            None => encoder.null()?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
