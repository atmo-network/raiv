//! Fixed-profile hash preimages. Domains belong to identity; CBOR stays private to format.

use super::{CodecError, Id32};
use minicbor::Encoder;
use sha2::{Digest, Sha256};

/// Encode only the fixed scalar prefix, then stream an optional validated record tail.
/// Callers use the pinned Raiv domains (at most 14 bytes) and at most three Id32s.
pub(crate) fn domain_hash(
    domain: &'static str,
    ids: &[Id32],
    index: Option<u64>,
    tail: Option<&[u8]>,
) -> Result<Id32, CodecError> {
    if domain.len() > 14 || ids.len() > 3 {
        return Err(CodecError::Malformed);
    }
    // At most: 1 array header + 15 text bytes + 3*34 Id32 bytes + 9 uint bytes = 127.
    let mut prefix = [0; 128];
    let mut encoder = Encoder::new(&mut prefix[..]);
    encoder
        .array(1 + ids.len() as u64 + u64::from(index.is_some()) + u64::from(tail.is_some()))
        .map_err(|_| CodecError::Malformed)?;
    encoder.str(domain).map_err(|_| CodecError::Malformed)?;
    for id in ids {
        encoder.bytes(id).map_err(|_| CodecError::Malformed)?;
    }
    if let Some(index) = index {
        encoder.u64(index).map_err(|_| CodecError::Malformed)?;
    }
    let used = 128 - encoder.writer().len();
    let mut hash = Sha256::new();
    hash.update(&prefix[..used]);
    if let Some(record) = tail {
        hash.update(record);
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_prefix_capacity_handles_its_maximum_and_rejects_widened_contracts() {
        let ids = [[0xff; 32]; 3];
        assert!(domain_hash("raiv/episode/1", &ids, Some(u64::MAX), Some(&[0xf6])).is_ok());
        assert_eq!(
            domain_hash("raiv/episode/10", &ids, None, None),
            Err(CodecError::Malformed)
        );
        assert_eq!(
            domain_hash("raiv/episode/1", &[[0; 32]; 4], None, None),
            Err(CodecError::Malformed)
        );
    }
}
