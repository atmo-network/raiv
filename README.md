# raiv

Raiv keeps an AI agent's behavior continuous across short, separate executions. Instead of replaying a transcript, each accepted step becomes a small checkpoint: canonical DRISL/CBOR records linked by content identifiers (CIDs), holding the agent's Definition, bounded sparse Memory, history and receipts. Identical state always reproduces identical bytes and identifiers, so any later execution can restore exactly where the agent left off.

Raiv 0.0.2 is a working MVP, not production, financial or blockchain readiness.

## How it fits together

- **Core** ([`lib/`](lib/README.md), crate [`raiv`](https://crates.io/crates/raiv)): portable `no_std + alloc` Rust. It validates records under explicit limits, restores checkpoints, projects bounded model context, runs the in-RAM Activation loop and composes verified commit candidates. No I/O and no execution authority.
- **Guest** ([`apps/raiv-agent/guest`](apps/raiv-agent/guest/README.md)): an isolated `wasm32v1-none` module that owns the agent's decisions, context, tools and state transitions.
- **Host** ([`apps/raiv-agent/host`](apps/raiv-agent/host/README.md)): a native Linux bridge on Wasmi. It owns everything external: storage, model inference over HTTPS, local acceptance and recovery, and the terminal UI.

## Run the interactive demo (Linux)

With `OPENCODE_API_KEY` already exported, run [`./run.sh`](run.sh) and chat; the agent remembers what matters across launches (`./run.sh --status` shows its Memory). The default model is `deepseek-v4.1-flash`; a new session can use `qwen3.8-flash` through the generic OpenAI-compatible profile with the same key:

```sh
RAIV_PROVIDER=openai-compatible ./run.sh var/raiv/qwen
```

The Bash launcher builds the real WASM/native host, prepares a private local session and asks for explicit source/plan approval before opening the TUI. State stays in `var/raiv/chat`, outside Cargo's `target/`; reopening uses the same accepted Memory, never a restored transcript or automatic retry. See [setup, limits and other launcher commands](apps/raiv-agent/README.md#simple-bash-launcher).

## Use the library

```toml
[dependencies]
raiv = "0.0.2"
```

The [crate documentation](https://docs.rs/raiv) starts with a compiled Memory/patch example; the [library API guide](lib/README.md#api-guide) walks the Core from canonical data to commit candidates.

## Start here

- [Library package and API guide](lib/README.md)
- [Reference guest and native bridge](apps/raiv-agent/README.md)
- [Project constraints](AGENTS.md)
- [Normative specification and documentation](docs/README.md)
- [Remaining implementation work](BACKLOG.md)
- [Completed development and validation outcomes](CHANGELOG.md)
- [Conformance fixtures and provenance](fixtures/README.md)
- [Rust boundary CI definition](.github/workflows/rust-boundaries.yml)

## Current status

**Works today**

- Core (`no_std + alloc`, Rust 1.85+): bounded typed records, sparse object-patch Memory, checkpoint restoration and context projection, the in-RAM Activation loop and verified commit candidates.
- The isolated guest runs inside Wasmi 2.0.0 and produces byte-identical results to its native build.
- The Linux host stores exact blocks on disk, commits boundaries locally with a typed manifest, recovers from pinned checkpoints, and never retries, refunds or replays an uncertain outcome.
- Live chat through `opencode-go` (DeepSeek) or any OpenAI-compatible endpoint. Memory carries across launches without restoring a transcript.
- An offline [retry-and-sleep scenario](apps/raiv-agent/README.md#runnable-retry-and-sleep-demo) exercises the action → failure → retry → Finish → wake cycle against a scripted unit world.

**Not yet**

- External authority and provenance, durable pin selection, real effects or funds, CAR/IPFS transport and provider-loss recovery. See [BACKLOG.md](BACKLOG.md).
- Nothing here is ready for real financial operations or blockchain deployment.

## Development

Root Cargo commands select the library (`lib`, the default workspace member); select application packages with `-p`. Dependencies are exactly pinned with runtime defaults disabled. Host test/dev dependencies do not establish runtime no_std compatibility. To reproduce the MSRV gates without changing the default toolchain:

```sh
rustup toolchain install 1.85.0 --profile minimal --component clippy --component rustfmt --target wasm32-unknown-unknown --target thumbv7em-none-eabi
cargo +1.85.0 test --locked
cargo +1.85.0 clippy --locked --all-targets -- -D warnings
cargo +1.85.0 fmt --check
cargo +1.85.0 check --locked --lib --target wasm32-unknown-unknown
cargo +1.85.0 check --locked --lib --target thumbv7em-none-eabi
```

Current-toolchain gates:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
rustup target add wasm32-unknown-unknown thumbv7em-none-eabi
cargo check --locked --lib --target wasm32-unknown-unknown
cargo check --locked --lib --target thumbv7em-none-eabi
cargo tree --locked --target wasm32-unknown-unknown -e features --edges normal,build
cargo tree --locked --target thumbv7em-none-eabi -e features --edges normal,build
cargo tree --locked --target all --edges normal,build,dev
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p raiv --no-deps
bash .github/scripts/check-links.sh
```

Application gates (guest, host, actual Wasmi and launcher) are listed in the [application entrypoint](apps/raiv-agent/README.md). By owner decision, documentation snapshots have no checksum manifest; their provenance is recorded in the [documentation index](docs/README.md).

The bare-metal target has no std library. Both library target builds and feature inspection establish compile-time portability evidence only—not sandbox execution, allocator integration, native/WASM parity or JAM readiness. The checked-in Rust boundary workflow defines Core 1.85/stable tests/lint/portable checks, exact fixture/dependency gates and host 1.86/stable plus built Rust-guest/Wasmi/isolated-process checks. It uses read-only permissions and a pinned checkout action, with no live credentials or publication. Hosted GitHub execution passed on the published 0.0.1 commit; local command evidence alone is not a hosted run. It also builds rustdoc with warnings denied, checks documentation links and runs the launcher regressions. It does not establish full acceptance coverage, coverage-guided fuzzing or measured RAM/resource evidence, which remain in BACKLOG.md.
