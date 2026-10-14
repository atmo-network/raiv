//! Usage-report data, not a complete lifetime expense ledger or budget consumption.

use crate::format::{validate_drisl, CodecError, DecodeLimits, RecordEncoder, ValueView};
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageError {
    Codec(CodecError),
    InvalidSchema,
}

impl From<CodecError> for UsageError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::InvalidSchema => f.write_str(
                "Usage requires two unsigned counters and three nullable unsigned cost fields",
            ),
        }
    }
}

impl core::error::Error for UsageError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// Full-width runtime counters. Unknown token counts or billed units are None/null,
/// not zero. All five keys remain required even when their nullable values are unknown.
/// A checkpoint report does not account for all failed paid attempts or lost activations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Usage {
    pub model_calls: u64,
    pub local_tool_calls: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub billed_units: Option<u64>,
}

impl Usage {
    /// Encode complete usage data without estimating missing cost or spending budget.
    ///
    /// ```
    /// use raiv::usage::Usage;
    /// use raiv::format::DecodeLimits;
    /// let usage = Usage { model_calls: 1, local_tool_calls: 0, input_tokens: None,
    ///     output_tokens: Some(u64::MAX), billed_units: Some(0) };
    /// let bytes = usage.encode(&DecodeLimits::default())?;
    /// assert_eq!(Usage::decode(&bytes, &DecodeLimits::default())?, usage);
    /// assert_eq!(usage.input_tokens, None);
    /// assert_eq!(usage.billed_units, Some(0));
    /// # Ok::<(), raiv::usage::UsageError>(())
    /// ```
    pub fn encode(&self, bounds: &DecodeLimits) -> Result<Vec<u8>, UsageError> {
        if bounds.max_depth < 2 {
            return Err(CodecError::DepthLimit.into());
        }
        let mut encoder = RecordEncoder::new(bounds)?;
        self.write(&mut encoder)?;
        let bytes = encoder.finish();
        validate_drisl(&bytes, bounds)?;
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, UsageError> {
        validate_drisl(bytes, bounds)?;
        Self::from_view(ValueView::preflighted(bytes))
    }

    /// The enclosing typed owner must preflight the entire object before inspection.
    pub(crate) fn from_view(view: ValueView<'_>) -> Result<Self, UsageError> {
        let fields = view
            .fields([
                "model_calls",
                "local_tool_calls",
                "input_tokens",
                "output_tokens",
                "billed_units",
            ])
            .map_err(|_| UsageError::InvalidSchema)?;
        let nullable = |value: ValueView<'_>| {
            if value.is_null() {
                Ok(None)
            } else {
                value
                    .unsigned()
                    .map(Some)
                    .map_err(|_| UsageError::InvalidSchema)
            }
        };
        Ok(Self {
            model_calls: fields[0]
                .unsigned()
                .map_err(|_| UsageError::InvalidSchema)?,
            local_tool_calls: fields[1]
                .unsigned()
                .map_err(|_| UsageError::InvalidSchema)?,
            input_tokens: nullable(fields[2])?,
            output_tokens: nullable(fields[3])?,
            billed_units: nullable(fields[4])?,
        })
    }

    pub(crate) fn write(&self, encoder: &mut RecordEncoder) -> Result<(), CodecError> {
        encoder.map(5)?;
        for (name, value) in [
            ("model_calls", Some(self.model_calls)),
            ("billed_units", self.billed_units),
            ("input_tokens", self.input_tokens),
            ("output_tokens", self.output_tokens),
            ("local_tool_calls", Some(self.local_tool_calls)),
        ] {
            encoder.text(name)?;
            match value {
                Some(value) => encoder.unsigned(value)?,
                None => encoder.null()?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
