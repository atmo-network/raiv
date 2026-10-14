//! Test-only independent generic CBOR codec; never part of the library/runtime graph.
//! Canonical text-map ordering uses ciborium's own comparator, not Raiv/minicbor helpers.

use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use ciborium::value::{CanonicalValue, Value};
use serde_json::Value as Json;

pub(crate) fn encode(value: &Value) -> Vec<u8> {
    let mut value = value.clone();
    canonicalize(&mut value);
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&value, &mut bytes).unwrap();
    bytes
}

pub(crate) fn map_bytes(fields: BTreeMap<String, Value>) -> Vec<u8> {
    encode(&Value::Map(
        fields
            .into_iter()
            .map(|(key, value)| (Value::Text(key), value))
            .collect(),
    ))
}

/// Broad CBOR witness: deliberately permits raw-codec links for rejection tests.
pub(crate) fn link(cid: crate::format::DaslCid) -> Value {
    let mut bytes = alloc::vec![0];
    bytes.extend_from_slice(cid.as_bytes());
    Value::Tag(42, Box::new(Value::Bytes(bytes)))
}

pub(crate) fn json_bytes(value: &Json) -> Vec<u8> {
    encode(&json_value(value, false))
}

pub(crate) fn fixture_bytes(value: &Json) -> Vec<u8> {
    encode(&fixture_value(value))
}

pub(crate) fn fixture_value(value: &Json) -> Value {
    json_value(value, true)
}

/// Borrowed test-data mutation, not a new runtime value model or codec API.
pub(crate) struct MapMut<'a>(&'a mut Vec<(Value, Value)>);

pub(crate) fn map_mut(value: &mut Value) -> MapMut<'_> {
    MapMut(value.as_map_mut().expect("fixture is not a map"))
}

impl<'a> MapMut<'a> {
    pub(crate) fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.iter().map(|(key, _)| match key {
            Value::Text(key) => key,
            _ => panic!("fixture key is not text"),
        })
    }

    pub(crate) fn get_mut(self, key: &str) -> Option<&'a mut Value> {
        self.0
            .iter_mut()
            .find_map(|(name, value)| (name.as_text() == Some(key)).then_some(value))
    }

    pub(crate) fn remove(self, key: &str) -> Option<Value> {
        let index = self
            .0
            .iter()
            .position(|(name, _)| name.as_text() == Some(key))?;
        Some(self.0.remove(index).1)
    }

    pub(crate) fn insert(self, key: String, value: Value) -> Option<Value> {
        if let Some((_, slot)) = self
            .0
            .iter_mut()
            .find(|(name, _)| name.as_text() == Some(key.as_str()))
        {
            return Some(core::mem::replace(slot, value));
        }
        self.0.push((Value::Text(key), value));
        None
    }
}

fn canonicalize(value: &mut Value) {
    match value {
        Value::Map(entries) => {
            for (key, value) in entries.iter_mut() {
                canonicalize(key);
                canonicalize(value);
            }
            entries.sort_by(|(a, _), (b, _)| {
                CanonicalValue::from(a.clone()).cmp(&CanonicalValue::from(b.clone()))
            });
        }
        Value::Array(values) => values.iter_mut().for_each(canonicalize),
        Value::Tag(_, value) => canonicalize(value),
        _ => {}
    }
}

