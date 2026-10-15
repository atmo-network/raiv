# Project context

Canonical open work: BACKLOG.md

## Meta-Protocol Principles

- `Constraint-Driven Evolution`: Add structure only when real project constraints justify it; do not manufacture conventions to satisfy generic tooling.
- `Single Source of Truth`: The [specification](docs/raiv.spec.en.md) owns requirements, [BACKLOG.md](BACKLOG.md) owns remaining work, and [CHANGELOG.md](CHANGELOG.md) owns completed outcomes.
- `Context Hygiene`: Consolidate stale or duplicated context rather than accumulating delivery narration in entrypoints.
- `Boundary Clarity`: Keep durable rules, open work, delivery history and design contracts distinct. Protocol-data restoration and pure RAM acceptance are not runnable Activation, authorized host acceptance or dispatch.

## Concept

Raiv preserves an agent's behavioral continuity across short executions through bounded persistent memory and reproducible checkpoints. Its portable Core and isolated guest own agent behavior; the native host owns external I/O and trusted acceptance.

## Topology

- `Cargo.toml`: Root workspace with `lib` as its default member; shared Cargo.lock and target directory stay at the root.
- `lib/`: The raiv Cargo package; `lib/src/` owns the no_std library. Typed records and behavior belong to their domain owners, while codec mechanics remain private to `format`.
- `apps/raiv-agent/{guest,host}/`: Isolated no_std guest and native std Wasmi bridge; application dependencies stay outside Core's graph.
- `docs/`: Normative specification, external specification snapshots and their documentation/provenance index.
- `fixtures/`: Conformance vectors, the Raiv profile overlay and upstream license/attribution; provenance is documented in `fixtures/README.md`.

## Durable Conventions

### Compatibility and dependency boundaries

- `MSRV`: Minimum supported Rust is 1.85, declared in lib/Cargo.toml. Keep language/library APIs and locked dependencies compatible; validate Core/guest on Rust 1.85.0 and the current toolchain. The Wasmi 2.0.0 native host separately requires and is validated on Rust 1.86.0; never raise Core's MSRV to match it.
- `Portable Core`: The entire raiv library and its runtime dependency graph MUST support no_std + alloc. Use core and alloc; no filesystem, environment, networking, clocks, threads or std-dependent runtime inside the library.
- `Dependency features`: Disable dependency defaults unless their no_std feature graph is explicitly verified. Validate library changes on thumbv7em-none-eabi and wasm32-unknown-unknown and inspect normal/build features. Std-enabled test/dev dependencies are host-side evidence only and must not leak into the runtime graph; host tests alone do not prove no_std compatibility.

### Architecture and wire contract

- `Guest/host boundary`: Only the raiv-agent native host may use Rust std. Core and the isolated Rust wasm32v1-none guest remain no_std + alloc. Wasmi 2.x without WASI, HTTPS/JSON/SSE, filesystem/config/environment access, Ratatui/Crossterm and simulated external effects belong to the host. The guest owns decisions, context, tools and state transitions. Introduce host dependencies only at the application milestone, never into Core's graph. Guest integration must prove the bounded versioned CBOR ABI, retained activation across inference and native/WASM equality.
- `Typed ownership`: Keep codec implementations private to format and typed schemas with their domain owners. Retain minicbor, fixed-profile CID and typed records; do not introduce a universal runtime IPLD Value/registry. No IPLD/CID dependency crates may appear in normal/build/dev graphs. Tests use the independent Ciborium oracle.
- `Pinned sessions`: A local session keeps its own guest module, configuration and grant plan; newer builds never rebind or widen it. Guest profile changes must be additive: never change the bytes of an existing profile's Definition (policy, tool descriptors, inference profile) while the host derives Definitions from compiled profiles; add a new profile instead. Verify with the fixture generators' `--check` and the profile byte tests.
- `Item spacing`: Separate Rust items with one blank line at module, `impl` and `trait` level (functions, types, constants, `impl` blocks, modules with bodies); keep consecutive `use` statements and `mod name;` declarations grouped, and put a blank line after `//!` module docs. rustfmt keeps but never inserts these lines, so preserve them when editing.
- `Tooling languages`: Rust for code and tests, Bash for scripts; no Python. Fixture vectors are frozen golden data: new vectors are assembled with the independent test-only Ciborium oracle and checked by `fixture_tests`, never produced from Raiv's own encoder output.
- `Wire preservation`: Private codec simplification must preserve canonical bytes, strict bounds, validation-before-ownership and model/runtime separation. A package/document version bump alone must not change wire formats, hash domains or golden fixtures. Any copied n0-computer/dasl code must retain its upstream license/attribution.

### Context and change discipline

- `Documentation ownership`: Necessary concise specification updates are permitted; implementation progress belongs in CHANGELOG.md, not the normative spec. README entrypoints describe current APIs, support boundaries, setup and navigation, not per-slice test/delivery diaries. Fixture provenance belongs in fixtures/README.md; detailed completed evidence belongs in CHANGELOG.md.
- `Stable references`: In README entrypoints and active project-context prose, avoid bare numbered specification-section citations. Name the concept and omit redundant references when the specification is already linked. When a precise reference is necessary, use a Markdown link to a verified heading anchor. Stable acceptance IDs remain available for traceability; normative cross-references and historical delivery records need not be rewritten.
- `Worktree preservation`: Preserve the existing specification relocation and unrelated changes.

## Validation

Run the applicable library gates after code changes; context-only changes require documentation/link/structure validation (`bash .github/scripts/check-links.sh`), not repetition of unchanged Rust evidence.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cargo check --locked --lib --target wasm32-unknown-unknown
cargo check --locked --lib --target thumbv7em-none-eabi
cargo tree --locked --target wasm32-unknown-unknown -e features --edges normal,build
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p raiv --no-deps
bash .github/scripts/check-links.sh
cargo +1.85.0 test --locked
cargo +1.85.0 clippy --locked --all-targets -- -D warnings
cargo +1.85.0 fmt --check
cargo +1.85.0 check --locked --lib --target wasm32-unknown-unknown
cargo +1.85.0 check --locked --lib --target thumbv7em-none-eabi
```

Application build/runtime/MSRV commands live in apps/raiv-agent/README.md; ordinary Core tests do not execute the ignored Wasmi runtime tests. After application changes also run `sh apps/raiv-agent/test-host.sh` (with `RAIV_GUEST_WASM`) and `bash apps/raiv-agent/test-run.sh --native`. Live provider checks spend real credentials and are run only when the owner asks.

A wasm32 compile check is not sandbox execution or JAM readiness. Missing or blocked validation remains unproven; do not bypass an owner-gated policy decision to manufacture a passing result.
