//! Closed world codec: canonical header and ALL nested ledger/cost/conservation checks
//! precede ledger payload ownership. Costs are known synthetic units only, never money.
use super::*;
use minicbor::{encode::Write, Decoder, Encoder};
use raiv::format::validate_drisl;
struct Writer {
    bytes: Vec<u8>,
    max: u64,
}
impl Write for Writer {
    type Error = WorldError;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), WorldError> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n as u64 > self.max)
        {
            return Err(WorldError::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| WorldError::Allocation)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
fn error(e: minicbor::encode::Error<WorldError>) -> WorldError {
    e.into_write().unwrap_or(WorldError::Schema)
}
pub(super) fn costs(units: u64) -> Result<Vec<u8>, WorldError> {
    let mut e = Encoder::new(Writer {
        bytes: Vec::new(),
        max: 32,
    });
    e.map(1)
        .and_then(|e| e.str("virtual_units"))
        .and_then(|e| e.u64(units))
        .map_err(error)?;
    Ok(e.into_writer().bytes)
}
pub(super) fn used(
    ledger: &checkpoint::ExecutorCheckpointView<'_>,
    wire: &DecodeLimits,
    id: Id32,
) -> Result<u64, WorldError> {
    let mut used = 0u64;
    for (row, receipt_bytes) in ledger.records() {
        let units = match row.status {
            Status::Ready => 0,
            Status::Submitted => 1,
            Status::Terminal => {
                let bytes = receipt_bytes.ok_or(WorldError::Schema)?;
                let bound = Limits {
                    max_block_bytes: wire.max_block_bytes,
                    max_receipt_bytes: wire.max_block_bytes,
                    max_depth: wire.max_depth,
                    max_collection_entries: wire.max_collection_entries,
                    ..Limits::default()
                };
                let receipt = ReceiptView::decode(bytes, &bound)
                    .map_err(|e| WorldError::Simulator(SimulatorError::Receipt(e)))?;
                if receipt.finalized_at().domain != id
                    || receipt.finalized_at().number != row.anchor.revision
                    || receipt.execution_reference().is_some()
                    || receipt.finalized_at().reference.is_some()
                {
                    return Err(WorldError::Schema);
                }
                let mut d = Decoder::new(receipt.costs_bytes());
                if d.map().map_err(|_| WorldError::Schema)? != Some(1)
                    || d.str().map_err(|_| WorldError::Schema)? != "virtual_units"
                {
                    return Err(WorldError::Schema);
                }
                let units = d.u64().map_err(|_| WorldError::Schema)?;
                if units > 1
                    || d.position() != receipt.costs_bytes().len()
                    || receipt.outcome() == Outcome::NotExecuted && units != 0
                {
                    return Err(WorldError::Schema);
                }
                units
            }
        };
        used = used.checked_add(units).ok_or(WorldError::Limit)?;
    }
    Ok(used)
}
pub(super) fn encode(state: &WorldState, limits: &ExecutorLimits) -> Result<Vec<u8>, WorldError> {
    let view = ExecutorCheckpoint::borrowed(state.ledger.as_bytes(), limits)
        .map_err(WorldError::Checkpoint)?;
    if state
        .initial
        .checked_sub(used(&view, &limits.wire, state.id)?)
        != Some(state.available)
    {
        return Err(WorldError::Schema);
    }
    let mut e = Encoder::new(Writer {
        bytes: Vec::new(),
        max: limits.wire.max_block_bytes,
    });
    e.array(6)
        .and_then(|e| e.str("raiv-host/scripted-world"))
        .and_then(|e| e.u64(1))
        .and_then(|e| e.bytes(&state.id))
        .and_then(|e| e.u64(state.initial))
        .and_then(|e| e.u64(state.available))
        .and_then(|e| e.bytes(state.ledger.as_bytes()))
        .map_err(error)?;
    let bytes = e.into_writer().bytes;
    validate_drisl(&bytes, &limits.wire).map_err(WorldError::Codec)?;
    Ok(bytes)
}
pub(super) fn decode(bytes: &[u8], limits: &ExecutorLimits) -> Result<WorldState, WorldError> {
    validate_drisl(bytes, &limits.wire).map_err(WorldError::Codec)?;
    let mut d = Decoder::new(bytes);
    if d.array().map_err(|_| WorldError::Schema)? != Some(6)
        || d.str().map_err(|_| WorldError::Schema)? != "raiv-host/scripted-world"
        || d.u64().map_err(|_| WorldError::Schema)? != 1
    {
        return Err(WorldError::Schema);
    }
    let id = d
        .bytes()
        .map_err(|_| WorldError::Schema)?
        .try_into()
        .map_err(|_| WorldError::Schema)?;
    let initial = d.u64().map_err(|_| WorldError::Schema)?;
    let available = d.u64().map_err(|_| WorldError::Schema)?;
    let ledger = d.bytes().map_err(|_| WorldError::Schema)?;
    if d.position() != bytes.len() {
        return Err(WorldError::Schema);
    }
    let borrowed = ExecutorCheckpoint::borrowed(ledger, limits).map_err(WorldError::Checkpoint)?;
    if initial.checked_sub(used(&borrowed, &limits.wire, id)?) != Some(available) {
        return Err(WorldError::Schema);
    }
    let ledger = ExecutorCheckpoint::decode(ledger, limits).map_err(WorldError::Checkpoint)?;
    Ok(WorldState {
        id,
        initial,
        available,
        ledger,
    })
}
