# Native host

Package `raiv-agent-host` (library `raiv_agent_host`, binary `raiv-agent`) is the only std component. It requires Rust 1.86 for pinned Wasmi 2.0.0, runs the import-free guest with finite memory and per-step fuel, and owns every external capability: inference transport, storage, local acceptance, the synthetic world and the terminal. It never builds context, selects tools or overrides guest decisions; those stay in the [guest](../guest/README.md).

- [Build, runtime checks and ABI](../README.md)
- [Specification](../../../docs/raiv.spec.en.md), in particular the reference execution boundary, step ABI and reference-agent sections
- [Remaining work](../../../BACKLOG.md)

## Module map

Each module's header comment states its boundary, and its tests are the detailed contract.

| Area | Modules | Responsibility |
| --- | --- | --- |
| Guest execution | `lib.rs` (`GuestVm`), `run.rs`, `output.rs`, `atomic.rs` | Wasmi instance with finite limits, live-run routing with process-local request tickets, closed guest output decoding, checked unique ID allocation |
| Inference | `inference.rs`, `inference/{http,sse,json,request}.rs` | Bounded HTTPS Chat Completions, duplicate-key JSON preflight, SSE assembly and normalization, provider-specific request mapping and reasoning continuation |
| Storage | `store.rs`, `disk.rs`, `disk/manifest.rs` | Exact CID-indexed RAM and Linux disk blocks; locked no-overwrite write, fsync and readback; atomic manifest file |
| Acceptance | `registry.rs`, `admission.rs`, `launch.rs`, `candidate.rs`, `checkpoint.rs`, `acceptance.rs`, `acceptance/local.rs`, `policy.rs` | Administrative program registry, ActorWake/ActionResult admission, Reserved → Attempted → Launched accounting, candidate closure checks, typed root/consumption manifest, boundary commit and approved pins |
| Recovery | `recovery.rs`, `recovery/reservation.rs` | Explicit pinned adoption and zero-attempt reservation continuation; never automatic relaunch, refund or replay |
| Synthetic world | `simulator.rs`, `simulator/{checkpoint,local,recovery,world}.rs` | Scripted virtual-unit world with an exact Receipt ledger; not a market, backend or dispatch permit |
| Terminal | `terminal.rs`, `terminal/session.rs` | Two-region Ratatui/Crossterm presentation and owned TTY session |
| Application | `main.rs`, `app.rs`, `app/*.rs` | CLI commands, configuration, interactive controller, Memory presentation, one-boundary wake/resume, pending execution, scripted scenario, world provisioning |

## Guarantees and limits

- Local acceptance is local-only: exact disk readiness and a typed manifest CAS before RAM publication, not global HEAD, finality or dispatch authority.
- Uncertain outcomes keep the admission unresolved; nothing retries, refunds or replays automatically.
- Credentials come only from the configured environment variable and never reach the guest, context, state, logs or test output.
- The synthetic world never performs real effects or spends real funds.

Detailed per-slice history of these modules is in the [0.0.1 changelog](https://github.com/atmo-network/raiv/blob/v0.0.1/CHANGELOG.md#pre-release-development-evidence) and the previous [host README](https://github.com/atmo-network/raiv/blob/v0.0.1/apps/raiv-agent/host/README.md).
