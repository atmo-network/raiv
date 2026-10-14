//! Pure, bounded agent checkpoint primitives.
//!
//! Registered RAM preparation/request/advancement and validated candidate data exist;
//! host persistence/acceptance and guest integration remain incremental.
#![no_std]

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
