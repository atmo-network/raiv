//! Shared fallible byte sink for bounded canonical encoding; no CBOR primitives live here.

use super::CodecError;
use alloc::vec::Vec;
use minicbor::encode::Write;

pub(super) struct BoundedWriter {
    pub(super) bytes: Vec<u8>,
    pub(super) max_bytes: u64,
}

impl BoundedWriter {
    pub(super) fn check_additional(&self, additional: usize) -> Result<(), CodecError> {
        let length = self
            .bytes
            .len()
            .checked_add(additional)
            .ok_or(CodecError::ByteLimit)?;
        if length as u64 > self.max_bytes {
            return Err(CodecError::ByteLimit);
        }
        Ok(())
    }
}

impl Write for BoundedWriter {
    type Error = CodecError;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.check_additional(bytes.len())?;
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| CodecError::AllocationFailure)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
