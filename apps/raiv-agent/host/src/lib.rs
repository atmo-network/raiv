//! Native Wasmi transport and local administrative ports; agent decisions remain guest-owned.
use core::fmt;
pub mod acceptance;
pub mod admission;
pub mod candidate;
pub mod checkpoint;
#[cfg(target_os = "linux")]
pub mod disk;
pub mod inference;
#[cfg(target_os = "linux")]
pub mod launch;
pub mod output;
#[cfg(target_os = "linux")]
pub mod policy;
#[cfg(target_os = "linux")]
pub mod recovery;
pub mod registry;
pub mod run;
#[cfg(target_os = "linux")]
pub mod simulator;
pub mod store;
#[cfg(target_os = "linux")]
pub mod terminal;
use wasmi::{
    Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};
pub const MAX_FRAME_BYTES: usize = 8_388_608;
#[derive(Clone, Copy, Debug)]
pub struct VmLimits {
    pub memory_bytes: usize,
    pub fuel_per_step: u64,
}
impl VmLimits {
    pub(crate) fn validate(self) -> Result<(), BridgeError> {
        if self.memory_bytes == 0
            || self.memory_bytes > 256 * 1024 * 1024
            || self.fuel_per_step == 0
        {
            return Err(BridgeError::InvalidLimits);
        }
        Ok(())
    }
}
impl Default for VmLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            fuel_per_step: 50_000_000,
        }
    }
}
#[derive(Debug)]
pub enum BridgeError {
    InvalidLimits,
    ForbiddenImports,
    FrameLimit,
    Allocation,
    InvalidRange,
    Dead,
    Trap(wasmi::Error),
    Output(output::DecodeError),
}
impl From<wasmi::Error> for BridgeError {
    fn from(error: wasmi::Error) -> Self {
        Self::Trap(error)
    }
}
impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => f.write_str("invalid finite VM limits"),
            Self::ForbiddenImports => f.write_str("guest imports are forbidden"),
            Self::FrameLimit => f.write_str("ABI frame/module byte limit"),
            Self::Allocation => f.write_str("guest or host allocation failed"),
            Self::InvalidRange => f.write_str("invalid or overlapping ABI buffer range"),
            Self::Dead => f.write_str("guest instance failed and must be discarded"),
            Self::Trap(error) => error.fmt(f),
            Self::Output(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for BridgeError {}
/// Import-free finite-memory/fuel VM. This is isolation/transport, not module approval,
/// canonical acceptance, a dispatch permit or proof against the trusted host itself.
pub struct GuestVm {
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    dealloc: TypedFunc<(u32, u32), ()>,
    step: TypedFunc<(u32, u32), u64>,
    limits: VmLimits,
    dead: bool,
}
impl GuestVm {
    pub fn new(wasm: &[u8], limits: VmLimits) -> Result<Self, BridgeError> {
        if wasm.len() > MAX_FRAME_BYTES {
            return Err(BridgeError::FrameLimit);
        }
        limits.validate()?;
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm)?;
        if module.imports().next().is_some() {
            return Err(BridgeError::ForbiddenImports);
        }
        let resource = StoreLimitsBuilder::new()
            .memory_size(limits.memory_bytes)
            .instances(1)
            .memories(1)
            .tables(1)
            .table_elements(65_536)
            .build();
        let mut store = Store::new(&engine, resource);
        store.limiter(|limits| limits);
        store.set_fuel(limits.fuel_per_step)?;
        let instance = Linker::new(&engine).instantiate_and_start(&mut store, &module)?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or(BridgeError::InvalidRange)?;
        let alloc = instance.get_typed_func::<u32, u32>(&store, "raiv_alloc")?;
        let dealloc = instance.get_typed_func::<(u32, u32), ()>(&store, "raiv_dealloc")?;
        let step = instance.get_typed_func::<(u32, u32), u64>(&store, "raiv_step")?;
        Ok(Self {
            store,
            memory,
            alloc,
            dealloc,
            step,
            limits,
            dead: false,
        })
    }
    /// One complete frame; copy guest-owned output before any later step and free only
    /// the exact input allocation. Closed typed output preflight precedes return.
    /// Trap/zero/invalid range or invalid output schema/integrity discards the instance.
    pub fn round_trip(&mut self, bytes: &[u8]) -> Result<Vec<u8>, BridgeError> {
        if self.dead {
            return Err(BridgeError::Dead);
        }
        if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
            return Err(BridgeError::FrameLimit);
        }
        let result = self.exchange(bytes);
        if result.is_err() {
            self.dead = true;
        }
        result
    }
    fn exchange(&mut self, bytes: &[u8]) -> Result<Vec<u8>, BridgeError> {
        self.store.set_fuel(self.limits.fuel_per_step)?;
        let len = u32::try_from(bytes.len()).map_err(|_| BridgeError::FrameLimit)?;
        let pointer = self.alloc.call(&mut self.store, len)?;
        if pointer == 0 {
            return Err(BridgeError::Allocation);
        }
        let input = checked_range(pointer, len, self.memory.data_size(&self.store))?;
        self.memory
            .write(&mut self.store, input.start, bytes)
            .map_err(|_| BridgeError::InvalidRange)?;
        let packed = self.step.call(&mut self.store, (pointer, len))?;
        if packed == 0 {
            return Err(BridgeError::InvalidRange);
        }
        let out_ptr = (packed >> 32) as u32;
        let out_len = packed as u32;
        if out_len == 0 || out_len as usize > MAX_FRAME_BYTES {
            return Err(BridgeError::FrameLimit);
        }
        let output = checked_range(out_ptr, out_len, self.memory.data_size(&self.store))?;
        if input.start < output.end && output.start < input.end {
            return Err(BridgeError::InvalidRange);
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(out_len as usize)
            .map_err(|_| BridgeError::Allocation)?;
        result.resize(out_len as usize, 0);
        self.memory
            .read(&self.store, output.start, &mut result)
            .map_err(|_| BridgeError::InvalidRange)?;
        self.dealloc.call(&mut self.store, (pointer, len))?;
        output::Output::decode(&result, &raiv::limits::Limits::default())
            .map_err(BridgeError::Output)?;
        Ok(result)
    }
}
fn checked_range(
    ptr: u32,
    len: u32,
    memory: usize,
) -> Result<core::ops::Range<usize>, BridgeError> {
    if ptr == 0 {
        return Err(BridgeError::InvalidRange);
    }
    let end = ptr.checked_add(len).ok_or(BridgeError::InvalidRange)? as usize;
    if end > memory {
        return Err(BridgeError::InvalidRange);
    }
    Ok(ptr as usize..end)
}
#[cfg(test)]
mod tests;
