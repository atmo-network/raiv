//! Deterministic mutation fuzzing of every public Core decoder and the patch path.
//! Seeds are the golden fixture vectors; mutations must never panic, and anything a
//! decoder accepts must be exactly the canonical bytes it was given.

use crate::{
    activation::ModelReply,
    boundary::Boundary,
    budget::BudgetAdmission,
    command::{Command, CommandView},
    definition::Definition,
    episode::Episode,
    format::{validate_drisl, DaslCid, DecodeLimits, ModelValue, ValueRef},
    input::ActivationInput,
    limits::Limits,
    memory::{Memory, PatchArguments},
    position::Position,
    receipt::{Receipt, ReceiptView},
    state::AgentState,
    usage::Usage,
};
use alloc::{string::ToString, vec, vec::Vec};
use serde_json::Value;

const FIXTURES: [&str; 10] = [
    include_str!("../../fixtures/format/boundary.json"),
    include_str!("../../fixtures/format/command.json"),
    include_str!("../../fixtures/format/definition.json"),
    include_str!("../../fixtures/format/episode.json"),
    include_str!("../../fixtures/format/genesis.json"),
    include_str!("../../fixtures/format/input.json"),
    include_str!("../../fixtures/format/memory.json"),
    include_str!("../../fixtures/format/state.json"),
    include_str!("../../fixtures/format/terminal.json"),
    include_str!("../../fixtures/format/lifecycle.json"),
];

fn seeds() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for text in FIXTURES {
        let mut stack = vec![serde_json::from_str::<Value>(text).unwrap()];
        while let Some(value) = stack.pop() {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(hex)) = map.get("hex") {
                        out.push(hex_bytes(hex));
                    }
                    stack.extend(map.into_iter().map(|(_, v)| v));
                }
                Value::Array(items) => stack.extend(items),
                _ => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// xorshift64*: reproducible without a dependency; the seed is fixed so failures replay.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut Rng, seed: &[u8], donor: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for _ in 0..1 + rng.below(3) {
        let at = rng.below(bytes.len() + 1);
        match rng.below(7) {
            0 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes[i] ^= 1 << rng.below(8);
            }
            1 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                // Interesting CBOR heads: lengths, indefinite markers, tags, floats, null.
                bytes[i] = [
                    0x00, 0x17, 0x18, 0x1f, 0x5f, 0x7f, 0x9f, 0xbf, 0xd8, 0xf6, 0xfb, 0xff,
                ][rng.below(12)];
            }
            2 => bytes.truncate(at),
            3 => bytes.insert(at, rng.next() as u8),
            4 if at < bytes.len() => {
                bytes.remove(at);
            }
            5 if !donor.is_empty() => {
                let from = rng.below(donor.len());
                let len = rng.below(donor.len() - from + 1).min(64);
                let part = donor[from..from + len].to_vec();
                bytes.splice(at..at, part);
            }
            _ => {
                let end = (at + 1 + rng.below(8)).min(bytes.len());
                let part = bytes[at.min(end)..end].to_vec();
                bytes.splice(at..at, part);
            }
        }
    }
    bytes
}

fn exercise(bytes: &[u8], memories: &mut [Memory]) {
    let limits = Limits::default();
    let bounds = limits.decode_limits();
    let _ = validate_drisl(bytes, &bounds);
    let _ = DaslCid::from_bytes(bytes);
    let _ = Position::decode(bytes, &bounds);
    let _ = BudgetAdmission::decode(bytes, &bounds);
    let _ = Usage::decode(bytes, &bounds);
    let _ = Limits::decode(bytes, &bounds);
    let _ = CommandView::decode(bytes, &limits);
    let _ = ReceiptView::decode(bytes, &limits);
    let _ = PatchArguments::decode(bytes, &limits);
    let _ = ModelReply::decode(bytes, &limits).map(|reply| reply.control_call().is_ok());
    if let Ok(value) = ModelValue::decode(bytes, &bounds) {
        assert_eq!(value.as_bytes(), bytes);
        let json = value.json().to_string();
        serde_json::from_str::<Value>(&json).expect("rendered model value is JSON");
    }
    macro_rules! exact {
        ($($decoded:expr),* $(,)?) => {$(
            if let Ok(record) = $decoded {
                assert_eq!(record.as_bytes(), bytes);
            }
        )*};
    }
    exact!(
        Definition::decode(bytes, &bounds),
        Memory::decode(bytes, &bounds),
        ActivationInput::decode(bytes, &limits),
        Boundary::decode(bytes, &limits),
        Episode::decode(bytes, &bounds),
        AgentState::decode(bytes, &bounds),
        Command::decode(bytes, &limits),
        Receipt::decode(bytes, &limits),
    );
    for memory in memories.iter_mut() {
        let before = memory.as_bytes().to_vec();
        match memory.apply_patch(bytes, &limits) {
            Ok(_) => {
                Memory::decode(memory.as_bytes(), &bounds).expect("patched Memory stays valid");
            }
            Err(_) => assert_eq!(memory.as_bytes(), before, "failed patch mutated Memory"),
        }
    }
}

#[test]
fn mutated_golden_vectors_never_panic_and_accepted_bytes_are_canonical() {
    let seeds = seeds();
    assert!(seeds.len() >= 40, "fixture seeds missing: {}", seeds.len());
    let bounds = DecodeLimits::default();
    let note = [("note", ValueRef::Text("keep"))];
    let steps = [("steps", ValueRef::List(&[ValueRef::Integer(1)]))];
    let mut memories = [
        Memory::encode(&[], &bounds).unwrap(),
        Memory::encode(
            &[
                ("intents", ValueRef::Map(&note)),
                ("tactics", ValueRef::Map(&steps)),
            ],
            &bounds,
        )
        .unwrap(),
    ];
    let mut rng = Rng(0x5241_4956_2026_1015);
    for seed in &seeds {
        exercise(seed, &mut memories);
    }
    for round in 0..500 {
        for (index, seed) in seeds.iter().enumerate() {
            let donor = &seeds[(index + round + 1) % seeds.len()];
            exercise(&mutate(&mut rng, seed, donor), &mut memories);
        }
    }
}
