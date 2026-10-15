//! Private closed host-input encoding; existing Core payloads are emitted unchanged.

use super::RunError;
use crate::MAX_FRAME_BYTES;
use minicbor::{encode::Write, Encoder};
use raiv::{graph::RestoredAgent, preparation::PreparedInput};

struct Writer(Vec<u8>);

impl Write for Writer {
    type Error = RunError;

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|len| len > MAX_FRAME_BYTES)
        {
            return Err(RunError::Frame);
        }
        self.0
            .try_reserve(bytes.len())
            .map_err(|_| RunError::Allocation)?;
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

fn encode(
    kind: &str,
    payload: impl FnOnce(&mut Encoder<Writer>) -> Result<(), RunError>,
) -> Result<Vec<u8>, RunError> {
    let mut e = Encoder::new(Writer(Vec::new()));
    e.map(3)
        .and_then(|e| e.str("type"))
        .and_then(|e| e.str(kind))
        .and_then(|e| e.str("payload"))
        .map_err(error)?;
    payload(&mut e)?;
    e.str("abi_version").and_then(|e| e.u64(1)).map_err(error)?;
    Ok(e.into_writer().0)
}

fn error(error: minicbor::encode::Error<RunError>) -> RunError {
    error.into_write().unwrap_or(RunError::Frame)
}

pub(super) fn activate(
    source: &RestoredAgent,
    packet: &PreparedInput,
) -> Result<Vec<u8>, RunError> {
    encode("activate", |e| {
        e.map(3)
            .and_then(|e| e.str("root"))
            .and_then(|e| e.bytes(source.root().as_bytes()))
            .and_then(|e| e.str("input"))
            .and_then(|e| e.bytes(packet.input().as_bytes()))
            .and_then(|e| e.str("blocks"))
            .and_then(|e| e.array(source.block_count()))
            .map_err(error)?;
        for (cid, body) in source.blocks() {
            e.map(2)
                .and_then(|e| e.str("cid"))
                .and_then(|e| e.bytes(cid.as_bytes()))
                .and_then(|e| e.str("body"))
                .and_then(|e| e.bytes(body))
                .map_err(error)?;
        }
        Ok(())
    })
}

pub(super) fn reply(id: u64, bytes: &[u8]) -> Result<Vec<u8>, RunError> {
    encode("inference_reply", |e| {
        e.map(2).and_then(|e| e.str("reply")).map_err(error)?;
        e.writer_mut().write_all(bytes)?;
        e.str("request_id").and_then(|e| e.u64(id)).map_err(error)?;
        Ok(())
    })
}

pub(super) fn error_frame(id: u64, code: &str) -> Result<Vec<u8>, RunError> {
    encode("inference_error", |e| {
        e.map(2)
            .and_then(|e| e.str("code"))
            .and_then(|e| e.str(code))
            .and_then(|e| e.str("request_id"))
            .and_then(|e| e.u64(id))
            .map_err(error)?;
        Ok(())
    })
}
