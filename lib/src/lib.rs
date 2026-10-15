//! Pure, bounded agent checkpoint primitives.
//!
//! Raiv preserves an agent's behavioral continuity across short executions.
//! Each accepted step becomes a small, content-addressed checkpoint: canonical
//! DRISL/CBOR records linked by fixed-profile CIDs, so identical state always
//! reproduces identical bytes and identifiers.
//!
//! This crate is the portable `no_std + alloc` Core. It decodes and validates
//! records under explicit [`limits`], restores checkpoint closures
//! ([`graph`]), projects bounded model context ([`projection`]), runs the RAM
//! Activation loop ([`activation`]) and composes verified commit candidates
//! ([`commit`]). It performs no I/O and grants no execution authority: storage,
//! inference transport and trusted acceptance belong to the host.
//!
//! Working [`memory`] is a sparse object edited by JSON-merge-style patches:
//!
//! ```
//! use raiv::{
//!     format::{DecodeLimits, ModelValue, ValueRef},
//!     limits::Limits,
//!     memory::Memory,
//! };
//!
//! let bounds = DecodeLimits::default();
//! let goal = [("goal", ValueRef::Text("ship 0.0.2"))];
//! let mut memory = Memory::encode(&[("intents", ValueRef::Map(&goal))], &bounds)?;
//! let before = memory.cid();
//!
//! // A model-issued patch, as canonical CBOR: add a plane, keep the rest.
//! let step = [("next", ValueRef::Text("write the changelog"))];
//! let patch = ModelValue::encode(ValueRef::Map(&[("tactics", ValueRef::Map(&step))]), &bounds)?;
//! memory.apply_patch(patch.as_bytes(), &Limits::default())?;
//!
//! let planes = [("intents", ValueRef::Map(&goal)), ("tactics", ValueRef::Map(&step))];
//! assert_eq!(memory, Memory::encode(&planes, &bounds)?);
//! assert_ne!(memory.cid(), before); // Every accepted change has a new, reproducible CID.
//! assert_eq!(
//!     memory.json().to_string(),
//!     r#"{"intents":{"goal":"ship 0.0.2"},"tactics":{"next":"write the changelog"}}"#
//! );
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod activation;
pub mod boundary;
pub mod budget;
pub mod command;
pub mod commit;
pub mod definition;
pub mod episode;
pub mod format;
pub mod graph;
pub mod identity;
pub mod input;
pub mod limits;
pub mod memory;
pub mod position;
pub mod preparation;
pub mod projection;
pub mod receipt;
pub mod state;
pub mod tools;
pub mod usage;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod fuzz_tests;
