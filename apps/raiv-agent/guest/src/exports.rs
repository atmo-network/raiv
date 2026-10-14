//! Single-threaded import-free Wasm buffer ownership; never dereferences caller pointers.
use super::{abi, AbiError, Guest, MAX_FRAME_BYTES};
use alloc::vec::Vec;
use core::cell::UnsafeCell;
#[global_allocator]

static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}
struct State {
    guest: Guest,
    input: Option<Vec<u8>>,
    output: Vec<u8>,
}
struct Global(UnsafeCell<State>);
// Wasm MVP has no threads, imports or reentrant callbacks; exports run synchronously.
unsafe impl Sync for Global {}
static STATE: Global = Global(UnsafeCell::new(State {
    guest: Guest::new(),
    input: None,
    output: Vec::new(),
}));
#[no_mangle]
pub extern "C" fn raiv_alloc(len: u32) -> u32 {
    // SAFETY: the module's single-threaded synchronous export boundary is exclusive.
    let state = unsafe { &mut *STATE.0.get() };
    if len == 0 || len as usize > MAX_FRAME_BYTES || state.input.is_some() {
        return 0;
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(len as usize).is_err() {
        return 0;
    }
    bytes.resize(len as usize, 0);
    let pointer = bytes.as_mut_ptr() as u32;
    state.input = Some(bytes);
    pointer
}
#[no_mangle]
pub extern "C" fn raiv_dealloc(ptr: u32, len: u32) {
    // SAFETY: exclusive export access; only an exact live allocation can be freed.
    let state = unsafe { &mut *STATE.0.get() };
    if state
        .input
        .as_ref()
        .is_some_and(|bytes| bytes.as_ptr() as u32 == ptr && bytes.len() == len as usize)
    {
        state.input = None;
    }
}
#[no_mangle]
pub extern "C" fn raiv_step(ptr: u32, len: u32) -> u64 {
    // SAFETY: exclusive access. Foreign offsets are compared, never dereferenced.
    let state = unsafe { &mut *STATE.0.get() };
    state.output = Vec::new(); // Previous output expires at this step, never at alloc/free.
    let result = match state.input.as_ref() {
        Some(bytes) if bytes.as_ptr() as u32 == ptr && bytes.len() == len as usize => {
            state.guest.step(bytes)
        }
        _ => abi::error(AbiError::InvalidFrame),
    };
    let Ok(output) = result else { return 0 };
    if output.is_empty() || output.len() > MAX_FRAME_BYTES {
        return 0;
    }
    state.output = output;
    ((state.output.as_ptr() as u64) << 32) | state.output.len() as u64
}
