//! Crate-private canonical assembly of typed runtime records and unsigned subrecords.
//! Field membership belongs to the typed record owner; no generic IPLD API is exported.

use super::{writer::BoundedWriter, CodecError, DaslCid, DecodeLimits};
use alloc::vec::Vec;
use minicbor::{encode::Write, Encoder};

/// Trusted typed-record assembly. Owners validate embedded chunks and combined depth
/// before copying; their schema inspection validates the finished complete record.
pub(crate) struct RecordEncoder {
    encoder: Encoder<BoundedWriter>,
    bounds: DecodeLimits,
}

impl RecordEncoder {
    pub(crate) fn new(bounds: &DecodeLimits) -> Result<Self, CodecError> {
        if bounds.max_depth == 0 {
            return Err(CodecError::DepthLimit);
        }
        let writer = BoundedWriter {
            bytes: Vec::new(),
            max_bytes: bounds.max_block_bytes,
        };
        Ok(Self {
            encoder: Encoder::new(writer),
            bounds: *bounds,
        })
    }

    pub(crate) fn map(&mut self, count: u64) -> Result<(), CodecError> {
        self.collection(count)?;
        self.encoder.map(count)?;
        Ok(())
    }

    pub(crate) fn list(&mut self, count: u64) -> Result<(), CodecError> {
        self.collection(count)?;
        self.encoder.array(count)?;
        Ok(())
    }

    pub(crate) fn text(&mut self, value: &str) -> Result<(), CodecError> {
        self.encoder.str(value)?;
        Ok(())
    }

    pub(crate) fn unsigned(&mut self, value: u64) -> Result<(), CodecError> {
        self.encoder.u64(value)?;
        Ok(())
    }

    pub(crate) fn boolean(&mut self, value: bool) -> Result<(), CodecError> {
        self.encoder.bool(value)?;
        Ok(())
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) -> Result<(), CodecError> {
        self.encoder.bytes(value)?;
        Ok(())
    }

    pub(crate) fn link(&mut self, cid: DaslCid) -> Result<(), CodecError> {
        cid.require_state().map_err(|_| CodecError::InvalidLink)?;
        let mut payload = [0; 37];
        payload[1..].copy_from_slice(cid.as_bytes());
        self.encoder.tag(minicbor::data::Tag::new(42))?;
        self.encoder.bytes(&payload)?;
        Ok(())
    }

    pub(crate) fn null(&mut self) -> Result<(), CodecError> {
        self.encoder.null()?;
        Ok(())
    }

    /// Copy one already-preflighted canonical object, never an arbitrary untrusted stream.
    pub(crate) fn encoded(&mut self, value: &[u8]) -> Result<(), CodecError> {
        self.encoder.writer_mut().write_all(value)?;
        Ok(())
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.encoder.into_writer().bytes
    }

    fn collection(&self, count: u64) -> Result<(), CodecError> {
        if count > self.bounds.max_collection_entries {
            return Err(CodecError::CollectionLimit);
        }
        Ok(())
    }
}

