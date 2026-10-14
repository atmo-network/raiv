# raiv library

The `raiv` Cargo package owns portable agent checkpoint data, atomic sparse intents/strategy/tactics object patches and borrowed patch-argument preflight, borrowed history/context-data projection Definition-bound callable registration/prepared-packet data checks RAM call-count/instance-request/control-phase/unit-reservation guards, borrowed bounded normalized replies and owning Activation/request/registered RAM advancement and immutable validated CommitCandidate data. Runtime code and dependencies support **no_std + alloc**; minimum Rust is **1.85**. Crate names and imports remain `raiv::…`.

`memory::PatchArguments` selects exact supported roots, borrows processed/skipped names and can encode a bounded full-Memory/change/reason report. The primitive does not remap aliases or persist diagnostics. Activation stages this report into the model-facing tool result/context before assignment; partial/all-skipped requests remain Continue without forced correction. Generated reports use result/context caps, not inbound model-reply caps; trajectory embedding and combined context remain independently bounded before assignment. Low-level `apply` still returns the accepted Memory value. Pinned descriptors, policy and scripted lifecycle cases describe intended roots and tolerant selection without weakening exact registration binding.

A complete correlated normalized reply with zero tool calls selects intrinsic Finish (text may be empty), without a corrective inference or a fabricated tool. Local calls remain Continue and external calls Yield; invalid/multiple/disabled calls, failed inference and budget exhaustion never become completion. Finish stages bounded RAM output then produces the existing atomic candidate/closed Episode; host acceptance remains separate. No-tool Definitions may still infer/Finish within overall count/unit bounds.

## Install

```toml
[dependencies]
raiv = "0.0.1"
```

Runtime dependencies already disable their default features. Core uses `alloc` but no filesystem, clocks, networking, environment, threads or std-dependent runtime. Native host approval/external I/O stay outside this crate.

## Entry points

- [`src/lib.rs`](https://github.com/atmo-network/raiv/blob/v0.0.1/lib/src/lib.rs): Public module surface; codecs remain private to `format`, schemas stay with domain owners.
- [Project overview](https://github.com/atmo-network/raiv/blob/v0.0.1/README.md): Current APIs, support boundaries and root workspace commands.
- [Project constraints](https://github.com/atmo-network/raiv/blob/v0.0.1/AGENTS.md): Durable rules and validation gates.
- [Specification and documentation](https://github.com/atmo-network/raiv/blob/v0.0.1/docs/README.md): Design contracts, not implementation status.
- [Remaining work](https://github.com/atmo-network/raiv/blob/v0.0.1/BACKLOG.md) and [completed outcomes](https://github.com/atmo-network/raiv/blob/v0.0.1/CHANGELOG.md).
- [Shared conformance fixtures](https://github.com/atmo-network/raiv/blob/v0.0.1/fixtures/README.md) and [project license](https://github.com/atmo-network/raiv/blob/v0.0.1/LICENSE).

Run `cargo test --locked` from the workspace root or this directory. The root workspace selects this library as its default member and owns the shared lockfile/target directory; `cargo test --locked -p raiv` selects it explicitly. The full unit/conformance suite is repository-owned: fixtures remain at the workspace root and test-only source includes are relative to that shared corpus. The crates.io archive supports standalone library builds and doctests; running the full unit/conformance suite requires the repository checkout.

`preparation::check_input(&RestoredAgent, &ActivationInput, &Limits)` exposes the existing borrowed caller/Definition wire + cause/phase/current-Command Receipt-ID/domain-digest pairing check to adapters without copying payloads. It is data compatibility only: no event freshness, grant approval, Receipt provenance or counter consumption.

Core does not perform external I/O or authorize host acceptance/dispatch. The separate [guest/native application foundation](https://github.com/atmo-network/raiv/blob/v0.0.1/apps/raiv-agent/README.md) implements isolated scripted reply progression and exact candidate-data step transport; trusted local acceptance and synthetic effect accounting are available there, while authentic external effects/funding/authority remain separate open work.
