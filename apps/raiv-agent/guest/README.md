# Rust guest

Package `raiv-agent-guest` is the isolated agent: pure `Guest::step` handling of the step ABI, compiled into an import-free `wasm32v1-none` module exporting `memory`, `raiv_alloc`, `raiv_dealloc` and `raiv_step`. It owns context, tools, decisions and state transitions. Its only runtime dependencies are Core, minicbor and, on wasm, dlmalloc, all with defaults disabled: no std, imports, I/O, randomness, clocks, threads or host callbacks.

- [Build, runtime checks and ABI](../README.md)
- [Native host](../host/README.md)
- [Core package](../../../lib/README.md)

## Profiles

Each constructor returns the exact Definition for a new, explicitly approved registration. It grants no approval itself, and the host uses it only for metadata; activation always runs in the isolated module.

| Constructor | Inference profile | Tools |
| --- | --- | --- |
| `scripted_definition(profile)` | Offline fixture model | Veto: `veto`, `pass`; Survivor: `patch_memory`, `trade`, `set_wake` |
| `opencode_definition(profile, base_url, model, mandate)` | `opencode-go`, thinking disabled | Same compiled tools as the scripted profiles |
| `opencode_reasoning_definition(…, ReasoningEffort::Low \| High)` | `opencode-go`, thinking enabled for `deepseek-v4.1-flash` | Same; the native host alone keeps same-run reasoning |
| `openai_compatible_definition(profile, base_url, model, mandate)` | `openai-compatible`, no provider extensions | Survivor's `patch_memory` uses the `anyOf` plane schema |

Existing profiles never change their bytes, because sessions pin them; new behavior arrives as a new profile. No profile registers a `no_op` or model-facing Finish tool: a complete reply without calls selects intrinsic Finish, local calls Continue and external calls Yield. Quotes are synthetic units, not billing or authority.

## Runtime

`runtime.rs` keeps one live `Activation` across inference yields, with its immutable prefix, bounded trajectory and counters. Patch results return the full Memory plus processed/skipped planes and actual change; a rejected patch gets actionable `invalid_patch:` feedback. A sealed Yield or Finish emits exact validated candidate data, never host acceptance. Failures report their category: `corrective_turns_exhausted`, `budget_exhausted`, `context_limit` or `activation_failed`; stale or malformed frames leave the pending request untouched.

The module is single-threaded and synchronous. It owns one live input allocation and one output buffer, compares foreign or stale pointers against ownership without dereferencing them, and keeps output until the next step. Panics trap, and allocation failure is an error, never a successful completion. The library also builds as an rlib, so native and Wasmi runs can be compared byte for byte.
