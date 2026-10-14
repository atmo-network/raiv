# raiv

Raiv 0.0.1 is a working MVP for preserving an agent's behavioral continuity across short executions through persistent memory and reproducible checkpoints: a portable Rust library plus an isolated WASM guest/native reference application. It is not production or financial/blockchain readiness.

## Run the interactive demo (Linux)

With `OPENCODE_API_KEY` already exported, run [`./run.sh`](run.sh). The Bash launcher builds the real WASM/native host, prepares a private local session and asks for explicit source/plan approval before opening the TUI. State stays in `var/raiv/chat`, outside Cargo's `target/`; reopening uses the same accepted Memory, never a restored transcript or automatic retry. See [setup, limits and other launcher commands](apps/raiv-agent/README.md#simple-bash-launcher).

## Start here

- [Library package](lib/README.md)
- [Reference guest and native bridge](apps/raiv-agent/README.md)
- [Project constraints](AGENTS.md)
- [Normative specification and documentation](docs/README.md)
- [Remaining implementation work](BACKLOG.md)
- [Completed development and validation outcomes](CHANGELOG.md)
- [Conformance fixtures and provenance](fixtures/README.md)
- [Rust boundary CI definition](.github/workflows/rust-boundaries.yml)

## Current status

The library is **`no_std + alloc`**, requires **Rust 1.85 or newer**, and implements bounded typed protocol data, sparse object-patch Memory, borrowed checkpoint projections, owning RAM Activation and pure verified CommitCandidate composition.

The import-free Rust wasm32v1-none guest runs scripted decisions through a finite Wasmi 2.0.0 native host. Native routing validates output/source transitions and uses process-local correlation tickets; explicit RAM registry/admission gates and full-readback checkpoint CAS remain isolated volatile computation. MemoryBlockStore and Linux DiskBlockStore preserve exact CID-indexed bytes; the latter adds locked no-overwrite write/fsync/reopen/readback, not durable acceptance.

