//! Terminal executor data, not host provenance, phase admission, or execution authority.

use crate::command::Command;
use crate::format::{
    validate_drisl, validate_model_value, CodecError, DecodeLimits, Id32, RecordEncoder, ValueView,
};
use crate::identity::command_digest;
use crate::limits::Limits;
use crate::position::Position;
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptError {
    Codec(CodecError),
    InvalidSchema,
}

impl From<CodecError> for ReceiptError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::InvalidSchema => f.write_str(
                "Receipt requires exact terminal outcome/ID/digest/position/model-map fields",
            ),
        }
    }
}

impl core::error::Error for ReceiptError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

/// Unknown, pending, and transport interruptions are not terminal outcomes. These labels
/// do not prove execution or finality; a trusted host must supply that evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Succeeded,
    Failed,
    NotExecuted,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::NotExecuted => "not_executed",
        }
    }

    fn parse(text: &str) -> Result<Self, ReceiptError> {
        match text {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "not_executed" => Ok(Self::NotExecuted),
            _ => Err(ReceiptError::InvalidSchema),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReceiptSpec<'a> {
    pub action_id: Id32,
    pub command_digest: Id32,
    pub outcome: Outcome,
    pub result: &'a [u8],
    pub execution_reference: Option<&'a str>,
    pub finalized_at: Position,
    pub costs: &'a [u8],
}

/// A borrowed complete terminal Receipt, not phase admission or trusted host evidence.
/// Result/cost maps remain exact model-value spans; runtime Position/IDs remain typed.
#[derive(Clone, Copy)]
pub struct ReceiptView<'a> {
    bytes: &'a [u8],
    fields: [ValueView<'a>; 7],
}
impl<'a> ReceiptView<'a> {
    /// Validate untrusted borrowed bytes without allocating another Receipt payload.
    /// Canonical preflight may allocate bounded traversal metadata.
    pub fn decode(bytes: &'a [u8], limits: &Limits) -> Result<Self, ReceiptError> {
        Receipt::validate_encoded(bytes, limits)?;
        Ok(Self::from_validated(bytes))
    }
    pub(crate) fn from_validated(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            fields: validated_fields(bytes),
        }
    }
    pub fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }
    pub fn action_id(self) -> Id32 {
        self.fields[0]
            .bytes()
            .expect("validated action ID")
            .try_into()
            .expect("validated Id32")
    }
    pub fn command_digest(self) -> Id32 {
        self.fields[1]
            .bytes()
            .expect("validated command digest")
            .try_into()
            .expect("validated Id32")
    }
    pub fn outcome(self) -> Outcome {
        Outcome::parse(self.fields[2].text().expect("validated outcome text"))
            .expect("validated terminal outcome")
    }
    pub fn result_bytes(self) -> &'a [u8] {
        self.fields[3].encoded()
    }
    pub fn execution_reference(self) -> Option<&'a str> {
        (!self.fields[4].is_null()).then(|| {
            self.fields[4]
                .text()
                .expect("validated execution reference")
        })
    }
    pub fn finalized_at(self) -> Position {
        Position::from_view(self.fields[5]).expect("validated Position")
    }
    pub fn costs_bytes(self) -> &'a [u8] {
        self.fields[6].encoded()
    }
}

/// Immutable complete subrecord with borrowed views into validated owned bytes.
/// Construction validates data only; it cannot establish a host source or admit a phase.
#[derive(Debug, Eq, PartialEq)]
pub struct Receipt {
    encoded: Vec<u8>,
    action_id: Id32,
    command_digest: Id32,
    outcome: Outcome,
    result: Range<usize>,
    execution_reference: Option<Range<usize>>,
    finalized_at: Position,
    costs: Range<usize>,
}

const RECEIPT_FIELDS: [&str; 7] = [
    "action_id",
    "command_digest",
    "outcome",
    "result",
    "execution_reference",
    "finalized_at",
    "costs",
];

impl Receipt {
    /// Encode terminal data without executing, admitting, or authenticating it.
    ///
    /// ```
    /// use raiv::receipt::{Receipt, ReceiptSpec, Outcome};
    /// use raiv::position::Position;
    /// use raiv::limits::Limits;
    /// let input = ReceiptSpec { action_id: [1; 32], command_digest: [2; 32],
    ///     outcome: Outcome::NotExecuted, result: &[0xa0], execution_reference: None,
    ///     finalized_at: Position { domain: [3; 32], number: u64::MAX, reference: None },
    ///     costs: &[0xa0] };
    /// let receipt = Receipt::encode(&input, &Limits::default())?;
    /// assert_eq!(receipt.finalized_at().number, u64::MAX);
    /// # Ok::<(), raiv::receipt::ReceiptError>(())
    /// ```
    pub fn encode(spec: &ReceiptSpec<'_>, limits: &Limits) -> Result<Self, ReceiptError> {
        let bounds = receipt_bounds(limits);
        if bounds.max_depth < 3 {
            return Err(CodecError::DepthLimit.into());
        }
        let child_bounds = DecodeLimits {
            max_depth: bounds.max_depth - 1,
            ..bounds
        };
        for data in [spec.result, spec.costs] {
            validate_model_value(data, &child_bounds)?;
            ValueView::preflighted(data)
                .map()
                .map_err(|_| ReceiptError::InvalidSchema)?;
        }
        let mut encoder = RecordEncoder::new(&bounds)?;
        encoder.map(7)?;
        encoder.text("costs")?;
        encoder.encoded(spec.costs)?;
        encoder.text("result")?;
        encoder.encoded(spec.result)?;
        encoder.text("outcome")?;
        encoder.text(spec.outcome.as_str())?;
        encoder.text("action_id")?;
        encoder.bytes(&spec.action_id)?;
        encoder.text("finalized_at")?;
        spec.finalized_at.write(&mut encoder)?;
        encoder.text("command_digest")?;
        encoder.bytes(&spec.command_digest)?;
        encoder.text("execution_reference")?;
        match spec.execution_reference {
            Some(reference) => encoder.text(reference)?,
            None => encoder.null()?,
        }
        let encoded = encoder.finish();
        let mut receipt = Self::inspect(&encoded, limits)?;
        receipt.encoded = encoded;
        Ok(receipt)
    }

    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Self, ReceiptError> {
        let mut receipt = Self::inspect(bytes, limits)?;
        receipt.encoded = crate::format::copy_validated(bytes)?;
        Ok(receipt)
    }

