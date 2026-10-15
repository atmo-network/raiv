# Raiv backlog

Canonical open work for implementing [Raiv 0.0.2](docs/raiv.spec.en.md). The specification owns requirements, this file owns remaining work, and [CHANGELOG.md](CHANGELOG.md) owns completed outcomes. Acceptance IDs (C/L/H/T/D/W/M) refer to the specification's test matrix.

## Guardrails

- Method: assess reality → choose one bounded validation cohort → implement → run focused and shared checks → reconcile this file.
- Never operate real funds, invent missing host authority, add a persistent transcript, or report skipped live/IPFS checks as passed.
- Publishing requires an explicitly selected owner release scope. A concrete blockchain/JAM integration requires an ABI and admission owner.
- Do not edit normative content or manufacture conventions just to satisfy a generic validator; module dependency direction is checked by hand.

## Next

1. **Read the pinned Definition from the session.** The host recomputes a session's Definition from its compiled guest profiles, so any change to an existing profile's policy or descriptors would orphan sessions (§18.6). Load and verify the Definition from the accepted closure instead. Done when an existing session keeps running after a descriptor change, and `opencode-go` can adopt the `anyOf` plane schema for new sessions (W13).
2. **Resolve zero-effect failed launches.** A launch that fails before any Command (for example `corrective_turns_exhausted`) leaves an unresolved admission that blocks the session forever. Specify and implement an explicit, operator-approved resolution that consumes the event without a boundary and records the lost attempt (H05, H12). This needs a specification decision first.
3. **Interactive responsiveness.** Show streamed text provisionally, and let Esc interrupt a running inference without replay or partial acceptance (W07, W08, W11). Today inference is synchronous and exit waits for the bounded decision.
4. **Provider breadth.** Live evidence for more `openai-compatible` endpoints (W12), and a decision on whether loopback `http://` endpoints are allowed for local servers. Show a bounded classification of refused provider error bodies (for example an unsupported parameter) in the TUI; an earlier opt-in classifier is recoverable from `apps/raiv-agent/host/src/inference/http/diagnostic.rs` at `v0.0.1`. Other API families need separate adapters; `openai-responses` remains a Chat Completions alias.
5. **Coverage-guided fuzzing and measurements.** Seeded mutation tests cover Core decoders and the patch path. Add coverage-guided fuzzing of ABI frames, graphs and provider replies, plus measured RAM high-water, restore/encode time and binary size (§21).

## Milestone work

### Guest and step bridge (W02–W04, C01)

- [ ] Route live runs through durable registry policy, external Actor/Receipt provenance and durable admission/consumption. Process-local AdmissionBook and RequestTicket freshness are not cross-process authentication.
- [ ] Extend native/Wasmi equality to broader adversarial profiles and codec cases. Equality for the shipped scripted and live profiles exists.

### Local trusted host (H01–H15, T01–T08)

- [ ] Durable authority/availability manifest policy for the program/genesis Registry.
- [ ] Trusted external Actor/Receipt provenance and real-executor-ledger-bound ActionResult admission.
- [ ] Attempted/Launched exact-input recovery with genuine lost-unit, funding and quote accounting.
- [ ] Independent main/world pin retention and current selection; reconciliation after Core-store loss without world rollback or effect replay.
- [ ] Real executor world/Receipt coupling, complete trade/portfolio/wake semantics, effect-time funding and dispatch permission.
- [ ] Prove H01–H15 and T01–T08 on the tested OS/filesystem. SIGKILL and reopen evidence is not power-loss proof.

### Availability and transport (C10, C16–C17, D01–D08)

- [ ] Strict CAR import/export (`export`, `verify`, `import`) and a second exact-byte store.
- [ ] Closure-bound readiness, retention/renewal, accounting, failure handling and primary-provider-loss recovery (`run-scenario provider-loss`) without re-executing effects.

### Interactive and reference agent

- [ ] Physical-terminal usability, bounded raw ingress, signal and I/O-failure outcomes.
- [ ] Separate IPFS adapter; run IPFS smoke only with approved access.
- [ ] Watch: if the host 1.86 parallel executable init failure recurs, diagnose it from the retained init stderr rather than hiding it with retries or serialization.

## Evidence

- [ ] Trace all 96 acceptance requirements to executed evidence; a build alone is not completion. The CI workflow covers Core (1.85/stable), host 1.86/stable with guest 1.85/stable, fixtures, dependency graph, rustdoc, documentation links and launcher regressions. It is not full acceptance, sandbox/JAM readiness or power-loss proof.