Local-only sealed boundary commit now couples full disk readiness with one typed root/consumption manifest before RAM publication; pinned local recovery and exact uncertainty retries are available. A separate scripted one-unit world now atomically persists virtual reservations/debits with exact terminal Receipts, independently of Core metadata; it is not a market/backend implementation or live dispatch permit. Paired-pin world-session adoption preserves balances/Receipts/intents with new native identities and no automatic submission; its ledger-bound ActionResult gate approves only exact installed terminal bytes after primary/main acknowledgement, before RAM reservation. The Linux [scripted CLI and bounded retry-and-sleep demo](apps/raiv-agent/README.md#runnable-retry-and-sleep-demo) run actual WASM/local acceptance, fresh paired-pin adoption and a second ActorWake without effect replay. The [bounded online interactive demo](apps/raiv-agent/README.md#interactive-terminal-native-local-test-approval) also uses real opencode-go/deepseek-v4.1-flash high-reasoning inference through Wasmi, with accepted sparse Memory and independently pinned fresh UI/VM continuation for its previously approved profile/module. Current natural-completion profiles are native/WASM-validated and have a separate fresh bounded Go/high witness; all use new explicit approval, never rebinding that older state. Independent durable pin retention/currentness, after-Core-loss reconciliation, full admission/accounting-to-real-effects application, external provenance, CAR/replication and broader provider/terminal operations remain open. Nothing here is ready for real financial operations or blockchain deployment.

### Canonical data

- `raiv::format`: fixed-profile `DaslCid`, `validate_drisl`, `validate_model_value`, flat owned `ModelValue` and borrowed `ValueRef`. Canonical map order uses encoded text-key order; Unicode is validated, not normalized. Strict preflight rejects duplicates, nonminimal encodings, trailing data and floats before payload ownership.
- Six structural records: `Definition`, `Memory`, `ActivationInput`, `Boundary`, `Episode` and `AgentState`, each owned by its corresponding module. `Command`, `Receipt`, `Position`, `BudgetAdmission`, `Usage` and `Limits` are embedded data, not additional state blocks.
- `raiv::identity::{episode_id, action_id, command_digest}` implements the pinned DRISL/SHA-256 domains. Runtime fields retain full u64; model values exclude Bytes/Links and restrict integers to ±(2^53−1). Unknown costs remain distinct from known zero.
- `DecodeLimits` bounds one complete object. `Limits` retains all eighteen policy fields and demo defaults; decoding embedded policy never authorizes it or widens caller bounds. Encoders and borrowed nested readers preserve contextual byte/depth/count limits without recursive owned trees or duplicate embedded payload sets.

Independent test-only Ciborium checks all 38 standalone record vectors; the selected DASL profile executes all 125 rows. Runtime dependencies retain minicbor and fixed-profile CID handling, with no universal IPLD Value/registry or IPLD/CID dependency crates.

### Checkpoint-data restoration

`raiv::graph::RestoredAgent::restore(root, blocks, &Limits)` validates a complete bounded closure supplied through `MaterializedBlocks`; there is no hidden I/O, hydration, previous-root chain or fallback to genesis/empty Memory. Required blocks are checked by exact CID and typed role; unrelated store bytes and scalar evidence references are not dependencies.

The snapshot composes Episode-role, identity, ordinal/cause/Receipt, memory-lineage, history-window, Usage, linked-input grant and declared Yield-descriptor checks. It exposes borrowed state/Definition/Memory/current Episode/history and typed Boundary/Input/Memory lookups. Unrelated or wrong-kind queries return `None`. `from_inspected()` validates and moves an `InspectedClosure` without another store read or payload set; inspection and individual check stages remain available for diagnostics. `Genesis::build()` and `Genesis::restore()` stay explicitly revision-zero-only.

Caller limits and linked Definition caps only tighten. Definition history zero still retains one completed Episode; caller history zero is an actual materialization capacity and rejects rather than truncating. Aggregate encoded-byte accounting is **not** measured allocator/RAM high-water. Retained memory joins are checked without inventing an anchor for the earliest retained entry or pruned history.

**Restored data is not execution authority.** Successful restoration does not reconstruct an Activation/transcript, approve application arguments/code/registration, establish grant freshness/top-ups/units or lifetime/caller execution policy, select authoritative HEAD/finality, or authorize dispatch.

### Retained data projection

`raiv::projection::HistoryProjection::new(&snapshot)` exposes completed Episodes oldest-to-newest and the current open chain separately. Episode views borrow entry/latest checkpoint Memory; action views pair exact Command bytes with the next input's terminal Receipt. Final Finish contributes the previous Receipt, not an extra action; a current pending Command keeps `None` until a new input is admitted. Definition history zero hides completed projection without deleting retained bytes or the current chain. Disabled memory remains `None`. Borrowed views introduce no payload copying, allocation or store reads.

`ActionProjection::command()` and `receipt()` expose schema-owned `CommandView`/`ReceiptView` fields, including exact model argument/result/cost spans and typed runtime IDs/versions/Position. Owned records also offer `view()`. Standalone view decoders validate caller bytes under the same record limits without another payload set; preflight may allocate bounded traversal metadata.

`HistoryProjection::encode(&Limits)` produces one bounded transient model-compatible CBOR fragment: completed history first, then the current chain. Runtime IDs become lowercase hex text and full-u64 metadata becomes exact decimal text; argument/result/cost maps and compact Memory remain validated model values. Caller/Definition context-byte, depth and count limits only tighten, including actual embedding depth; oversize projection fails without truncating Episodes. This fragment is not a structural block and is bounded by context bytes rather than block bytes.

### Initial context data

`raiv::projection::InitialProjection::build(&snapshot, &input, &Limits)` captures a six-section immutable prefix: policy/mandate, declared tools, machine input, completed history, current checkpoint chain and initial Memory. Incoming input wire caps, phase and pending Command/Receipt ID/domain digest consistency are checked before capture. Runtime descriptor/input metadata normalizes losslessly; a machine event is explicitly tagged as machine data, not a human request.

`context(&trajectory)` adds the seventh RAM-trajectory section and validates the complete canonical CBOR byte size, including all message and declared descriptor data. Limits remain the captured caller/Definition intersection. Initial prefix bytes never change; working-memory updates belong in appended trajectory data. Oversize/deep/count-invalid trajectories fail without truncation or prefix mutation.

This projection API is context DATA, not a ready request by itself. Initial RAM preparation now composes it with registered/count/unit guards below; fresh host admission/program approval, provider normalization/JSON transport and full advancement/commit acceptance remain separate. Trajectory entries are data, not automatically accepted tool results. Native/WASM lifecycle acceptance is still open.

### Registered implementation binding

`raiv::tools::ToolSet::bind(&definition, &registrations, &Limits)` checks an exact whitelist/registration bijection under caller/Definition count ceilings. Version, control, description, canonical schema bytes and authority must match; compiled capability control must agree too. Memory-disabled profiles cannot bind a memory-patch implementation, regardless of its name. Binding and lookup invoke no validator/planner and allocate no payload or metadata buffers.

The set is a Copy/Clone immutable view retaining a Definition CID and borrowed registrations, not a reference into the owned snapshot. Copying it never rebinds, allocates, invokes callbacks, grants authority or extends registration lifetimes. `tools(&definition)` checks that CID and yields Definition order; `lookup(id)` has no unknown-tool fallback. Implementations are intrinsic MemoryPatch or supplied Pure/Yield callbacks with read-only PlanningContext. Supplied metadata agreement does not prove caller/program approval, schema-validator honesty or native callback purity. Binding itself grants no execution; prepare/advance_ram perform scoped selection/invocation under supplied policy, while trusted dispatch approval remains host work; native purity is a code contract, not a sandbox guarantee.

### Prepared-packet data binding

`raiv::preparation::PreparedInput::new(binding, input)` adopts existing owned input bytes and pins agent/domain/root/revision plus admitted event ID and exact input SHA-256 digest. `validate(&snapshot, &tools, &caller)` checks those snapshot identities, ToolSet's Definition CID, input phase/Receipt pairing, raw caller closure/history/wire/embedded-record caps and events already present in retained data. It returns borrowed `ValidatedInput` data with unchanged captured caller limits, without store reads, another payload set or callback invocation.

This is not host approval or ready Activation. Pruned/global event freshness, Actor/Receipt source, accepted HEAD/run/program binding, execution budgets/units/lifetime and live inference/call consumption remain separate. A repeated uncommitted packet can still be checked for recovery; the library does not invent a global consumed-event ledger or shrink Definition history policy.

### RAM count and inference gates

`raiv::activation::CallBudget::from_input(&validated)` derives count capacities from the current Episode's accepted Usage/actions, Definition/caller ceilings and input grants. ActionResult does not reset Episode counts; a fresh ActorWake excludes completed history. Explicit model/Continue/Yield/corrective debits preserve counters on refusal; exhausted corrective attempts report CorrectiveTurnsExhausted, never synthetic Finish. Counts remain full-u64.

`count_enabled_tools(&tools, &definition)` yields registered Definition-order views filtered by Continue/action capacity, with no callbacks or payload allocation. Action exhaustion removes Yield descriptors, not intrinsic Finish; the last admitted model request may still explicitly finish after no new model call fits. These are **count** guards, not completed Activation or ready-inference permission: execution-unit reservation/consumption, host approval, complete reply/call validation, invocation/candidate construction and overall-budget pause remain separate.

`InferenceSession` owns a CallBudget and a runtime-supplied instance `RequestSequence`. `issue()` count-debits once and creates one pending monotonic ID; repeated pending views do not debit. Wrong/stale/duplicate replies or errors cannot advance that pending state. Matching complete replies enter ReplyReady; explicit Continue/correction permit another request, Yield/Finish seal, and inference/count/corrective failure cannot create a candidate. A terminal session releases its non-Clone sequence once for the next Activation without ID reuse. Runtime code must supply fresh instance labels, validate full replies/cardinality/tools/units before control and resolve acceptance before reuse: this session is not a complete live Activation or host-authority proof.

`UnitBudget::from_input(&validated)` captures the exact packet/admission and both activation/remaining-Episode unit capacities. `issue_with_units()` and `control_with_units()` compose packet, phase, count/nonce and trusted profile-quote checks before reserving units; wrong scope/phase or count refusal does not spend them. Unit exhaustion fails without a candidate, even for a zero quote at zero available units. Positive-capacity free operations remain count-bounded. Reservations are conservative upper-bound debits, **not exact bills**: unknown Usage/failure cannot refund them or supply token-to-money conversion. Quote pricing/freshness/host authority, whole-runtime retention and lifetime/lost-attempt accounting remain external; count-only APIs alone are not unit admission.

### Owning RAM Activation

`raiv::activation::prepare(snapshot, packet, &tools, RunPolicy, RequestSequence)` owns restored/input data, captures the immutable prefix/empty trajectory and creates a separate working Memory only when enabled. The Activation retains ToolSet's immutable view by value, borrowing only its independent registration metadata; the caller's local ToolSet wrapper may move/drop, so a live runtime can store the Activation without self-reference or leaked storage. Policy supplies raw caller limits and trusted profile ExecutionQuotes. All input/closure/history/context/allocation preflights precede the first model-count/nonce/unit reservation; failure returns the unspent sequence. Caller capacity must retain the next promised completed window, and revision overflow/no affordable registered or intrinsic control rejects rather than inventing completion.

`Activation::request()` repeatedly borrows the same ModelRequest with binding, version-1 canonical context, exact Definition model-parameter Map and count/unit-enabled descriptors in Definition order. Declared-prefix metadata is not a dispatch list. Operational limits only tighten; Definition history policy and raw caller capacity are exposed separately. Wrong inference-error keys preserve it; matching failure produces no candidate and permits terminal sequence extraction. No provider or callback runs during preparation. RAM advancement and pure candidate construction are available below; **durable host acceptance/dispatch is not implemented**; host/run/principal/freshness/price authority is not proven by these Rust types.

`Activation::advance_ram(binding, bytes)` admits the complete reply, rejects all multiple/unknown/disabled/invalid calls before handlers and debits before registered callbacks. Patch, Pure and Yield planner capabilities run with read-only context; complete bounded results append to transient trajectory without rewriting the initial prefix. Patch output is staged until combined context fits, preserving working Memory on failure. Errors take bounded correction; fatal failures close without checkpoint. `RamProgress::InferAgain` yields a new reserved request; `Sealed` retains read-only `RamDecision::Yield(Command)` or intrinsic Finish inside Activation from a complete zero-call reply (text may be empty). A failed planner spends admitted units but never skips an action slot. `usage()` keeps actual Core call counts and overflow-checked nullable reported costs, never reservation-as-bill guesses. This is not CommitCandidate, acceptance or dispatch.

### Bounded normalized replies

`raiv::activation::ModelReply::decode(bytes, &Limits)` borrows a closed normalizer-v1 CBOR frame with request ID, text, Usage and call metadata. `control_call()` first rejects **all** multiple calls, preserving every native call ID through `calls()` for adapter error results; only a single call exposes validated canonical model-Map arguments. Zero calls mean no control decision, never Finish. Runtime u64 metadata and unknown costs remain exact; whole-frame reply/depth/count bounds are captured without an owned payload/call set.

`InferenceSession::accept_normalized_reply(key, bytes, &definition, &caller)` checks pending instance/request correlation before parsing, intersects Definition/caller reply bounds, rejects a mismatched body ID and consumes only complete valid frames. Malformed/stale frames preserve pending state; admitted multi-call/invalid-argument replies remain recoverable at selection before any handler effect. Provider JSON/SSE normalization, application/schema/control/unit authorization, error-result encoding and actual execution still belong to their runtime/adapter layers; reported Usage never overrides Core counters.

### Candidate checkpoint data

`Activation::candidate()` builds `raiv::commit::CommitCandidate` only after a successful sealed Yield/Finish decision (also after terminal request-sequence transfer). It encodes the next Boundary/Episode/AgentState, increments revision once, preserves episode entry/trigger identity, closes only on Finish and retains the Definition-promised chronological history window. It restores and validates the entire resulting required closure under captured caller/Definition limits before returning any candidate; failure leaves the sealed run unchanged and retries require no inference or callback/count/unit debit.

The immutable candidate exposes its exact prepared `InputBinding`, `new_root()`, `disposition()`, validated `snapshot()`, complete `blocks()` and binary-CID-sorted unique `new_blocks()` relative to the source closure. `command_bytes()` borrows the very Command embedded in the last Boundary, never a second mutable dispatch instruction. Unchanged/previously retained Memory is reused. Root bytes contain no previous-root link, RAM turns, reply text or native call metadata. Whole-closure byte/block limits apply, not just per-record caps; bounded validation owns a candidate closure while the source remains intact, so encoded-byte totals are not measured RAM high-water.

A candidate or `into_snapshot()` transfers DATA, not canonical HEAD, fresh host admission, availability or execution authority. The host must place/retain the required closure, bind the authorized run and resolve root/revision CAS/event acceptance before dispatch or canonical continuation. Native scripted data recovery covers retry → success → set_wake → Finish → new ActorWakes with exact independent golden roots, monotonic request IDs, history zero/minimum/eviction, preserved Memory and a disabled-memory chain. These simulations deliberately do not act as an acceptance host or executor. Actual guest/host integration remains in [BACKLOG.md](BACKLOG.md).

### Atomic working memory

Core and guest now emit the [organic semantic-editing report](docs/raiv.spec.en.md#62-patching): unsupported roots are skipped without synonym remapping; selected content remains strict. Mixed/all-skipped Continue returns full accepted Memory plus processed/skipped reasons and actual change, without forced correction. Generated reports obey result/context caps, independently of inbound model-reply size; full embedding checks still precede assignment. Pinned policy, compiled descriptors and scripted examples distinguish intended roots from tolerant selection; actual native/WASM results match independent report witnesses. Broader application/acceptance work remains in [BACKLOG.md](BACKLOG.md).

`raiv::memory::Memory::encode(&planes, &DecodeLimits)` and `decode()` own the first-standard sparse `intents` / `strategy` / `tactics` object. Present planes must be objects; normalized empty Memory is `{}`, distinct from a disabled profile. Persisted nulls, empty object fields and aliases reject rather than being repaired. Empty objects in array positions are allowed. The enabled Definition profile is `intents-strategy-tactics/1`; structural record versions remain 1.

`Memory::apply_patch(arguments, &Limits)` and `PatchArguments::decode(arguments, &Limits)` accept canonical model-CBOR for the JSON object patch itself, not serialized JSON text, an operations envelope or paths. PatchArguments borrows exact bytes and captures authorized limits. `processed_planes()` and `skipped_planes()` borrow canonical root names; skipped names report unsupported_root and are never aliases. `report(&before, &accepted)` produces bounded canonical model DATA `{memory,changed,skipped_planes:[{plane,reason}],processed_planes}`; processed does not imply changed, and the report is not persisted Memory. `patched(&memory)` stages one candidate output buffer; `apply(&mut memory)` atomically returns complete compact accepted Memory. The native JSON adapter must reject duplicate keys before CBOR conversion; that adapter is not implemented yet.

Objects merge recursively, including into `{}` when the old value is absent or non-object. Missing fields survive; null deletes object fields/subtrees/planes. Arrays replace wholly and must contain no null at any depth, checked before cleanup. Nested names including dots/slashes/`[0]`/`$ref` are literal. `{}` does not clear a nonempty object. Whole-draft normalization removes empty object fields and empty planes, preserving root `{}`, array positions, `false`, `0`, `""` and `[]`.

Complete raw-input model-data/duplicate/resource bounds include skipped payloads. Object/null shape and replacement-array null checks apply strictly under selected roots; skipped values are not interpreted as patches. Final Memory/result/trajectory/context checks precede RAM assignment. `max_patch_entries` bounds aggregate map fields/array members in the full patch. Private iterative linear map merge uses a flat borrowed-span plan bounded by both preflighted input byte lengths and one output buffer, without a recursive owned value tree. Failure preserves original bytes/CID/assignment. No-effect Continue retains its block/CID and still charges actual work. Patches create no checkpoint or journal and never rewrite the initial prefix. Tactics cannot cancel accepted Commands; intrinsic Finish closes the Episode without clearing Memory.

The caller must supply authorized Definition/caller limits. This pure operation does not register a tool, debit a budget or enable memory on a disabled profile. RAM advancement stages both `patched(&memory)` output and its report before combined trajectory/context acceptance, then installs Memory; partial/all-skipped results remain successful Continue, not a corrective retry; durable commits, provider adapters and full native/WASM lifecycle proof remain in [RAM-core work](BACKLOG.md).

## Execution boundary

Core and the Rust `wasm32v1-none` guest remain `no_std + alloc` on Rust 1.85. The native `raiv-agent` bridge alone uses std and separately requires Rust 1.86 for pinned Wasmi 2.0.0. Its import-free, bounded initialization, retained Activation/reply progression, error correlation, buffer boundary and fixed candidate checkpoint bytes/CIDs have actual native/WASM evidence for the synthetic profiles; this does not establish trusted acceptance, effects or arbitrary/live profile coverage.

The guest must own context, tools, decisions and transitions and retain live Activation across inference. Host-only filesystem/config/environment credentials, HTTPS/JSON/SSE, acceptance/recovery and the two-region ephemeral-history/editable-prompt terminal remain future work. See the [application build and runtime checks](apps/raiv-agent/README.md).

## Workspace layout

The root `Cargo.toml` workspace contains `lib` (the default), `apps/raiv-agent/guest` and `apps/raiv-agent/host`. `lib/Cargo.toml` retains the `raiv` package identity, version and exact dependency declarations; normal Rust sources live in `lib/src/`. Cargo.lock, target output, shared fixtures, project documentation and LICENSE remain at the root. Application dependencies are isolated from Core's runtime graph.

Root commands below still select the library; use `-p raiv` for explicit package selection. Commands from inside `lib/` use the same root lockfile and target directory. See the [library entrypoint](lib/README.md) for package-local navigation.

## Development

Dependencies are exactly pinned with runtime defaults disabled. Host test/dev dependencies do not establish runtime no_std compatibility. To reproduce the MSRV gates without changing the default toolchain:

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
sha256sum -c docs/SHA256SUMS
```

The checksum command is blocked: `docs/SHA256SUMS` is missing and restoration/location versus intentional policy removal requires an owner decision. Other checks do not substitute for that gate. The generic context validator also flags eleven inherited specification-table delimiters.

The bare-metal target has no std library. Both library target builds and feature inspection establish compile-time portability evidence only—not sandbox execution, allocator integration, native/WASM parity or JAM readiness. The checked-in Rust boundary workflow defines Core1.85/stable tests/lint/portable checks, exact fixture/dependency gates and host1.86/stable plus built Rust-guest/Wasmi/isolated-process checks. It uses read-only permissions and a pinned checkout action, with no live credentials or publication. Local command evidence is not a hosted GitHub run; hosted execution remains to be observed. This scoped workflow does not resolve the checksum owner gate, inherited context errors, full acceptance coverage, fuzzing or measured RAM/resource evidence, which remain in BACKLOG.md.
