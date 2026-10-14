//! Owns DASL identifiers and canonical wire/model-value codecs; no storage or authority.
//! Typed record schemas live with their domain owners.

mod codec;
mod hash;
mod map_edit;
mod model_value;
mod record;
mod view;
mod writer;

#[cfg(test)]
pub(crate) mod oracle;

pub(crate) use hash::domain_hash;
pub(crate) use map_edit::merge_map_field;
pub(crate) use record::{encode_unsigned_record, RecordEncoder};
pub(crate) use view::ValueView;

pub use codec::{validate_drisl, validate_model_value, CodecError, DecodeLimits};
pub use model_value::{ModelValue, ValueRef};

use alloc::vec::Vec;
use core::{fmt, str::FromStr};
use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha256};

fn try_push<T>(stack: &mut Vec<T>, frame: T) -> Result<(), CodecError> {
    stack
        .try_reserve(1)
        .map_err(|_| CodecError::AllocationFailure)?;
    stack.push(frame);
    Ok(())
}

/// Callers validate complete bytes and resource bounds before this ownership copy.
pub(crate) fn copy_validated(bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| CodecError::AllocationFailure)?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}

/// A protocol scalar of exactly 32 raw bytes, never an implicit graph Link.
pub type Id32 = [u8; 32];

/// The only codecs admitted by the DASL profile. Structural Raiv blocks require DRISL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DaslCodec {
    Raw = 0x55,
    Drisl = 0x71,
}

/// A validated CIDv1 with a SHA2-256 digest and a restricted DASL codec.
///
/// Identifier validity does not establish block schema validity or canonical HEAD.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DaslCid([u8; 36]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CidError {
    InvalidLength,
    InvalidPrefix,
    InvalidText,
    WrongStateCodec,
    IntegrityFailure,
}

impl fmt::Display for CidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLength => "CID has the wrong length",
            Self::InvalidPrefix => "unsupported CID version, codec, hash, or digest length",
            Self::InvalidText => "CID text is not canonical lowercase unpadded base32",
            Self::WrongStateCodec => "structural state requires the DRISL codec",
            Self::IntegrityFailure => "block bytes do not match the CID digest",
        })
    }
}

impl DaslCid {
    pub const BINARY_LEN: usize = 36;
    pub const TEXT_LEN: usize = 59;

    /// Reconstruct an identifier under an explicitly selected profile, not a guessed hash type.
    pub fn from_digest(codec: DaslCodec, digest: [u8; 32]) -> Self {
        let mut bytes = [0; 36];
        bytes[..4].copy_from_slice(&[0x01, codec as u8, 0x12, 0x20]);
        bytes[4..].copy_from_slice(&digest);
        Self(bytes)
    }

    /// Hash the complete block without re-encoding, chunking, or inspecting its schema.
    pub fn for_block(codec: DaslCodec, bytes: &[u8]) -> Self {
        Self::from_digest(codec, Sha256::digest(bytes).into())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CidError> {
        let bytes: [u8; 36] = bytes.try_into().map_err(|_| CidError::InvalidLength)?;
        if bytes[0] != 0x01
            || !matches!(bytes[1], 0x55 | 0x71)
            || bytes[2] != 0x12
            || bytes[3] != 0x20
        {
            return Err(CidError::InvalidPrefix);
        }
        Ok(Self(bytes))
    }

    /// Tag-42 identity prefix plus the exact fixed structural CID profile.
    fn from_link_payload(payload: &[u8]) -> Result<Self, CodecError> {
        Self::from_bytes(payload.strip_prefix(&[0]).ok_or(CodecError::InvalidLink)?)
            .and_then(Self::require_state)
            .map_err(|_| CodecError::InvalidLink)
    }

    pub fn as_bytes(&self) -> &[u8; 36] {
        &self.0
    }

    pub fn digest(&self) -> [u8; 32] {
        self.0[4..].try_into().expect("validated fixed CID length")
    }

    pub fn codec(&self) -> DaslCodec {
        if self.0[1] == DaslCodec::Drisl as u8 {
            DaslCodec::Drisl
        } else {
            DaslCodec::Raw
        }
    }

    /// Validate the stricter profile required at every structural root and link boundary.
    pub fn require_state(self) -> Result<Self, CidError> {
        if self.codec() != DaslCodec::Drisl {
            return Err(CidError::WrongStateCodec);
        }
        Ok(self)
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<(), CidError> {
        if Self::for_block(self.codec(), bytes) != *self {
            return Err(CidError::IntegrityFailure);
        }
        Ok(())
    }
}

impl TryFrom<&[u8]> for DaslCid {
    type Error = CidError;
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(bytes)
    }
}

impl FromStr for DaslCid {
    type Err = CidError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.as_bytes();
        if text.len() != Self::TEXT_LEN {
            return Err(CidError::InvalidLength);
        }
        if text[0] != b'b'
            || !text[1..]
                .iter()
                .all(|b| b.is_ascii_lowercase() || matches!(b, b'2'..=b'7'))
        {
            return Err(CidError::InvalidText);
        }
        let mut encoded = [0; 58];
        encoded.copy_from_slice(&text[1..]);
        encoded.make_ascii_uppercase();
        let mut decoded = [0; 36];
        BASE32_NOPAD
            .decode_mut(&encoded, &mut decoded)
            .map_err(|_| CidError::InvalidText)?;
        Self::from_bytes(&decoded)
    }
}

