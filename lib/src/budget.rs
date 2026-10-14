//! Budget-admission data shape, not trusted-host admission or budget enforcement.

use crate::format::{validate_drisl, CodecError, DecodeLimits, Id32, RecordEncoder, ValueView};
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetError {
    Codec(CodecError),
    InvalidSchema,
}

impl From<CodecError> for BudgetError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::InvalidSchema => f.write_str(
                "BudgetAdmission requires an exact raw admission ID and four unsigned counters",
            ),
        }
    }
}

impl core::error::Error for BudgetError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// Full-width host-profile units, not a universal token-to-money conversion. Zero is
/// valid data, not permission to continue an exhausted activation. The host still binds
/// admission identity, freshness and authorization; the lifecycle enforces consumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetAdmission {
    pub admission_id: Id32,
    pub activation_units: u64,
    pub remaining_episode_units: u64,
    pub remaining_model_calls: u64,
    pub remaining_actions: u64,
}

impl BudgetAdmission {
    /// Encode bounded data without granting admission or checking coupled policy.
    ///
    /// ```
    /// use raiv::budget::BudgetAdmission;
    /// use raiv::format::DecodeLimits;
    /// let admission = BudgetAdmission { admission_id: [1; 32], activation_units: 0,
    ///     remaining_episode_units: u64::MAX, remaining_model_calls: 2, remaining_actions: 3 };
    /// let bytes = admission.encode(&DecodeLimits::default())?;
    /// assert_eq!(BudgetAdmission::decode(&bytes, &DecodeLimits::default())?, admission);
    /// # Ok::<(), raiv::budget::BudgetError>(())
    /// ```
    pub fn encode(&self, bounds: &DecodeLimits) -> Result<Vec<u8>, BudgetError> {
        if bounds.max_depth < 2 {
            return Err(CodecError::DepthLimit.into());
        }
        let mut encoder = RecordEncoder::new(bounds)?;
        self.write(&mut encoder)?;
        let bytes = encoder.finish();
        validate_drisl(&bytes, bounds)?;
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, BudgetError> {
        validate_drisl(bytes, bounds)?;
        Self::from_view(ValueView::preflighted(bytes))
    }

    /// The enclosing typed owner must preflight the entire object before inspection.
    pub(crate) fn from_view(view: ValueView<'_>) -> Result<Self, BudgetError> {
        let fields = view
            .fields([
                "admission_id",
                "activation_units",
                "remaining_episode_units",
                "remaining_model_calls",
                "remaining_actions",
            ])
            .map_err(|_| BudgetError::InvalidSchema)?;
        let admission_id = fields[0]
            .bytes()
            .map_err(|_| BudgetError::InvalidSchema)?
            .try_into()
            .map_err(|_| BudgetError::InvalidSchema)?;
        let activation_units = fields[1]
            .unsigned()
            .map_err(|_| BudgetError::InvalidSchema)?;
        let remaining_episode_units = fields[2]
            .unsigned()
            .map_err(|_| BudgetError::InvalidSchema)?;
        let remaining_model_calls = fields[3]
            .unsigned()
            .map_err(|_| BudgetError::InvalidSchema)?;
        let remaining_actions = fields[4]
            .unsigned()
            .map_err(|_| BudgetError::InvalidSchema)?;
        Ok(Self {
            admission_id,
            activation_units,
            remaining_episode_units,
            remaining_model_calls,
            remaining_actions,
        })
    }

    pub(crate) fn write(&self, encoder: &mut RecordEncoder) -> Result<(), CodecError> {
        encoder.map(5)?;
        encoder.text("admission_id")?;
        encoder.bytes(&self.admission_id)?;
        encoder.text("activation_units")?;
        encoder.unsigned(self.activation_units)?;
        encoder.text("remaining_actions")?;
        encoder.unsigned(self.remaining_actions)?;
        encoder.text("remaining_model_calls")?;
        encoder.unsigned(self.remaining_model_calls)?;
        encoder.text("remaining_episode_units")?;
        encoder.unsigned(self.remaining_episode_units)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