pub(crate) fn encode_unsigned_record<const N: usize>(
    mut fields: [(&str, u64); N],
    limits: &DecodeLimits,
) -> Result<Vec<u8>, CodecError> {
    if limits.max_depth == 0 || (N != 0 && limits.max_depth < 2) {
        return Err(CodecError::DepthLimit);
    }
    if N as u64 > limits.max_collection_entries {
        return Err(CodecError::CollectionLimit);
    }
    // Sorting a fixed array needs no scratch allocation. For minimal text headers,
    // encoded-key ordering is UTF-8 byte length followed by the payload bytes.
    fields.sort_unstable_by_key(|(key, _)| (key.len(), *key));
    if fields.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(CodecError::DuplicateKey);
    }
    let mut encoder = RecordEncoder::new(limits)?;
    encoder.map(N as u64)?;
    for (key, value) in fields {
        encoder.text(key)?;
        encoder.unsigned(value)?;
    }
    Ok(encoder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{validate_drisl, validate_model_value};

    #[test]
    fn structural_link_writer_and_view_match_exact_profile_and_output_bounds() {
        let cid = DaslCid::for_block(crate::format::DaslCodec::Drisl, &[0xa0]);
        let mut expected = alloc::vec![0xd8, 0x2a, 0x58, 0x25, 0];
        expected.extend_from_slice(cid.as_bytes());
        let bounds = DecodeLimits {
            max_block_bytes: 41,
            max_depth: 2,
            ..DecodeLimits::default()
        };
        let mut encoder = RecordEncoder::new(&bounds).unwrap();
        encoder.link(cid).unwrap();
        let encoded = encoder.finish();
        assert_eq!(encoded, expected);
        assert_eq!(validate_drisl(&encoded, &bounds), Ok(()));
        assert_eq!(
            crate::format::ValueView::preflighted(&encoded).link(),
            Ok(cid)
        );
        let mut encoder = RecordEncoder::new(&bounds).unwrap();
        let raw = DaslCid::from_digest(crate::format::DaslCodec::Raw, [0; 32]);
        assert_eq!(encoder.link(raw), Err(CodecError::InvalidLink));
        assert!(encoder.finish().is_empty());
        let mut encoder = RecordEncoder::new(&DecodeLimits {
            max_block_bytes: 40,
            ..bounds
        })
        .unwrap();
        assert_eq!(encoder.link(cid), Err(CodecError::ByteLimit));
        assert_eq!(
            validate_drisl(
                &encoded,
                &DecodeLimits {
                    max_depth: 1,
                    ..bounds
                }
            ),
            Err(CodecError::DepthLimit)
        );
        assert!(crate::format::ValueView::preflighted(&[0xf6])
            .link()
            .is_err());
    }

    #[test]
    fn runtime_u64_is_preserved_without_broadening_model_value_integers() {
        let limits = DecodeLimits::default();
        let fields = [("aa", 1), ("b", u64::MAX)];
        let bytes = encode_unsigned_record(fields, &limits).unwrap();
        let expected = [
            0xa2, 0x61, b'b', 0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x62, b'a',
            b'a', 1,
        ];
        assert_eq!(bytes, expected);
        assert_eq!(validate_drisl(&bytes, &limits), Ok(()));
        assert_eq!(
            validate_model_value(&bytes, &limits),
            Err(CodecError::ModelIntegerRange)
        );
        assert_eq!(
            encode_unsigned_record([("b", u64::MAX), ("aa", 1)], &limits),
            Ok(bytes)
        );
    }

    #[test]
    fn canonical_text_key_order_matches_independent_header_and_utf8_boundaries() {
        let keys = [
            "z".repeat(255),
            "é".into(),
            "a".repeat(24),
            "zz".into(),
            "a".repeat(256),
            "🦀".into(),
            "z".repeat(23),
            "a".into(),
            "".into(),
        ];
        let fields: [(&str, u64); 9] = core::array::from_fn(|i| (keys[i].as_str(), i as u64));
        let expected = crate::format::oracle::encode(&ciborium::value::Value::Map(
            fields
                .iter()
                .map(|(key, value)| {
                    (
                        ciborium::value::Value::Text((*key).into()),
                        ciborium::value::Value::Integer((*value).into()),
                    )
                })
                .collect(),
        ));
        let bounds = DecodeLimits::default();
        assert_eq!(
            encode_unsigned_record(fields, &bounds),
            Ok(expected.clone())
        );
        let model: [(&str, crate::format::ValueRef<'_>); 9] = core::array::from_fn(|i| {
            (
                fields[i].0,
                crate::format::ValueRef::Integer(fields[i].1 as i64),
            )
        });
        assert_eq!(
            crate::format::ModelValue::encode(crate::format::ValueRef::Map(&model), &bounds)
                .unwrap()
                .as_bytes(),
            expected
        );
    }

    #[test]
    fn fixed_record_encoding_checks_structure_and_output_bounds() {
        let limits = DecodeLimits::default();
        assert_eq!(
            encode_unsigned_record([("a", 1), ("a", 2)], &limits),
            Err(CodecError::DuplicateKey)
        );
        let limits = DecodeLimits {
            max_collection_entries: 0,
            ..limits
        };
        assert_eq!(
            encode_unsigned_record([("a", 1)], &limits),
            Err(CodecError::CollectionLimit)
        );
        let limits = DecodeLimits {
            max_depth: 1,
            ..limits
        };
        assert_eq!(encode_unsigned_record([], &limits), Ok(alloc::vec![0xa0]));
        let limits = DecodeLimits {
            max_collection_entries: 1,
            ..limits
        };
        assert_eq!(
            encode_unsigned_record([("a", 1)], &limits),
            Err(CodecError::DepthLimit)
        );
        let limits = DecodeLimits {
            max_depth: 2,
            max_block_bytes: 3,
            ..limits
        };
        assert_eq!(
            encode_unsigned_record([("a", 1)], &limits),
            Err(CodecError::ByteLimit)
        );
    }
}
