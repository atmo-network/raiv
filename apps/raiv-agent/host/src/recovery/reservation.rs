//! Never resume a VM or an Attempted/Launched record without lost-attempt policy.
use super::*;
#[cfg(test)]
mod tests;
use raiv::{input::ActivationInput, preparation::PreparedInput};
/// Reissue one new process-local capability only for a pinned, adopted, unissued
/// Reserved/zero-attempt record. Original IDs/grant/observation/exact digest survive.
/// All data/allocation/durability checks precede the one nonfallible issuance mark.
/// Drop does not release it. Attempted/Launched/Consumed have no path through here.
pub fn resume_reserved(
    registered: &RegisteredAgent,
    book: &mut AdmissionBook,
    disk: &mut DiskBlockStore,
    event: Id32,
    pin: LocalRecoveryPin,
    limits: &ManifestLimits,
    caller: &Limits,
) -> Result<AdmittedInput, RecoveryError> {
    let (index, record) = book
        .recoverable_reserved(registered, event)
        .map_err(RecoveryError::Admission)?;
    let resources = registered.resources();
    if registered.anchor() != pin.anchor
        || registered.program_binding() != pin.program
        || resources.memory_bytes != pin.resources.memory_bytes
        || resources.fuel_per_step != pin.resources.fuel_per_step
    {
        return Err(RecoveryError::PinMismatch);
    }
    let frame = disk
        .manifest(limits.wire)
        .read()
        .map_err(RecoveryError::Disk)?
        .ok_or(RecoveryError::MissingManifest)?;
    if DaslCid::for_block(DaslCodec::Raw, &frame).digest() != pin.manifest_digest {
        return Err(RecoveryError::PinMismatch);
    }
    let expected = CheckpointManifest::capture(registered, book, limits)
        .map_err(RecoveryError::Checkpoint)?
        .encode(limits)
        .map_err(RecoveryError::Checkpoint)?;
    if expected != frame {
        return Err(RecoveryError::PinMismatch);
    }
    let cid = DaslCid::from_digest(DaslCodec::Drisl, record.binding.admitted_input_digest);
    let d = registered.snapshot().definition().limits();
    let bounds = Limits {
        max_block_bytes: caller.max_block_bytes.min(d.max_block_bytes),
        max_observation_bytes: caller.max_observation_bytes.min(d.max_observation_bytes),
        max_receipt_bytes: caller.max_receipt_bytes.min(d.max_receipt_bytes),
        max_depth: caller.max_depth.min(d.max_depth),
        max_collection_entries: caller.max_collection_entries.min(d.max_collection_entries),
        ..*caller
    };
    let body = disk
        .load_capped(cid, &bounds.decode_limits())
        .map_err(RecoveryError::Disk)?;
    let input = ActivationInput::decode(&body, &bounds)
        .map_err(|e| RecoveryError::Admission(AdmissionError::Input(e)))?;
    AdmissionBook::validate_input(registered, &input, caller).map_err(RecoveryError::Admission)?;
    if input.event_id() != event || input.budget_admission() != record.grant || input.cid() != cid {
        return Err(RecoveryError::PinMismatch);
    }
    let packet =
        PreparedInput::new(record.binding, input).map_err(|_| RecoveryError::PinMismatch)?;
    disk.readback(registered.snapshot(), caller)
        .map_err(RecoveryError::Disk)?;
    disk.put(cid, packet.input().as_bytes())
        .map_err(RecoveryError::Disk)?;
    disk.manifest(limits.wire)
        .replace(Some(&frame), &frame)
        .map_err(RecoveryError::Manifest)?;
    Ok(book.issue_recovered(index, packet))
}
