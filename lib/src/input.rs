//! Activation-input block data, not prepared-input provenance, freshness or phase admission.

use crate::budget::BudgetAdmission;
use crate::format::{
    validate_drisl, validate_model_value, CodecError, DaslCid, DaslCodec, DecodeLimits, Id32,
    RecordEncoder, ValueView,
};
use crate::limits::Limits;
use crate::position::Position;
use crate::receipt::{Receipt, ReceiptError};
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputError {
    Codec(CodecError),
    Receipt(ReceiptError),
    WrongKind,
    UnsupportedVersion,
    InvalidSchema,
}

impl From<CodecError> for InputError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<ReceiptError> for InputError {
    fn from(error: ReceiptError) -> Self {
        Self::Receipt(error)
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::Receipt(error) => error.fmt(f),
            Self::WrongKind => f.write_str("block is not an ActivationInput"),
            Self::UnsupportedVersion => f.write_str("unsupported ActivationInput version"),
            Self::InvalidSchema => f.write_str(
                "ActivationInput requires exact fields and consistent cause/receipt shapes",
            ),
        }
    }
}

impl core::error::Error for InputError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Receipt(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cause {
    ActorWake,
    ActionResult,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActorWake => "actor_wake",
            Self::ActionResult => "action_result",
        }
    }

    fn parse(value: &str) -> Result<Self, InputError> {
        match value {
            "actor_wake" => Ok(Self::ActorWake),
            "action_result" => Ok(Self::ActionResult),
            _ => Err(InputError::InvalidSchema),
        }
    }

    fn check_receipt(self, present: bool) -> Result<(), InputError> {
        if (self == Self::ActionResult) == present {
            Ok(())
        } else {
            Err(InputError::InvalidSchema)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ActivationInputSpec<'a> {
    pub event_id: Id32,
    pub cause: Cause,
    pub observed_at: Position,
    pub observation: &'a [u8],
    pub receipt: Option<&'a Receipt>,
    pub budget_admission: BudgetAdmission,
}

/// One complete immutable structural block. ActorWake requires null receipt;
/// ActionResult requires terminal receipt data. Matching the current pending command,
/// verifying a trusted host source and admitting an event remain lifecycle/host work.
#[derive(Debug, Eq, PartialEq)]
pub struct ActivationInput {
    encoded: Vec<u8>,
    event_id: Id32,
    cause: Cause,
    observed_at: Position,
    observation: Range<usize>,
    receipt: Option<Range<usize>>,
    budget_admission: BudgetAdmission,
}

impl ActivationInput {
    /// Encode input data using caller policy without treating embedded budget fields as authority.
    ///
    /// ```
    /// use raiv::input::{ActivationInput, ActivationInputSpec, Cause};
    /// use raiv::budget::BudgetAdmission;
    /// use raiv::position::Position;
    /// use raiv::limits::Limits;
    /// let spec = ActivationInputSpec { event_id: [1; 32], cause: Cause::ActorWake,
    ///     observed_at: Position { domain: [2; 32], number: 0, reference: None },
    ///     observation: &[0xa0], receipt: None, budget_admission: BudgetAdmission {
    ///         admission_id: [3; 32], activation_units: 1, remaining_episode_units: 2,
    ///         remaining_model_calls: 3, remaining_actions: 4 } };
    /// let input = ActivationInput::encode(&spec, &Limits::default())?;
    /// assert_eq!(input.cause(), Cause::ActorWake);
    /// assert!(input.receipt_bytes().is_none());
    /// # Ok::<(), raiv::input::InputError>(())
    /// ```
    pub fn encode(spec: &ActivationInputSpec<'_>, limits: &Limits) -> Result<Self, InputError> {
        let bounds = limits.decode_limits();
        if bounds.max_depth < 3 {
            return Err(CodecError::DepthLimit.into());
        }
        spec.cause.check_receipt(spec.receipt.is_some())?;
        check_observation(spec.observation, &observation_bounds(limits))?;
        if let Some(receipt) = spec.receipt {
            Receipt::validate_encoded(receipt.as_bytes(), &child_limits(limits))?;
        }
        let mut encoder = RecordEncoder::new(&bounds)?;
        encoder.map(8)?;
        encoder.text("kind")?;
        encoder.text("activation_input")?;
        encoder.text("cause")?;
        encoder.text(spec.cause.as_str())?;
        encoder.text("receipt")?;
        match spec.receipt {
            Some(receipt) => encoder.encoded(receipt.as_bytes())?,
            None => encoder.null()?,
        }
        encoder.text("version")?;
        encoder.unsigned(1)?;
        encoder.text("event_id")?;
        encoder.bytes(&spec.event_id)?;
        encoder.text("observation")?;
        encoder.encoded(spec.observation)?;
        encoder.text("observed_at")?;
        spec.observed_at.write(&mut encoder)?;
        encoder.text("budget_admission")?;
        spec.budget_admission.write(&mut encoder)?;
        let encoded = encoder.finish();
        let mut input = Self::inspect(&encoded, limits)?;
        input.encoded = encoded;
        Ok(input)
    }

    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Self, InputError> {
        let mut input = Self::inspect(bytes, limits)?;
        input.encoded = crate::format::copy_validated(bytes)?;
        Ok(input)
    }

    /// Recheck caller/Definition wire caps without taking another payload ownership copy.
    pub(crate) fn validate_encoded(bytes: &[u8], limits: &Limits) -> Result<(), InputError> {
        Self::inspect(bytes, limits).map(|_| ())
    }

    fn inspect(bytes: &[u8], limits: &Limits) -> Result<Self, InputError> {
        validate_drisl(bytes, &limits.decode_limits())?;
        let fields = ValueView::preflighted(bytes)
            .fields([
                "kind",
                "version",
                "event_id",
                "cause",
                "observed_at",
                "observation",
                "receipt",
                "budget_admission",
            ])
            .map_err(|_| InputError::InvalidSchema)?;
        if fields[0].text().map_err(|_| InputError::InvalidSchema)? != "activation_input" {
            return Err(InputError::WrongKind);
        }
        if fields[1]
            .unsigned()
            .map_err(|_| InputError::InvalidSchema)?
            != 1
        {
            return Err(InputError::UnsupportedVersion);
        }
        let event_id = fields[2]
            .bytes()
            .map_err(|_| InputError::InvalidSchema)?
            .try_into()
            .map_err(|_| InputError::InvalidSchema)?;
        let cause = Cause::parse(fields[3].text().map_err(|_| InputError::InvalidSchema)?)?;
        let observed_at = Position::from_view(fields[4]).map_err(|_| InputError::InvalidSchema)?;
        check_observation(fields[5].encoded(), &observation_bounds(limits))?;
        let present = !fields[6].is_null();
        cause.check_receipt(present)?;
        let receipt = if present {
            Receipt::validate_encoded(fields[6].encoded(), &child_limits(limits))?;
            Some(fields[6].span())
        } else {
            None
        };
        let budget_admission =
            BudgetAdmission::from_view(fields[7]).map_err(|_| InputError::InvalidSchema)?;
        Ok(Self {
            encoded: Vec::new(),
            event_id,
            cause,
            observed_at,
            observation: fields[5].span(),
            receipt,
            budget_admission,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    pub fn cid(&self) -> DaslCid {
        DaslCid::for_block(DaslCodec::Drisl, self.as_bytes())
    }

    pub fn event_id(&self) -> Id32 {
        self.event_id
    }

    pub fn cause(&self) -> Cause {
        self.cause
    }

    pub fn observed_at(&self) -> Position {
        self.observed_at
    }

    pub fn observation_bytes(&self) -> &[u8] {
        &self.encoded[self.observation.clone()]
    }

    /// Validated terminal bytes borrowed from the same immutable input buffer, never copied here.
    pub fn receipt_bytes(&self) -> Option<&[u8]> {
        self.receipt
            .as_ref()
            .map(|span| &self.encoded[span.clone()])
    }

    pub fn budget_admission(&self) -> BudgetAdmission {
        self.budget_admission
    }
}

fn check_observation(bytes: &[u8], bounds: &DecodeLimits) -> Result<(), InputError> {
    validate_model_value(bytes, bounds)?;
    ValueView::preflighted(bytes)
        .map()
        .map_err(|_| InputError::InvalidSchema)?;
    Ok(())
}

fn observation_bounds(limits: &Limits) -> DecodeLimits {
    DecodeLimits {
        max_block_bytes: limits.max_block_bytes.min(limits.max_observation_bytes),
        max_depth: limits.max_depth.saturating_sub(1),
        max_collection_entries: limits.max_collection_entries,
    }
}

fn child_limits(limits: &Limits) -> Limits {
    Limits {
        max_depth: limits.max_depth.saturating_sub(1),
        ..*limits
    }
}

#[cfg(test)]
mod tests;