    /// Validate embedded bytes with contextual bounds, without copying their payload.
    pub(crate) fn validate_encoded(bytes: &[u8], limits: &Limits) -> Result<(), ReceiptError> {
        Self::inspect(bytes, limits).map(|_| ())
    }

    /// ActivationInput/Boundary own immutable bytes already checked under contextual caps.
    pub(crate) fn matches_in_validated(
        bytes: &[u8],
        execution_domain: Id32,
        command: &[u8],
    ) -> bool {
        let view = ReceiptView::from_validated(bytes);
        view.action_id() == crate::command::Command::action_id_in_validated(command)
            && view.command_digest()
                == crate::identity::command_digest_in_validated(execution_domain, command)
    }

    fn inspect(bytes: &[u8], limits: &Limits) -> Result<Self, ReceiptError> {
        let bounds = receipt_bounds(limits);
        validate_drisl(bytes, &bounds)?;
        let fields = ValueView::preflighted(bytes)
            .fields(RECEIPT_FIELDS)
            .map_err(|_| ReceiptError::InvalidSchema)?;
        let action_id = fields[0]
            .bytes()
            .map_err(|_| ReceiptError::InvalidSchema)?
            .try_into()
            .map_err(|_| ReceiptError::InvalidSchema)?;
        let command_digest = fields[1]
            .bytes()
            .map_err(|_| ReceiptError::InvalidSchema)?
            .try_into()
            .map_err(|_| ReceiptError::InvalidSchema)?;
        let outcome = Outcome::parse(fields[2].text().map_err(|_| ReceiptError::InvalidSchema)?)?;
        for field in [fields[3], fields[6]] {
            field.map().map_err(|_| ReceiptError::InvalidSchema)?;
            validate_model_value(field.encoded(), &bounds)?;
        }
        let execution_reference = if fields[4].is_null() {
            None
        } else {
            Some(
                fields[4]
                    .text_span()
                    .map_err(|_| ReceiptError::InvalidSchema)?,
            )
        };
        let finalized_at =
            Position::from_view(fields[5]).map_err(|_| ReceiptError::InvalidSchema)?;
        Ok(Self {
            encoded: Vec::new(),
            action_id,
            command_digest,
            outcome,
            result: fields[3].span(),
            execution_reference,
            finalized_at,
            costs: fields[6].span(),
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    /// Borrow already validated fields without repeating preflight or copying payloads.
    pub fn view(&self) -> ReceiptView<'_> {
        ReceiptView::from_validated(self.as_bytes())
    }

    pub fn action_id(&self) -> Id32 {
        self.action_id
    }

    pub fn command_digest(&self) -> Id32 {
        self.command_digest
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn result_bytes(&self) -> &[u8] {
        &self.encoded[self.result.clone()]
    }

    pub fn costs_bytes(&self) -> &[u8] {
        &self.encoded[self.costs.clone()]
    }

    pub fn execution_reference(&self) -> Option<&str> {
        self.execution_reference.as_ref().map(|span| {
            core::str::from_utf8(&self.encoded[span.clone()])
                .expect("validated immutable Receipt text")
        })
    }

    pub fn finalized_at(&self) -> Position {
        self.finalized_at
    }

    /// Check only ID and domain-bound digest equality. The caller must separately check
    /// the pending phase, current action slot, bounds, and trusted host provenance.
    pub fn matches_command(&self, execution_domain: Id32, command: &Command) -> bool {
        self.action_id == command.action_id()
            && self.command_digest == command_digest(execution_domain, command)
    }
}

fn validated_fields(bytes: &[u8]) -> [ValueView<'_>; 7] {
    ValueView::preflighted(bytes)
        .fields(RECEIPT_FIELDS)
        .expect("validated immutable Receipt fields")
}

fn receipt_bounds(limits: &Limits) -> DecodeLimits {
    let mut bounds = limits.decode_limits();
    bounds.max_block_bytes = bounds.max_block_bytes.min(limits.max_receipt_bytes);
    bounds
}

#[cfg(test)]
mod tests;