impl fmt::Display for DaslCid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut encoded = [0; 58];
        BASE32_NOPAD.encode_mut(&self.0, &mut encoded);
        encoded.make_ascii_lowercase();
        let text = core::str::from_utf8(&encoded).map_err(|_| fmt::Error)?;
        write!(f, "b{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::ToString, vec::Vec};

    // Independent SHA-256/base32 oracle: Python hashlib + base64, input hex a0.
    const EMPTY_MAP: &str = "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua";
    const DIGEST: [u8; 32] = [
        0xc1, 0x9a, 0x79, 0x7f, 0xa1, 0xfd, 0x59, 0x0c, 0xd2, 0xe5, 0xb4, 0x2d, 0x1c, 0xf5, 0xf2,
        0x46, 0xe2, 0x9b, 0x91, 0x68, 0x4e, 0x2f, 0x87, 0x40, 0x4b, 0x81, 0xdc, 0x34, 0x5c, 0x7a,
        0x56, 0xa0,
    ];

    #[test]
    fn independent_empty_map_vector_and_anchor_reconstruction() {
        let cid = DaslCid::for_block(DaslCodec::Drisl, &[0xa0]);
        assert_eq!(cid.digest(), DIGEST);
        assert_eq!(cid.to_string(), EMPTY_MAP);
        assert_eq!(EMPTY_MAP.parse(), Ok(cid));
        assert_eq!(DaslCid::from_bytes(cid.as_bytes()), Ok(cid));
        assert_eq!(DaslCid::from_digest(DaslCodec::Drisl, DIGEST), cid);
        assert_eq!(cid.require_state(), Ok(cid));
        assert_eq!(cid.verify(&[0xa0]), Ok(()));
        assert_eq!(cid.verify(&[0xa0, 0]), Err(CidError::IntegrityFailure));
    }

    #[test]
    fn binary_profile_rejects_every_invalid_prefix_octet_and_length() {
        let valid = DaslCid::from_digest(DaslCodec::Drisl, DIGEST);
        for index in 0..4 {
            for byte in 0..=u8::MAX {
                if byte == valid.0[index] || (index == 1 && byte == 0x55) {
                    continue;
                }
                let mut bytes = valid.0;
                bytes[index] = byte;
                assert_eq!(DaslCid::from_bytes(&bytes), Err(CidError::InvalidPrefix));
            }
        }
        for len in 0..36 {
            assert_eq!(
                DaslCid::from_bytes(&valid.0[..len]),
                Err(CidError::InvalidLength)
            );
        }
        let mut trailing = Vec::from(valid.0);
        trailing.push(0);
        assert_eq!(DaslCid::from_bytes(&trailing), Err(CidError::InvalidLength));
    }

    #[test]
    fn structural_link_payload_has_one_exact_profile_across_preflight_and_view() {
        let cid = DaslCid::from_digest(DaslCodec::Drisl, DIGEST);
        let check = |payload: &[u8], expected: Result<DaslCid, CodecError>| {
            assert_eq!(DaslCid::from_link_payload(payload), expected);
            let encoded = oracle::encode(&ciborium::value::Value::Tag(
                42,
                alloc::boxed::Box::new(ciborium::value::Value::Bytes(payload.to_vec())),
            ));
            assert_eq!(
                validate_drisl(&encoded, &DecodeLimits::default()),
                expected.map(|_| ())
            );
            assert_eq!(ValueView::preflighted(&encoded).link(), expected);
            assert_eq!(
                validate_model_value(&encoded, &DecodeLimits::default()),
                Err(CodecError::UnsupportedValue)
            );
        };
        let mut payload = alloc::vec![0];
        payload.extend_from_slice(cid.as_bytes());
        for prefix in 0..=u8::MAX {
            payload[0] = prefix;
            check(
                &payload,
                if prefix == 0 {
                    Ok(cid)
                } else {
                    Err(CodecError::InvalidLink)
                },
            );
        }
        payload[0] = 0;
        for length in 0..payload.len() {
            check(&payload[..length], Err(CodecError::InvalidLink));
        }
        let mut trailing = payload.clone();
        trailing.push(0);
        check(&trailing, Err(CodecError::InvalidLink));
        for (index, octet) in [(1, 2), (2, 0x55), (3, 0x13), (4, 0x1f)] {
            let mut changed = payload.clone();
            changed[index] = octet;
            check(&changed, Err(CodecError::InvalidLink));
        }
    }

    #[test]
    fn raw_is_valid_dasl_but_not_structural_state() {
        let cid = DaslCid::for_block(DaslCodec::Raw, &[0xa0]);
        assert_eq!(DaslCid::from_bytes(cid.as_bytes()), Ok(cid));
        assert_eq!(cid.to_string().parse(), Ok(cid));
        assert_eq!(cid.codec(), DaslCodec::Raw);
        assert_eq!(cid.require_state(), Err(CidError::WrongStateCodec));
        assert_eq!(cid.verify(&[0xa0]), Ok(()));
    }

    #[test]
    fn text_rejects_wrong_base_case_padding_unicode_and_trailing_bits() {
        for text in [
            EMPTY_MAP.to_uppercase(),
            format!("z{}", &EMPTY_MAP[1..]),
            format!("{EMPTY_MAP}="),
            format!("{EMPTY_MAP}a"),
            EMPTY_MAP[..58].to_string(),
            format!("bé{}", &EMPTY_MAP[3..]),
            // The final base32 symbol has two data bits and three zero padding bits.
            format!("{}b", &EMPTY_MAP[..58]),
            format!("{}0", &EMPTY_MAP[..58]),
        ] {
            assert!(text.parse::<DaslCid>().is_err(), "{text}");
        }
    }

    #[test]
    fn every_final_data_symbol_and_digest_byte_roundtrips() {
        for byte in 0..=u8::MAX {
            let cid = DaslCid::from_digest(DaslCodec::Drisl, [byte; 32]);
            let text = cid.to_string();
            assert_eq!(text.len(), DaslCid::TEXT_LEN);
            assert_eq!(text.parse(), Ok(cid));
        }
    }
}
