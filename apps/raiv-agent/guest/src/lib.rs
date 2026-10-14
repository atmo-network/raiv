//! Pure no_std compiled demo Activation/reply/candidate runtime; no host acceptance.
#![no_std]
extern crate alloc;
mod abi;
#[cfg(target_arch = "wasm32")]
mod exports;
mod profiles;
mod runtime;
pub use abi::{AbiError, MAX_FRAME_BYTES};
use alloc::vec::Vec;
pub use profiles::{
    opencode_definition, opencode_reasoning_definition, scripted_definition, ChatProfile,
    ReasoningEffort,
};
use raiv::{definition::Definition, graph::Genesis, limits::Limits, memory::Memory};

/// Shared native/WASM guest logic, with no external capabilities or host decisions.
#[derive(Default)]
pub struct Guest {
    initialized: bool,
    live: Option<raiv::activation::Activation<'static>>,
    sequence: Option<raiv::activation::RequestSequence>,
}
impl Guest {
    pub const fn new() -> Self {
        Self {
            initialized: false,
            live: None,
            sequence: None,
        }
    }
    /// Rejected outer ABI frames and stale correlations preserve the pending Activation.
    /// Matching replies advance registered RAM work; fatal reply/inference errors
    /// terminate without a candidate, while successful sealing emits boundary DATA.
    /// Failure to allocate output is fatal and yields no successful response.
    pub fn step(&mut self, bytes: &[u8]) -> Result<Vec<u8>, AbiError> {
        match self.advance(bytes) {
            Ok(output) => Ok(output),
            Err(error) => abi::error(error),
        }
    }
    fn initialize(&mut self, input: abi::Initialize<'_>) -> Result<Vec<u8>, AbiError> {
        if self.initialized || self.live.is_some() {
            return Err(AbiError::AlreadyInitialized);
        }
        let limits = Limits::default();
        let definition = Definition::decode(input.definition, &limits.decode_limits())
            .map_err(|_| AbiError::InvalidState)?;
        let memory = input
            .memory
            .map(|bytes| {
                Memory::decode(
                    bytes,
                    &raiv::format::DecodeLimits {
                        max_block_bytes: limits.max_memory_bytes,
                        ..limits.decode_limits()
                    },
                )
            })
            .transpose()
            .map_err(|_| AbiError::InvalidState)?;
        let genesis = Genesis::build(input.agent_id, definition, memory, &limits)
            .map_err(|_| AbiError::InvalidState)?;
        let output = abi::initialized(&genesis)?;
        self.initialized = true;
        Ok(output)
    }
}