fn json_value(value: &Json, markers: bool) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(value) => {
            if let Some(value) = value.as_i64() {
                Value::Integer(value.into())
            } else if let Some(value) = value.as_u64() {
                Value::Integer(value.into())
            } else {
                Value::Float(value.as_f64().unwrap())
            }
        }
        Json::String(value) => Value::Text(value.clone()),
        Json::Array(values) => {
            Value::Array(values.iter().map(|v| json_value(v, markers)).collect())
        }
        Json::Object(values) if markers && values.len() == 1 && values.contains_key("$bytes") => {
            Value::Bytes(hex(values["$bytes"].as_str().unwrap()))
        }
        Json::Object(values) if markers && values.len() == 1 && values.contains_key("$link") => {
            let cid: crate::format::DaslCid = values["$link"].as_str().unwrap().parse().unwrap();
            link(cid)
        }
        Json::Object(values) => Value::Map(
            values
                .iter()
                .map(|(k, v)| (Value::Text(k.clone()), json_value(v, markers)))
                .collect(),
        ),
    }
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors(text: &str) -> Json {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn independent_cbor_oracle_matches_every_complete_record_vector() {
        let mut count = 0;
        for source in [
            include_str!("../../../fixtures/format/memory.json"),
            include_str!("../../../fixtures/format/limits.json"),
            include_str!("../../../fixtures/format/definition.json"),
            include_str!("../../../fixtures/format/command.json"),
            include_str!("../../../fixtures/format/input.json"),
            include_str!("../../../fixtures/format/boundary.json"),
            include_str!("../../../fixtures/format/episode.json"),
            include_str!("../../../fixtures/format/state.json"),
        ] {
            for vector in vectors(source).as_array().unwrap() {
                assert_eq!(
                    fixture_bytes(&vector["record"]),
                    hex(vector["hex"].as_str().unwrap()),
                    "{}",
                    vector["name"]
                );
                count += 1;
            }
        }
        for source in [
            include_str!("../../../fixtures/format/terminal.json"),
            include_str!("../../../fixtures/format/accounting.json"),
        ] {
            for vectors in vectors(source).as_object().unwrap().values() {
                for vector in vectors.as_array().unwrap() {
                    assert_eq!(
                        fixture_bytes(&vector["record"]),
                        hex(vector["hex"].as_str().unwrap()),
                        "{}",
                        vector["name"]
                    );
                    count += 1;
                }
            }
        }
        assert_eq!(count, 38);
    }

    #[test]
    fn canonical_text_key_order_uses_lengths_and_preserves_duplicate_witnesses() {
        let value = Value::Map(alloc::vec![
            (Value::Text("aa".into()), Value::Integer(1.into())),
            (Value::Text("b".into()), Value::Integer(2.into()))
        ]);
        assert_eq!(encode(&value), hex("a261620262616101"));
        let duplicate = Value::Map(alloc::vec![
            (Value::Text("a".into()), Value::Integer(0.into())),
            (Value::Text("a".into()), Value::Integer(1.into()))
        ]);
        assert_eq!(encode(&duplicate), hex("a2616100616101"));
    }

    #[test]
    fn fixture_markers_do_not_change_ordinary_model_json_maps() {
        let bytes = serde_json::json!({"$bytes":"00"});
        assert_eq!(fixture_bytes(&bytes), hex("4100"));
        assert_ne!(json_bytes(&bytes), fixture_bytes(&bytes));
        let link = serde_json::json!({"$link":"bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"});
        let ordinary = json_bytes(&link);
        assert_eq!(ordinary[0], 0xa1);
        assert_eq!(&fixture_bytes(&link)[..5], &[0xd8, 0x2a, 0x58, 0x25, 0]);
        let nested = serde_json::json!({"a":[{"$bytes":"00"}]});
        assert_ne!(json_bytes(&nested), fixture_bytes(&nested));
    }

    #[test]
    fn fixture_map_mutation_retains_null_keys_and_borrows_nested_values() {
        let mut value =
            fixture_value(&serde_json::json!({"known": null, "nested": {"keep": true}}));
        assert_eq!(map_mut(&mut value).keys().count(), 2);
        assert_eq!(map_mut(&mut value).remove("absent"), None);
        assert_eq!(map_mut(&mut value).remove("known"), Some(Value::Null));
        assert_eq!(
            map_mut(&mut value).insert("known".into(), Value::Integer(0.into())),
            None
        );
        assert_eq!(
            map_mut(&mut value).insert("known".into(), Value::Null),
            Some(Value::Integer(0.into()))
        );
        assert!(map_mut(&mut value).get_mut("absent").is_none());
        let nested = map_mut(&mut value).get_mut("nested").unwrap();
        assert_eq!(
            map_mut(nested).insert("keep".into(), Value::Bool(false)),
            Some(Value::Bool(true))
        );
        assert_eq!(
            encode(&value),
            json_bytes(&serde_json::json!({"known": null, "nested": {"keep": false}}))
        );
    }

    #[test]
    fn runtime_u64_width_is_preserved_independently_of_model_integer_rules() {
        for value in [
            0,
            23,
            24,
            255,
            256,
            65_535,
            65_536,
            u32::MAX as u64,
            1 << 32,
            1 << 53,
            u64::MAX,
        ] {
            let encoded = json_bytes(&serde_json::json!(value));
            let decoded: Value = ciborium::de::from_reader(encoded.as_slice()).unwrap();
            assert_eq!(decoded, Value::Integer(value.into()));
        }
        assert_eq!(
            json_bytes(&serde_json::json!(u64::MAX)),
            hex("1bffffffffffffffff")
        );
    }
}
