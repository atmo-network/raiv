//! Private duplicate/resource preflight before serde_json can own a lossy map.
use super::NormalizeError;
use raiv::format::DecodeLimits;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::{borrow::Cow, fmt};
pub(super) fn parse(
    bytes: &[u8],
    limits: &DecodeLimits,
) -> Result<serde_json::Value, NormalizeError> {
    if bytes.len() as u64 > limits.max_block_bytes || limits.max_depth == 0 || limits.max_depth > 64
    {
        return Err(NormalizeError::Limit);
    }
    let mut budget = Budget { limits, nodes: 0 };
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    Check {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(&mut decoder)
    .map_err(|_| NormalizeError::Json)?;
    decoder.end().map_err(|_| NormalizeError::Json)?;
    // Entire input has passed duplicates, UTF-8, syntax, depth, per-container and aggregate bounds.
    serde_json::from_slice(bytes).map_err(|_| NormalizeError::Json)
}
struct Budget<'a> {
    limits: &'a DecodeLimits,
    nodes: u64,
}
impl Budget<'_> {
    fn node<E: de::Error>(&mut self) -> Result<(), E> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| E::custom("entry bound"))?;
        if self.nodes > self.limits.max_collection_entries {
            return Err(E::custom("entry bound"));
        }
        Ok(())
    }
}
struct Check<'a, 'b> {
    budget: &'a mut Budget<'b>,
    depth: u64,
}
impl<'de> DeserializeSeed<'de> for Check<'_, '_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        if self.depth > self.budget.limits.max_depth {
            return Err(de::Error::custom("depth bound"));
        }
        self.budget.node()?;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Check<'_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON DATA")
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq
            .next_element_seed(Check {
                budget: self.budget,
                depth: self.depth + 1,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key_seed(Key)? {
            self.budget.node()?;
            if self.depth + 1 > self.budget.limits.max_depth
                || keys.iter().any(|prior| prior == &key)
            {
                return Err(de::Error::custom("key/depth bound"));
            }
            keys.try_reserve(1)
                .map_err(|_| de::Error::custom("allocation"))?;
            keys.push(key);
            map.next_value_seed(Check {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
        }
        Ok(())
    }
}
struct Key;
impl<'de> DeserializeSeed<'de> for Key {
    type Value = Cow<'de, str>;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key {
    type Value = Cow<'de, str>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON key")
    }
    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Cow::Borrowed(value))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let mut text = String::new();
        text.try_reserve_exact(value.len())
            .map_err(|_| E::custom("allocation"))?;
        text.push_str(value);
        Ok(Cow::Owned(text))
    }
}
