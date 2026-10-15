//! Self-consistency of every checked-in format fixture under the independent Ciborium
//! oracle: exact canonical bytes, matching records, hashes and CIDs, one block per CID,
//! and closure byte totals. The vectors themselves are frozen golden data.

use crate::format::{oracle, DaslCid, DaslCodec};
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

macro_rules! documents {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_str!(concat!("../../fixtures/format/", $name, ".json")))),*]
    };
}

const DOCUMENTS: [(&str, &str); 17] = documents!(
    "accounting",
    "boundary",
    "closure",
    "command",
    "definition",
    "demo_scenario",
    "episode",
    "genesis",
    "identity",
    "identity_graphs",
    "input",
    "lifecycle",
    "limits",
    "memory",
    "roles",
    "state",
    "terminal",
);

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// Depth-first visit of every JSON object in one document.
fn objects(document: &Value, mut visit: impl FnMut(&serde_json::Map<String, Value>)) {
    let mut stack = vec![document];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                visit(map);
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
}

#[test]
fn every_format_fixture_is_canonical_self_consistent_and_one_block_per_cid() {
    let documents: Vec<(&str, Value)> = DOCUMENTS
        .iter()
        .map(|(name, text)| (*name, serde_json::from_str(text).unwrap()))
        .collect();
    let mut blocks = BTreeMap::<String, Vec<u8>>::new();
    let mut vectors = 0;
    for (name, document) in &documents {
        objects(document, |object| {
            let Some(Value::String(encoded)) = object.get("hex") else {
                return;
            };
            if !["cid", "record", "value", "sha256"]
                .iter()
                .any(|key| object.contains_key(*key))
            {
                return; // Bare preimages and negative bytes are checked by their owning tests.
            }
            let raw = bytes(encoded);
            let expected = match object.get("record").or_else(|| object.get("value")) {
                Some(record) => oracle::fixture_bytes(record),
                None => oracle::encode(&ciborium::de::from_reader(raw.as_slice()).unwrap()),
            };
            assert_eq!(expected, raw, "{name}: bytes are not the canonical record");
            if let Some(Value::String(digest)) = object.get("sha256") {
                assert_eq!(Sha256::digest(&raw)[..], bytes(digest), "{name}: sha256");
            }
            if let Some(Value::String(cid)) = object.get("cid") {
                assert_eq!(
                    &DaslCid::for_block(DaslCodec::Drisl, &raw).to_string(),
                    cid,
                    "{name}: cid"
                );
                let prior = blocks.entry(cid.clone()).or_insert_with(|| raw.clone());
                assert_eq!(prior, &raw, "{name}: one CID names two blocks");
            }
            vectors += 1;
        });
    }
    let mut closures = 0;
    for (name, document) in &documents {
        objects(document, |object| {
            let total = object.get("closure_bytes").and_then(Value::as_u64);
            if let Some(Value::Array(listed)) = object.get("blocks") {
                let sum: usize = listed
                    .iter()
                    .map(|block| block["hex"].as_str().unwrap().len() / 2)
                    .sum();
                assert_eq!(total, Some(sum as u64), "{name}: block closure total");
                closures += 1;
            }
            if let Some(Value::Array(required)) = object.get("required_cids") {
                let cids: Vec<&str> = required.iter().map(|c| c.as_str().unwrap()).collect();
                assert!(
                    cids.windows(2).all(|w| w[0] < w[1]),
                    "{name}: unsorted CIDs"
                );
                let sum: usize = cids.iter().map(|cid| blocks[*cid].len()).sum();
                assert_eq!(total, Some(sum as u64), "{name}: required closure total");
                closures += 1;
            }
        });
    }
    assert_eq!((vectors, closures), (167, 21), "fixture corpus changed");
}
