# Raiv backlog

Canonical open work after [Raiv 0.0.2](docs/raiv.spec.en.md). The specification owns requirements, this file owns remaining work, and [CHANGELOG.md](CHANGELOG.md) owns completed outcomes. Acceptance IDs (C/L/H/T/D/W/M) refer to the specification's test matrix.

## Guardrails

- Method: assess reality → choose one bounded validation cohort → implement → run focused and shared checks → reconcile this file.
- Never operate real funds, invent missing host authority, add a persistent transcript, or report skipped live/IPFS checks as passed.
- Publishing requires an explicitly selected owner release scope. A concrete blockchain/JAM integration requires an ABI and admission owner.
- One logical change per commit; formatting-only and normative-spec changes get their own commits.
- Do not edit normative content or manufacture conventions just to satisfy a generic validator; module dependency direction is checked by hand.

## 0.0.3 scope

Focus: a sturdier library, honest evidence, less weight, and a demo that is pleasant to use. On-chain mechanics stay out of scope.

### Reliability

1. [ ] **Resolve zero-effect failed launches.** A launch that fails before any Command (for example `corrective_turns_exhausted` or a provider `length` stop) leaves an unresolved admission that blocks the session forever; this already happened to a real session. Specify, then implement, an explicit operator-approved resolution that consumes the event without a boundary and records the lost attempt (H05, H12). Done when the blocked session can continue after one explicit approval, with no automatic retry, refund or replay.
2. [ ] **Read the pinned Definition from the session.** The host recomputes a session's Definition from its compiled guest profiles, so changing an existing profile's policy or descriptors would orphan sessions (§18.6). Load and verify the Definition from the accepted closure instead. Done when an existing session keeps running after a descriptor change, and new `opencode-go` sessions can adopt the `anyOf` plane schema (W13).
3. [ ] **Explain blocked and legacy sessions.** `status` and the launcher should say why a session is blocked (unresolved admission, pending action, changed configuration) and what the operator can do next. Sessions created before 0.0.2 keep 1024 tokens with high reasoning, which can stop on `length`; the launcher should point this out instead of failing opaquely.
4. [ ] **Status while a session is open.** `status` can fail while the TUI holds the session directory. Make read-only inspection work alongside a running session, or report that the session is in use.
5. [ ] **Run the live launcher witness once.** The success path of `test-live-launcher.sh` has never run (about four model calls). Done when it passes, or its failure is fixed and recorded.

### Library and framework

6. [ ] **Embedding example.** A short compiled example, a crate doc or an `examples/` program, that restores a genesis checkpoint, prepares an Activation, advances it with a scripted reply and composes a candidate. Done when a Rust developer can embed Core from that example alone.
7. [ ] **Memory-enabled evaluator profile.** Agents that judge a stream of similar cases (for example referendum filters) benefit from remembering earlier cases, but untrusted inputs can try to plant instructions in Memory. Design an additive profile that keeps evaluator knowledge in Memory while resisting poisoning. Candidate measures: policy that stores conclusions rather than input text, explicit provenance of notes, bounded growth, and adversarial fixtures that try to persist injected instructions. Done when the profile exists with offline tests that show injected text does not become a later instruction.
8. [ ] **Memory size pressure.** Memory is capped at 4 KB. Tell the model how close it is to the cap in patch results, and show it in the TUI, so the agent prunes before patches start failing.
9. [ ] **Provider breadth.** Live evidence on at least one more `openai-compatible` endpoint (W12). Decide whether loopback `http://` endpoints are allowed for local servers. Narrow README/spec wording to what is evidenced. Other API families need separate adapters; `openai-responses` remains a Chat Completions alias.
10. [ ] **Provider error classification.** Show a bounded, credential-free category for refused provider responses (for example an unsupported parameter or a length stop) in the TUI. An earlier opt-in classifier is recoverable from `apps/raiv-agent/host/src/inference/http/diagnostic.rs` at `v0.0.1`.

### Demo

11. [ ] **Interactive responsiveness.** Show streamed text provisionally, and let Esc interrupt a running inference without replay or partial acceptance (W07, W08, W11). Today inference is synchronous and exit waits for the bounded decision.
12. [ ] **Demo polish.** Owner-driven improvements found while testing the demo. Each one is a separate bounded item with its own check.

### Evidence

13. [ ] **Owner review of 0.0.2 specification decisions.** Prepare an annotated diff of the 0.0.1 → 0.0.2 specification: provider families, dated-snapshot matching, empty-id stream continuations, `anyOf` schema, failure categories, compact JSON presentation, pinned local sessions. Each decision is confirmed, changed or reverted.
14. [ ] **Golden vectors for 0.0.2 additions.** Pin the `openai-compatible` Definition, the `anyOf` patch schema, JSON presentation cases (C19) and patch feedback (M17) as fixtures, built with the independent Ciborium oracle.
15. [ ] **Independent hashes for new vectors.** New fixture SHA-256 values come from the same `sha2` crate as the runtime. Cross-check every fixture hash in CI with coreutils `sha256sum`, so hash evidence stays independent of Raiv's dependencies.
16. [ ] **Memory continuity benchmark.** A reproducible set of tasks spread over several launches whose quality depends on remembered facts and strategy. Compare sparse Memory with no memory and with a full transcript. Offline scripted runs in CI; live runs opt-in. This measures the project's central hypothesis instead of assuming it.
17. [ ] **Coverage-guided fuzzing and measurements.** Seeded mutation tests cover Core decoders and the patch path. Add coverage-guided fuzzing of ABI frames, graphs and provider replies, plus measured RAM high-water, restore/encode time and binary size (§21).
18. [ ] **Shell lint.** Run `shellcheck` on `run.sh` and the test scripts in CI.

### Simplification

19. [ ] **Condense the library API guide.** `lib/README.md` carries about 20 KB of dense API narration. Keep a short module map and move per-function detail into rustdoc.
20. [ ] **Condense the application README.** `apps/raiv-agent/README.md` is about 45 KB. Keep quick start, launcher, headless commands and checks; move internals to the host README, the specification or rustdoc.
21. [ ] **Review RAM-only layers.** Volatile paths such as `accept_ram` and the in-memory executor book predate their durable counterparts. Keep each only if it carries unique acceptance evidence; otherwise fold its coverage into the durable path's tests and remove it.
22. [ ] **Plain-language pass.** Public docs say what Raiv does before listing what it does not, and state determinism precisely: transitions are reproducible from admitted inputs and replies, while model behavior itself is not deterministic. A readability pass over the specification changes no requirement.
23. [ ] **Release checklist.** Record the flow that worked for 0.0.2: release branch → PR → green CI → rebase-merge → annotated tag → `cargo publish -p raiv --locked` → GitHub Release.

## Later milestones

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

### On-chain direction

- [ ] Agents as a function of the chain (for example referendum veto filters): chain event → ActorWake, content-pinned external inputs, vote submission with finality-bound Receipts, decision roots anchored next to votes, and aggregation rules across diverse agents. Out of scope until an owner selects it.

## Evidence

- [ ] Trace all 96 acceptance requirements to executed evidence; a build alone is not completion. The CI workflow covers Core (1.85/stable), host 1.86/stable with guest 1.85/stable, the fixture self-consistency test, dependency graph, rustdoc, documentation links and launcher regressions. It is not full acceptance, sandbox/JAM readiness or power-loss proof.
