//! Crate-private borrowed schema inspection after canonical preflight.
//! No owned materialization, structural schemas, or authority decisions live here.

use super::{CodecError, DaslCid};
use core::ops::Range;
use minicbor::Decoder;

#[derive(Clone, Copy)]
pub(crate) struct ValueView<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ValueView<'a> {
    /// Caller must have preflighted this complete canonical object before inspection.
    pub(crate) fn preflighted(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(crate) fn span(self) -> Range<usize> {
        self.offset..self.offset + self.bytes.len()
    }

    pub(crate) fn text(self) -> Result<&'a str, CodecError> {
        Ok(Decoder::new(self.bytes).str()?)
    }

    pub(crate) fn unsigned(self) -> Result<u64, CodecError> {
        Ok(Decoder::new(self.bytes).u64()?)
    }

    pub(crate) fn boolean(self) -> Result<bool, CodecError> {
        Ok(Decoder::new(self.bytes).bool()?)
    }

    pub(crate) fn encoded(self) -> &'a [u8] {
        self.bytes
    }

    pub(crate) fn bytes(self) -> Result<&'a [u8], CodecError> {
        Ok(Decoder::new(self.bytes).bytes()?)
    }

    /// Structural tag-42 Link only; scalar bytes or CID text are not coerced.
    pub(crate) fn link(self) -> Result<DaslCid, CodecError> {
        let mut decoder = Decoder::new(self.bytes);
        if decoder.tag()?.as_u64() != 42 {
            return Err(CodecError::InvalidLink);
        }
        DaslCid::from_link_payload(decoder.bytes()?)
    }

    pub(crate) fn text_span(self) -> Result<Range<usize>, CodecError> {
        let text = self.text()?;
        let end = self.offset + self.bytes.len();
        Ok(end - text.len()..end)
    }

    pub(crate) fn is_null(self) -> bool {
        self.bytes == [0xf6]
    }

    pub(crate) fn list(self) -> Result<ListView<'a>, CodecError> {
        let mut decoder = Decoder::new(self.bytes);
        let remaining = decoder.array()?.ok_or(CodecError::Malformed)?;
        Ok(ListView {
            decoder,
            remaining,
            offset: self.offset,
        })
    }

    /// Closed field membership after canonical preflight. Unique keys plus exact
    /// cardinality prove every declared slot is populated, never defaulted.
    pub(crate) fn fields<const N: usize>(self, names: [&str; N]) -> Result<[Self; N], CodecError> {
        let map = self.map()?;
        if map.len() != N as u64 {
            return Err(CodecError::Malformed);
        }
        let mut fields = [self; N];
        for entry in map {
            let (name, value) = entry?;
            let index = names
                .iter()
                .position(|expected| *expected == name)
                .ok_or(CodecError::Malformed)?;
            fields[index] = value;
        }
        Ok(fields)
    }

    pub(crate) fn map(self) -> Result<MapView<'a>, CodecError> {
        let mut decoder = Decoder::new(self.bytes);
        let remaining = decoder.map()?.ok_or(CodecError::Malformed)?;
        Ok(MapView {
            decoder,
            remaining,
            offset: self.offset,
        })
    }
}

pub(crate) struct MapView<'a> {
    decoder: Decoder<'a>,
    remaining: u64,
    offset: usize,
}

impl MapView<'_> {
    pub(crate) fn len(&self) -> u64 {
        self.remaining
    }
}

impl<'a> Iterator for MapView<'a> {
    type Item = Result<(&'a str, ValueView<'a>), CodecError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let result = (|| {
            let key = self.decoder.str()?;
            let value = next_value(&mut self.decoder, self.offset)?;
            Ok((key, value))
        })();
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

fn next_value<'a>(decoder: &mut Decoder<'a>, offset: usize) -> Result<ValueView<'a>, CodecError> {
    let start = decoder.position();
    // Preflight permits only definite collections; skip is iterative and nonallocating.
    decoder.skip()?;
    let end = decoder.position();
    Ok(ValueView {
        bytes: &decoder.input()[start..end],
        offset: offset + start,
    })
}

pub(crate) struct ListView<'a> {
    decoder: Decoder<'a>,
    remaining: u64,
    offset: usize,
}

impl ListView<'_> {
    pub(crate) fn len(&self) -> u64 {
        self.remaining
    }
}

impl<'a> Iterator for ListView<'a> {
    type Item = Result<ValueView<'a>, CodecError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let result = next_value(&mut self.decoder, self.offset);
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{validate_drisl, DecodeLimits};
    use alloc::vec;

    #[test]
    fn borrowed_nested_spans_remain_relative_to_the_original_block() {
        let bytes = [0xa1, 0x61, b'a', 0xa1, 0x61, b'b', 0x18, 24];
        validate_drisl(&bytes, &DecodeLimits::default()).unwrap();
        let mut root = ValueView::preflighted(&bytes).map().unwrap();
        assert_eq!(root.len(), 1);
        let (key, value) = root.next().unwrap().unwrap();
        assert_eq!(key, "a");
        assert_eq!(value.span(), 3..8);
        assert!(root.next().is_none());
        let mut inner = value.map().unwrap();
        let (key, value) = inner.next().unwrap().unwrap();
        assert_eq!(key, "b");
        assert_eq!(value.span(), 6..8);
        assert_eq!(value.unsigned(), Ok(24));
        assert!(inner.next().is_none());
    }

    #[test]
    fn scalar_schema_inspection_does_not_coerce_types() {
        let bytes = [0x61, b'a'];
        validate_drisl(&bytes, &DecodeLimits::default()).unwrap();
        let value = ValueView::preflighted(&bytes);
        assert_eq!(value.text(), Ok("a"));
        assert!(value.unsigned().is_err());
        assert!(value.map().is_err());
    }

    #[test]
    fn skip_uses_no_recursive_descent_for_deep_preflighted_values() {
        let mut bytes = vec![0xa1, 0x61, b'a'];
        bytes.extend(core::iter::repeat_n(0x81, 10_000));
        bytes.push(0);
        let limits = DecodeLimits {
            max_depth: 10_002,
            ..DecodeLimits::default()
        };
        validate_drisl(&bytes, &limits).unwrap();
        let mut root = ValueView::preflighted(&bytes).map().unwrap();
        let (_, value) = root.next().unwrap().unwrap();
        assert_eq!(value.span(), 3..bytes.len());
        assert!(root.next().is_none());
    }
}
