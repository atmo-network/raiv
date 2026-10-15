//! Compact JSON presentation of validated model values; no parsing or authority.
//! Rendering is iterative, so display needs no recursion over model-controlled depth.

use alloc::vec::Vec;
use core::fmt::{self, Write};
use minicbor::{data::Type, Decoder};

/// Compact JSON text of one validated model value, as models see it.
///
/// Object keys keep canonical order and text is escaped per RFC 8259. This is
/// presentation only: it never re-enters a codec, checkpoint or authority decision.
#[derive(Clone, Copy)]
pub struct Json<'a>(pub(crate) &'a [u8]);

impl fmt::Display for Json<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Open {
            map: bool,
            left: u64,
            first: bool,
        }
        let mut d = Decoder::new(self.0);
        let mut stack: Vec<Open> = Vec::new();
        let mut value = true;
        loop {
            if value {
                value = false;
                match d.datatype().map_err(|_| fmt::Error)? {
                    Type::Map | Type::Array => {
                        let map = d.datatype().map_err(|_| fmt::Error)? == Type::Map;
                        let left = if map { d.map() } else { d.array() }
                            .map_err(|_| fmt::Error)?
                            .ok_or(fmt::Error)?;
                        f.write_char(if map { '{' } else { '[' })?;
                        stack.try_reserve(1).map_err(|_| fmt::Error)?;
                        stack.push(Open {
                            map,
                            left,
                            first: true,
                        });
                    }
                    Type::String => text(f, d.str().map_err(|_| fmt::Error)?)?,
                    Type::Bool => f.write_str(if d.bool().map_err(|_| fmt::Error)? {
                        "true"
                    } else {
                        "false"
                    })?,
                    Type::Null => {
                        d.null().map_err(|_| fmt::Error)?;
                        f.write_str("null")?;
                    }
                    Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
                        write!(f, "{}", d.u64().map_err(|_| fmt::Error)?)?
                    }
                    Type::I8 | Type::I16 | Type::I32 | Type::I64 => {
                        write!(f, "{}", d.i64().map_err(|_| fmt::Error)?)?
                    }
                    _ => return Err(fmt::Error),
                }
            }
            let Some(open) = stack.last_mut() else {
                return Ok(());
            };
            if open.left == 0 {
                f.write_char(if open.map { '}' } else { ']' })?;
                stack.pop();
                continue;
            }
            open.left -= 1;
            if !core::mem::replace(&mut open.first, false) {
                f.write_char(',')?;
            }
            if open.map {
                text(f, d.str().map_err(|_| fmt::Error)?)?;
                f.write_char(':')?;
            }
            value = true;
        }
    }
}

impl fmt::Debug for Json<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

fn text(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in value.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if c < ' ' => write!(f, "\\u{:04x}", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::super::{oracle, DecodeLimits, ModelValue};
    use alloc::string::ToString;
    use serde_json::json;

    #[test]
    fn rendering_round_trips_through_an_independent_json_parser() {
        for value in [
            json!({}),
            json!([]),
            json!(null),
            json!({"b": [1, -2, true, false, null], "a": {"": "", "z": {}}}),
            json!([[[[]]], {"k": [{}]}]),
            json!({"text": "quote\" back\\ nl\n tab\t bell\u{7} del\u{7f} \u{202e} 🙂"}),
            json!([9007199254740991i64, -9007199254740991i64, 0]),
        ] {
            let model =
                ModelValue::decode(&oracle::json_bytes(&value), &DecodeLimits::default()).unwrap();
            let rendered = model.json().to_string();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&rendered).unwrap(),
                value
            );
            assert!(!rendered.contains(['\n', '\t', '\u{7}']));
        }
    }

    #[test]
    fn keys_keep_canonical_order_and_output_is_compact() {
        let value = json!({"bb": 1, "a": [true, null], "c": "x"});
        let model =
            ModelValue::decode(&oracle::json_bytes(&value), &DecodeLimits::default()).unwrap();
        assert_eq!(
            model.json().to_string(),
            r#"{"a":[true,null],"c":"x","bb":1}"#
        );
    }
}
