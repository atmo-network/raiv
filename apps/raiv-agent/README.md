# Reference agent boundary

The [guest](guest/README.md) owns behavior; the [native host](host/README.md) owns external capabilities. Linux `raiv-agent` supports offline init/status, explicit independent-world provisioning/inspection, one-boundary wake, separate synthetic pending execution, ledger-bound resume and the bounded **retry-and-sleep runner** below. Actual Wasmi/local acceptance/fresh pinned adoption are runnable; the unit-world is not an authentic market/wake backend. New live-profile init/status/world provisioning is supported without credentials or HTTP; bounded native HTTPS wake/resume is wired. The native interactive command is wired with finite operator approvals and two regions; real-provider compatibility, external authentication and availability remain in [BACKLOG.md](../../BACKLOG.md). See the [specification](../../docs/raiv.spec.en.md) and [applications index](../README.md).

- Core and guest: Rust **1.85**, `no_std + alloc`.
- Native host: Rust **1.86**, pinned **Wasmi 2.0.0**, std only here.
- Root default Cargo commands select Core; select application packages explicitly.
- No imports/WASI, host-built model context, transcript restoration or real effects. Native stores expose bounded immutable DATA; only explicit Registry/admission/conditional local acceptance owns trusted publication. Input/Reserved precede Attempted/Launched; uncertain attempted or submitted work never permits automatic relaunch/refund. Recovery uses separately selected exact pins, fresh origins and no serialized capability. The [host contract](host/README.md) defines these ports and uncertainty boundaries.
- ScriptedWorld owns only a separate atomic virtual-unit reservation/debit + immutable Receipt ledger. Paired adoption preserves rows/balances without reset; only its installed terminal ledger can admit ActionResult. Genuine funding/pricing, authenticated authority, independent durable pin retention/currentness, after-Core-loss reconciliation and real tool semantics remain unproven.

## Simple Bash launcher

From repository-root, [`run.sh`](../../run.sh) is the ordinary interactive entrypoint, not a test harness. Linux, Bash/coreutils, Cargo with Rust≥1.86 and the `wasm32v1-none` target are required. Install/fetch prerequisites explicitly once; the launcher itself builds locked and offline, with the credential removed from Cargo's environment:

```bash
rustup target add wasm32v1-none
cargo fetch --locked
# If OPENCODE_API_KEY is not already exported, read it without echo/history:
read -r -s -p 'OPENCODE_API_KEY: ' OPENCODE_API_KEY; printf '\n'
export OPENCODE_API_KEY
./run.sh
```

The launcher copies the exact built guest into a private new session, creates random agent/world IDs and a fixed Survivor/Go/deepseek-v4.1-flash/high configuration with a general local-assistant mandate. It initializes only a newly created empty revision-zero namespace. Review the displayed config/module hashes, root/revision/manifest, world and finite plan, then type `run` to approve that exact local source. For a fresh session this also approves the displayed new empty synthetic world bootstrap; only its successful native genesis result supplies the world pin. For an existing session the exact inspected world digest is displayed before approval. Hashes/status alone never approve adoption; the explicit confirmation is the native operator selection. Enter in the TUI separately approves each prompt. No provider request occurs merely from startup.

Defaults are **2 prompts/invocation, ≤3 model calls/prompt (≤6 total), 1024 output tokens and20s/request, actions0**, with synthetic100 activation/episode units per event. They are not genuine billing or cumulative/lifetime funding. Inference remains synchronous; Esc is deferred until its bounded decision. Configuration, guest, grants, blocks and independent world live under the NEW `var/raiv/chat` default (private/gitignored, outside `target/`). Earlier `var/raiv/default` data is preserved, never migrated or rebound. Reopening keeps accepted Memory and uses a fresh UI/VM with explicitly selected current pins; no transcript is restored. Existing config/guest/grant changes, unresolved admissions, pending actions and incomplete bootstrap refuse without overwriting, repairing, dispatching or retrying anything. A declined fresh bootstrap retains its main baseline without a world; later launches conservatively refuse that incomplete session rather than infer permission to provision it.

```bash
./run.sh --status                    # Read-only DATA inspection; no key, TTY or approval needed
./run.sh --build                     # Offline build only; no session/inference
./run.sh var/raiv/another            # Separately approved new identity/namespace; NOT recovery of old uncertainty
./run.sh --status var/raiv/another
bash apps/raiv-agent/test-run.sh      # Offline Bash/mock regressions
bash apps/raiv-agent/test-run.sh --native  # After --build: actual Wasmi startup/Esc, zero admissions
```

Interactive refusal now shows a closed native category when available: typed provider errors (including numeric HTTP status and preserved SSE framing/normalization/completeness subtypes), request mapping, reply mode, guest routing or reasoning continuation. Provider-declared length and refusal/filter markers remain failures with distinct static labels, never salvaged Finish. Arbitrary error chains, provider bodies/headers, reasoning and arguments are never formatted; unknown errors remain withheld. These RAM-only labels are observations, not retry/refund permission or persisted outcome recovery. Read-only status can establish admission/acceptance state but cannot reconstruct a previously discarded error. The owner-approved1024 envelope now has a bounded nonempty real-provider response through the production driver. The earlier256-token failures stay unresolved in their original namespaces; history/provenance belongs in [CHANGELOG](../../CHANGELOG.md), not implicit retry/migration permission.

Offline launcher validation uses an owned PTY and strips credentials from the host. The general-mandate1024 profile also has ordinary Bash-launcher/real-inference/owned-PTY Memory continuity evidence: accepted patch, then a fresh process/UI/VM on the independently checked known source copies the saved value with no nickname in the second prompt, without effect replay. This bounded witness and the production-driver textual acceptance are distinct from the older task-specific live witness below; neither proves general model compliance or physical-terminal/async behavior. Advanced init/world/pin/grant/wake/resume commands stay available below; Core/guest/host contracts and fixtures are unchanged.

### Opt-in ordinary-launcher witness

[`test-live-launcher.py`](test-live-launcher.py) uses Python only as test-side stdlib PTY and independent fixture-codec plumbing; ordinary `run.sh` remains Bash. It launches the actual script in a private retained `var/raiv/launcher-witness-*` namespace with the unmodified general mandate,1024tokens/20s, at most3calls/event and6total across two fresh UI/VM processes. It checks exact semantic Memory, independent preferred-CBOR/CID bytes, unique native event/admission IDs, closed Episodes/Consumed rows and unchanged world. Raw terminal/model text stays bounded RAM-only; a small CSI screen reducer handles differential rendering instead of confusing partial writes with full lines. Normalized provider reasoning is never exposed by the UI or persisted by the witness.

```bash
python3 apps/raiv-agent/test-live-launcher.py --check-offline  # No credential/process/network
# Explicit native local-test approval of NEW source/events and this finite inference envelope:
RAIV_LIVE_SMOKE=opencode-go/deepseek-v4.1-flash/launcher-memory-1024 \
  python3 apps/raiv-agent/test-live-launcher.py
```

`--continue-accepted <owned-session>` is only for an independently verified exact revision1/Consumed/expected-Memory test source whose first operation is already known accepted; it submits ONLY the new second prompt after checking the exact previous manifest. It cannot replay the first input, adopt unresolved state, clear data or manufacture success. Test failure/unknown state stops with retained files; never repeat automatically. The completed witness includes one test-observer correction after first acceptance, recorded in CHANGELOG, not a passing assertion of normal first-process exit or first-screen text capture. The second fresh process showed nonempty Agent text and exited by fenced Esc/process0. Python≥3.11 is a test-only requirement, not an ordinary launcher/runtime dependency.

## Build and execute the real Rust guest

`--crate-type cdylib` is a **Cargo option before `--`**, not an extra rustc argument. The guest manifest keeps an rlib for direct native parity testing.

```sh
rustup target add --toolchain 1.85.0 wasm32v1-none
cargo +1.85.0 rustc --locked -p raiv-agent-guest --lib --crate-type cdylib --target wasm32v1-none --release -- -C panic=abort
cp target/wasm32v1-none/release/raiv_agent.wasm target/wasm32v1-none/release/raiv-agent.wasm
RAIV_GUEST_WASM="$PWD/target/wasm32v1-none/release/raiv-agent.wasm" sh apps/raiv-agent/test-host.sh 1.86.0
cargo +1.86.0 clippy --locked -p raiv-agent-host --all-targets -- -D warnings
cargo +1.85.0 clippy --locked -p raiv-agent-guest --target wasm32v1-none -- -D warnings
```

Module-dependent runtime tests are deliberately ignored by ordinary host tests because they require the compiled module. The separate owned-PTY terminal lifecycle witness is also ignored and explicitly excluded from the module selection; run it only inside an owned PTY, e.g. script -q -e -c 'cargo +1.86.0 test --locked -p raiv-agent-host --lib terminal::tests::owned_tty_session_restores_termios_normal_drop_and_unwind_and_rejects_second_owner -- --exact --ignored --test-threads=1' /dev/null. It changes only its child PTY, tests termios restoration/second-owner refusal and proves no native inference/admission. Non-TTY tests never change the operator terminal. The bounded [test-host.sh](test-host.sh) runner executes ordinary native tests, then module tests with the process suite excluded, then isolated serialized process tests; it rejects a missing/unset guest module before running Cargo. Omit its toolchain argument for the current toolchain. A bare `--ignored` mixes module and process tests and is not the supported combined invocation. The [Rust boundary workflow](../../.github/workflows/rust-boundaries.yml) builds a guest per matrix pair (host1.86/guest1.85 and stable/stable), checks guest bare-metal/wasm features and executes this runner on a declared filesystem test directory; no live provider/IPFS/financial effects are enabled. They cover bootstrap, retained RAM reply progression, fixed Command/Finish candidate bytes and six independent golden roots, data recovery in fresh instances, stale/error correlation, buffer ownership and finite memory/fuel failure before and during a live run. Profile-specific parity is execution evidence, not W01–W11 completion or host acceptance.

## Runnable retry-and-sleep demo

Use the build above, copy/review the template and approve the exact module digest. Enable the separate [executor] section and select Survivor with at least **3 virtual units and 3 retained slots**: the third known-zero settlement still needs a fresh one-unit reservation before its explicit release. Create two distinct operator-owned **empty** directories; do not run init/init-world first for this alternative entrypoint.

```sh
cargo +1.86.0 build --locked -p raiv-agent-host --bin raiv-agent
# In raiv-agent.local.toml: approve the built module, enable [executor], review IDs/funding.
mkdir -p apps/raiv-agent/var/blocks apps/raiv-agent/var/world
target/debug/raiv-agent run-scenario retry-and-sleep --inference scripted --config apps/raiv-agent/raiv-agent.local.toml
target/debug/raiv-agent status --config apps/raiv-agent/raiv-agent.local.toml
target/debug/raiv-agent world-status --config apps/raiv-agent/raiv-agent.local.toml
```

The command explicitly authorizes a **fixed native scripted administrator** for this bounded recipe; it is not a status-derived policy approval or authenticated external Actor. It runs only on empty namespaces and approved configured module/profile/world/funding. Already populated/stray/same-directory data never triggers repair, reset, replay or reseed.

Five actual-WASM accepted boundaries perform: organic patch → trade; installed Failed Receipt → patched retry trade; installed Succeeded Receipt → patch/set_wake; installed Succeeded Receipt → patch/natural no-call Finish; second ActorWake → all-skipped patch/natural no-call Finish. These are fixed mock-provider replies, not native Guest runtime decisions. The guest constructs context, interprets patches and owns every boundary/tool choice. Native world separately settles **1/1/0 virtual units** for the three exact accepted Commands, recording one intent and immutable Receipt each. The third zero releases only its synthetic reservation, not Core budgets.

Between checkpoints the runner drops all VM/native session/book/FD instances. It exports/acknowledges pins only from already approved known-accepted RAM owners, explicitly approves that next bounded source as the recipe's administrator and freshly adopts the exact pair; it never guesses approval from untrusted current files. The two final Finish/wake boundaries leave the primary ledger/balance bytes unchanged; the new wake starts a new Episode while accepted Memory survives. Native local results are DATA/provenance only for this synthetic unit ledger, not real market/portfolio/wake pricing or executor authenticity.

Output lists five roots/revisions, world digests/balances and fresh-adoption counts. All accepted files remain in the selected directories. The independent [demo scenario oracle](../../fixtures/format/demo_scenario.json) matches complete input/Memory/Boundary/Episode/State bytes for the template IDs; different approved agent/world IDs legitimately change hashes. Re-running the command refuses existing namespaces. A failure after initialization/acceptance/settlement may leave a durable partial checkpoint or uncertain attempt: stop and use explicit independent pinned inspection/recovery, never re-run initialization to manufacture completion.

`sleep` here is **scripted tick DATA** followed by a native test ActorWake, not a real timer, external wake backend or provider/core-store-loss availability proof. A fresh native/VM instance in one process is not process-crash/power-loss/finality evidence; owned-child SIGKILL and conservative unknown gates have separate tests. For manually reviewed individual operations, use the commands below.

## Headless scripted init/status

The supported configuration is the closed offline subset in [raiv-agent.scripted.toml](raiv-agent.scripted.toml), not the full target application schema. Profile metadata comes from the guest-owned `scripted_definition` constructor and exactly matches independent fixtures/compiled registrations; the host does not call native `Guest::step` to imitate sandbox execution. TOML0.8.23 parsing is host-only/default-disabled; Core/guest stay no_std.

Build the real guest above and the native executable. Copy the template beside it, review the exact module and replace the intentionally invalid zero `module_digest` with its approved SHA-256. Relative module/storage paths resolve against the config file's parent, not the invocation directory. A hash alone is not approval: issuing `init` is explicit local operator administration.

```sh
cargo +1.86.0 build --locked -p raiv-agent-host --bin raiv-agent
cp apps/raiv-agent/raiv-agent.scripted.toml apps/raiv-agent/raiv-agent.local.toml
sha256sum target/wasm32v1-none/release/raiv_agent.wasm
# Edit module_digest in raiv-agent.local.toml after approving this exact module.
mkdir -p apps/raiv-agent/var/blocks
target/debug/raiv-agent init --config apps/raiv-agent/raiv-agent.local.toml
target/debug/raiv-agent status --config apps/raiv-agent/raiv-agent.local.toml
```

Configuration requires exact schema/state-format/selectors, profile, bytes32 agent ID and module digest; unknown/duplicate fields, profile aliases, live protocol fields and credential values reject in this scripted subset before bootstrap; the separate closed live bootstrap shape below is supported. Config is capped at16KiB and module bytes at8MiB. The directory must already be operator-owned. Init invokes the real import-free WASM guest, restores its exact selected genesis, approves it through Registry and writes the guarded local baseline. It refuses an existing manifest even if identical: never replace/repair/reseed an existing namespace by initialization.

Status performs bounded manifest/indexed-closure inspection, checks configuration agreement and reports root/revision/profile/phase/current Episode/admission counts/readable local bytes. It instantiates no VM, infers nothing, approves no current pin and issues no admission or execution capability. Budget/lifetime spending is explicitly DATA-only/unproven; readiness is indexed local readability, not replication/funding/finality. A missing/corrupt manifest or block, profile/identity/module mismatch or nonempty uninitialized namespace is an error, never fallback genesis. Init/status never initialize a world implicitly. The scripted runner and selected live demo are documented separately; full W06/W11 remain open.

## Natural completion transition

Complete validated replies without tool calls select intrinsic guest/Core Finish, including empty text. Reference Veto tools are veto/pass; Survivor tools are patch_memory/trade/set_wake. There is no no_op tool, registered Finish capability or special strict-empty-function sampling. Local calls Continue and external calls Yield; patches alone do not commit. Native normalization admits only normal stop/no-calls or complete tool-call termination/actual-calls; malformed/refused/truncated/contradictory replies and SSE without DONE/successful EOF reject. A tool-free request omits tools/tool_choice/parallel_tool_calls. Atomically staged Memory, closed Episode, typed candidate and separate trusted acceptance are unchanged. Scripted/JSON/SSE/native/actual-WASM/Controller/fresh-source evidence passes offline. Changed Definition/module requires new explicit approval and fresh namespace, never migration/rebind of old accepted or uncertain sources.

## Live-provider setup and bootstrap

[raiv-agent.opencode.toml](raiv-agent.opencode.toml) is the separate closed NEW live-demo configuration for bootstrap plus bounded text-only HTTPS wake/resume. Build the current guest with live-profile support, copy/review the template beside it, approve its exact module digest, and create distinct operator-owned live-blocks/live-world directories. The deliberately invalid zero module pin must be replaced; changing model/base URL/mandate creates a different Definition, not an update to an existing accepted agent.

```sh
cp apps/raiv-agent/raiv-agent.opencode.toml apps/raiv-agent/raiv-agent.online.local.toml
# For a NEW demo, add reasoning_effort = "high" inside [inference] BEFORE init.
# Independently approve/review guest, module_digest, agent identity, mandate and resource caps.
mkdir -p apps/raiv-agent/var/live-blocks apps/raiv-agent/var/live-world
target/debug/raiv-agent init --config apps/raiv-agent/raiv-agent.online.local.toml
target/debug/raiv-agent status --config apps/raiv-agent/raiv-agent.online.local.toml
```

The live shape requires explicit agent mandate and native inference protocol/base_url/model/api_key_env/stream plus positive integer max_tokens, request/response/header/timeout_ms/SSE event/count caps. Request/response are capped at8MiB; config remains16KiB. Only the selected NAME OPENCODE_API_KEY is accepted—no raw key field or value, credential lookup, TLS/request or catalogue lookup occurs during parsing/init/status. ProviderConfig.validate performs data-only protocol/HTTPS/model/bounds validation; guest-owned opencode_definition supplies exact new Definition metadata. The canonical protocol is openai-chat-completions; historical openai-responses remains an explicit Chat Completions alias, not actual Responses support. Omitting reasoning_effort selects the current disabled-thinking profile; tool/policy changes still require a new Definition. To select the new exact deepseek-v4.1-flash profile, add reasoning_effort = "low" or "high" within the existing [inference] table; off/max/other types, duplicate/unknown fields and enabled effort for another model refuse. This is a NEW Definition/module approval and fresh namespace, never a toggle on saved or uncertain state. All profiles retain synthetic one-unit quotes, not authenticated model billing. Historical bounded live high evidence belongs to its originally approved module/Definition; current natural-completion high has a separate fresh bounded live witness; low remains offline-validated only. Broad model compliance is not implied.

Init executes actual Wasmi, verifies the selected guest output and registers/publishes an empty guarded revision-zero baseline. Status reports mode=live-profile-data and does not adopt, approve currentness or create capability. Neither command repairs/reseeds/rebinds existing storage. Independent init-world/world-status work through the same native world owners and separately approved main source below; the world is still synthetic. Scripted wake/resume/run-scenario are explicitly rejected for a live config before attempts or fixture reads, so mocked replies cannot masquerade as selected external inference. The live wake/resume driver below is now wired; broader provider compatibility and operational interrupt/terminal evidence remain in [BACKLOG](../../BACKLOG.md). Bootstrap itself proves no external inference.

## Bounded native live wake/resume

For live config, use the SAME explicit event/grant/main/world pin approval described for scripted wake/resume, but OMIT --replies. Do not reuse accepted or attempted event/grant IDs or infer pin currentness from status. The process looks up only OPENCODE_API_KEY when the approved live advancement has passed data/source/phase/ledger/freshness checks; provide its value through the operator's existing environment, never config/Definition/guest/input/files/logs.

```sh
# Native operator separately reviews the whole event, grant and exact main/world pins.
target/debug/raiv-agent wake --event approved-event.cbor --grant approved-grant.cbor --pin approved-main.toml --world-pin "$APPROVED_WORLD_DIGEST" --config apps/raiv-agent/raiv-agent.online.local.toml
# After separate execute-pending installs its exact synthetic terminal Receipt:
target/debug/raiv-agent resume --receipt approved-result-input.cbor --grant approved-result-grant.cbor --pin approved-next-main.toml --world-pin "$APPROVED_NEXT_WORLD_DIGEST" --config apps/raiv-agent/raiv-agent.online.local.toml
```

Native RAM admission precedes credential preparation; missing/invalid credential or known-key echo in the exact Definition/input fails before durable reservation/attempt and exposes no request. Reserved/input/Attempted/Launched are durable before the first guest request reaches HTTP. The driver sends only exact mapped validated guest context/tools, checks the explicitly selected thinking mode, routes replies via originating RequestTicket and retains one actual VM/Activation through patches/corrections until exactly one verified boundary. Enabled thinking constructs one bounded native ReasoningSession per Run. Only complete mode-checked replies successfully routed into a Pending VM enter its exact assistant-turn continuation; a complete zero-call final may omit reasoning_content because it needs no continuation, the final reply is not retained, and the carrier is dropped before boundary sealing/acceptance or on failure. Tool replies still require present reasoning_content (including explicit empty text); no missing thought is fabricated. Fresh wake/resume/interactive calls cannot restore it, including within the same Episode. Per-record/aggregate storage and combined request bounds refuse rather than truncate; a failure leaves Launched/old accepted root, never a retryable or implicit Finish. Neither carrier data nor raw reasoning is routed to guest, Memory, disk or UI. Finite captured Core model/corrective/unit caps apply independently of max_tokens/time/wire caps. No HTTP retry/pooling/redirect/ambient proxy, guessed Finish, effects or pin auto-approval is added.

HTTP/EOF/normalization/model/request-mapping/mode/guest failures after launch terminate/discard the VM and keep unresolved Launched/attempt1 at the old accepted root. Provider error details are withheld. No refund/new event/relaunch is permitted to manufacture completion. Known accepted boundary output is mode=live-accepted-boundary with root/revision/phase/reply count and unchanged world balance/ledger; counts are not authentic provider expense. execute-pending remains separate and synthetic, and resume only admits the exact installed current Terminal Receipt; phase/grant/Receipt/pin conflicts refuse before inference. A provider tool request or accepted Command alone never authorizes execution.

Offline native provider-port/JSON/SSE faults and actual-WASM fresh-adoption/Memory/ledger continuation prove composition, not actual authenticated TLS, server-specific thinking compatibility or live model behavior. Historical high-reasoning evidence belongs to its prior approved profile; the current natural-completion high witness below has separately passed; these offline fault witnesses themselves use no credentials/provider inference.

## Interactive terminal (native local-test approval)

After NEW live bootstrap/world provisioning and independent main/world source selection, copy/review the [finite interactive grant plan](raiv-agent.interactive-grants.toml). All five fields are canonical unsigned decimal text; max_events is1..1024, unit/model fields positive, actions may be zero. It approves a per-event envelope and per-invocation event count, not genuine pricing/lifetime funding/authenticated external Actor rights. Starting or reopening the command is explicit native operator approval; new process invocation does not establish cumulative funding.

```sh
# Independently approve starting main/world pins, configured program/mandate and finite plan.
target/debug/raiv-agent interactive --grants apps/raiv-agent/raiv-agent.interactive-grants.toml --pin approved-main.toml --world-pin "$APPROVED_WORLD_DIGEST" --config apps/raiv-agent/raiv-agent.online.local.toml
```

TTY stdin/stdout is required. Exactly two regions show bounded RAM history and prompt; source tags remain visible during wrapping/scrolling. Enter approves only the current prompt as a fresh ActorWake under the selected plan and source, using native Linux random event/admission IDs and a synthetic source-revision ordinal (not an external clock/finality proof). Ctrl+J/Shift+Enter multiline, Ctrl+U clear, grapheme caret/delete, PageUp/Down scroll, Esc/Ctrl+C exit. Paste/repeat/pending/disabled/too-small states never auto-admit. No temporary event/grant file or UI transcript is created: the native owner persists the approved protocol Input only through existing Reserved/Attempted/Launched publication. UI history is never reconstructed into model context; guest alone projects context/Memory/history.

After each known boundary, only already approved registered RAM/book/world owners can export an exact paired continuation pin. The next Enter explicitly approves that known source, never a head discovered from untrusted files. Last nonempty complete model text/status are ephemeral presentation; an empty Finish does not erase earlier text from the same Activation. If the provider supplied no text at all, native status states that explicitly—acceptance alone is not a textual answer. A known accepted commit with failed/unavailable next pin displays accepted + blocked—not failed/retryable—and consumed data stays accepted. Finite plan exhaustion, AwaitAction, inference/native refusal or uncertainty disables submit. Pending Command is not effect permission: use separate execute-pending and exact installed-ledger resume, then reopen interactive only at independently approved AwaitActor. Startup rejects unresolved admissions, pending phases, wrong/stale config/pins/world before credentials/inference.

Transport is synchronous and finite, so exit/scroll does not interrupt an in-flight decision immediately; Esc/Ctrl+C is handled after its bounded decision. Typing/Enter queued during inference is discarded rather than silently becoming another approved event; queued-input overflow blocks. External kill/IO failure requires independent checkpoint inspection, never automatic replay/refund/relaunch. These limitations are not asynchronous responsiveness or every signal/terminal failure proof.

Offline actual-WASM controller patch/Finish, fresh approved/UI continuation, same sparse Memory, no transcript/effect replay, blocked unknown/AwaitAction and accepted-without-next-pin witnesses pass. Ready-fenced owned PTY startup/ESC exit and non-TTY refusal make no admission or provider call; historical high-reasoning PTY success belongs to its prior approved profile; the current natural-completion high witness has separately passed. A physical-terminal usability/async-interrupt review remains distinct from TestBackend/owned-PTY evidence.

## Go routing and live smoke

OpenCode Go requires a native per-conversation routing header: **x-opencode-session**. The application now binds this once to the approved event ID before exposure and keeps it stable across every inference turn of that retained Activation. It is routing DATA, not a credential, RequestTicket, persistent transcript or acceptance authority. Generic users of ChatCompletionsClient connecting to Go must bind a native session with with_session_id before issuing requests. The bound label is limited to128 ASCII alphanumeric/dash/underscore bytes, nonempty, cannot be rebound and cannot echo the selected key.

Historical Go/high HTTPS/SSE/actual-WASM/owned-PTY semantic continuation passed on its original explicit-finish profile. This remains valid evidence for that artifact, not a live test of the new no-tool completion profile. Current natural-completion metadata/module has a separate fresh Go/high owned-PTY witness after fixing an over-strict terminal reasoning guard: two accepted semantic tasks and four total calls. The observed success included reasoning on final replies; omission is separately covered by offline regressions. No blind repeat, rebinding or retroactive success is permitted. Detailed prior results and retained namespace provenance are in [CHANGELOG](../../CHANGELOG.md).

## Opt-in live interactive behavioral witness

[test-live-interactive.py](test-live-interactive.py) exercises an owned120x30 PTY and the real native interactive controller. It is excluded from ordinary/CI/module selections. Supply the independently approved reasoning-capable guest via RAIV_GUEST_WASM and the selected credential through the existing OPENCODE_API_KEY environment; never put its value in config/files/logs. Current natural-completion selectors are NEW; high has a passing fresh bounded witness, while disabled modes have offline/filter evidence only. They do not re-run old explicitly finished profiles:

```sh
# Explicit approval of NEW Definition/module/sources/events and this finite envelope.
# RAIV_GUEST_WASM must already name the independently approved current guest.
mkdir -p target/raiv-disk-test-root
RAIV_LIVE_SMOKE=opencode-go/deepseek-v4.1-flash/natural-recall-high \
  TMPDIR="$PWD/target/raiv-disk-test-root" \
  python3 apps/raiv-agent/test-live-interactive.py --with-high-reasoning
```

The high mode allows3calls/event,6total,256 output tokens and20s/call, actions0; its170s process deadline and1MiB PTY output cap do not widen provider/Core budgets. Nominal patch/final uses2calls/event; a corrective slot is available only for invalid tool decisions, not mandatory completion. Current disabled selectors are /natural-recall (2calls/event) and /natural-recall-correction (3calls/event). Old attempted/accepted profiles/outcomes remain in their original namespaces, not silently upgraded by current selectors. Do not repeat unchanged sampling to manufacture success.

The NEW task-specific Definition asks first to store a nickname with patch_memory then Finish with a normal complete no-call reply, and in a fresh independently pinned session to copy accepted intents.nickname into tactics.recalled_nickname then Finish. Neither the second prompt nor mandate contains the nickname value. Exact semantic Memory, usage, two Consumed inputs/completed Episodes and unchanged world are checked; a no-op Memory write or optional assistant text alone cannot satisfy the semantic witness. Calls to a removed no_op are unknown-tool errors, never successful completion.

The harness submits bracketed paste/Enter only after a fresh editable frame and Esc only after finite-plan blocking. Raw PTY/provider buffers remain RAM-only. Allowlisted output contains proof markers, known control labels, argument sizes/empty-map flags, text/reasoning presence/byte counts, post counts and closed native HTTP error classes—not raw reasoning/text/arguments/headers/keys. Run python3 apps/raiv-agent/test-live-interactive.py --check-filter for offline grammar/selector/privacy checks without credentials/TTY/network.

Every outcome namespace is retained. On refusal/uncertainty/infrastructure failure, stop and inspect independently; process-group termination applies only to harness-owned children and never supplies retry/refund permission. Earlier Launched attempts must not be cleared, repaired, reseeded, rebound or relaunched after a transport/profile change. Status and hashes remain DATA, not currentness/adoption/dispatch authority.

## Explicit scripted world provisioning

A world must be provisioned **before** accepting the first action: bootstrap refuses post-genesis sources rather than inventing a world that forgot prior spending. Enable the optional closed `[executor]` table in the template, selecting `scripted-unit-world`, separate existing directory, bytes32 world ID, canonical decimal-text virtual units and1..1024 ledger capacity. Existing init/status configs remain valid without this table; world commands require it.

`init-world` is an explicit local administrative command, not a guest/provider tool. It requires a separately selected4KiB-capped pin file with exactly three text fields. Agent/program/Definition/domain/default VM resources come from the approved configuration. Root/revision/whole-manifest digest must match the independently selected current main source; reading or hashing a file alone never supplies approval. No automatic source selection, token deserialization, fresh funding on recovery or dispatch permission.

```toml
root = "<independently approved genesis CID>"
revision = "0"
manifest_digest = "<approved exact main-manifest SHA-256, 64 lowercase hex>"
```

After reviewing/selecting this source, save the pin file (for example beside the config), create the independent operator-owned world directory and invoke:

```sh
mkdir -p apps/raiv-agent/var/world
target/debug/raiv-agent init-world --pin apps/raiv-agent/approved-main.pin.toml --config apps/raiv-agent/raiv-agent.local.toml
target/debug/raiv-agent world-status --config apps/raiv-agent/raiv-agent.local.toml
```

Bootstrap uses existing explicit pinned main adoption and ScriptedWorld genesis ports, preserving main blocks/metadata while creating the one-file virtual-unit ledger. Repeating the exact approved empty baseline acknowledges identical bytes, never adds units or clears rows. Wrong/stale pins, changed seed/scope, nonempty/missing/corrupt/same-directory namespaces and late valid post-genesis sources refuse rather than repair/reseed. Printed world digest remains DATA for later separately approved paired-pin selection.

World status checks selected world/program/agent/initial funding and bounded ledger, and reports its digest, virtual balances and row count without adoption, inference, reservation, submission or effects. It can inspect the independently held primary even if Core-store bytes are absent; this is **not** after-Core-loss reconciliation or permission to continue. The backend remains a synthetic unit counter, not real money, trading/portfolio/wake semantics or full W06.

## Wake to one accepted boundary

`wake` is explicit **local native administration** of an ActorWake snapshot and separate approved grant, not external Actor authentication. The config must select an already provisioned independent world; choose current main pin and world digest independently. Old pins or decoding/importing current bytes never approve a source automatically.

```sh
target/debug/raiv-agent wake --event event.cbor --grant approved-grant.cbor --replies replies.cbor --pin approved-main.pin.toml --world-pin <approved-world-digest-hex> --config apps/raiv-agent/raiv-agent.local.toml
```

These operands are canonical bounded CBOR DATA, not JSON or a live provider:

- Event: complete typed ActivationInput with ActorWake/null Receipt and exact native-approved observation/grant.
- Grant: separately reviewed BudgetAdmission; every field must equal the event grant.
- Replies: definite CBOR List of ByteStrings, each containing a complete normalizer-v1 ModelReply with fresh-VM request IDs1..N. This is an explicit mock inference provider, never a second host decision engine. Whole-script/cardinality/per-frame/wire/schema preflight happens before native admission/attempt; semantic selection/corrections/budgets remain guest-owned.

For a reproducible **Survivor-only first wake**, the independent fixture encoder can prepare sample DATA from repository-root (review the event/grant before approving their use):

```sh
python3 - <<'PY'
import json, pathlib, sys
sys.path.insert(0, "fixtures")
from generate_lifecycle import cbor
from generate_profiles import decode
step = json.loads(pathlib.Path("fixtures/format/lifecycle.json").read_text())["steps"][0]
event = bytes.fromhex(step["input"]["hex"])
grant = decode(event)["budget_admission"]
usage = dict(model_calls=0, local_tool_calls=0, input_tokens=None, output_tokens=None, billed_units=None)
calls = [("patch_memory", step["patch"]), ("trade", step["arguments"])]
frames = [{"$bytes": cbor(dict(request_id=i+1, text="", usage=usage,
          calls=[dict(call_id="scripted", tool_id=tool, arguments=args)])).hex()}
          for i, (tool, args) in enumerate(calls)]
pathlib.Path("event.cbor").write_bytes(event)
pathlib.Path("approved-grant.cbor").write_bytes(cbor(grant))
pathlib.Path("replies.cbor").write_bytes(cbor(frames))
PY
```

The native command explicitly adopts selected main/world pins, admits the exact ActorWake/grant, persists Reserved/Input, Attempted before VM and Launched before request exposure, then feeds the same actual VM through bounded replies. It commits exactly one verified Command/Finish boundary using accept_local. Full organic feedback/context stays guest-owned; no transcript or patch report is checkpointed. The primary world remains unchanged: **wake does not reserve/debit/submit/execute the accepted Command**.

An empty/malformed/wrong-ID script or grant mismatch rejects before an attempt. A semantically unsealed/exhausted script, failure after launch or extra unused replies after early sealing leaves an unresolved durable attempt; no fabricated Finish, refund or automatic relaunch. After known acceptance, output reports the new root/revision/phase without fallible pin export or another inference; independently select new paired pins before the next mutating command. Busy AwaitAction and consumed/recovered IDs cannot become another ActorWake.

## Separate synthetic pending execution

`execute-pending` does not infer or accept another Core boundary. It selects the exact already accepted pending Command from the approved main snapshot, adopts an independently approved paired world, and applies only the **scripted virtual-unit** result/Receipt transaction. This is not real trade/veto/wake semantics or external effect authority.

```sh
target/debug/raiv-agent status --config apps/raiv-agent/raiv-agent.local.toml
target/debug/raiv-agent execute-pending --plan approved-plan.toml --pin approved-main.pin.toml --world-pin <approved-current-world-digest> --config apps/raiv-agent/raiv-agent.local.toml
```

Status now exposes `pending_action_id`, domain-bound `command_digest` and `pending_tool` as DATA; hashes do not approve execution. Independently review/select that exact Command and write the native plan (16KiB cap, exact five text fields):

```toml
action_id = "<approved pending ID, 64 lowercase hex>"
command_digest = "<approved exact Command domain digest, 64 lowercase hex>"
outcome = "failed" # succeeded / failed / not_executed only
debit = "1" # known synthetic 0 or 1; not_executed requires 0
result_hex = "a0" # complete canonical model CBOR, e.g. empty Map; not a JSON string
```

The complete Receipt, result/model bounds, accepted ID/digest, world seed/scope and existing status/conflict are checked before Ready/submission. Only Ready can record a new one-unit Submitted reservation; Terminal can acknowledge an **identical** installed result with no second begin/debit. Submitted refuses another command invocation: it is an unresolved intent, not permission to re-submit, infer zero/refund, fabricate success or resolve an uncertain external effect.

Settlement atomically stores known debit/release and exact immutable Receipt in the primary world. Known zero releases its virtual reservation, never Core inference/tool budgets. Main checkpoint/accepted Command remain unchanged in AwaitAction. Output gives the exact receipt hex from pre-staged DATA and known terminal balances, without a fallible post-commit pin export; independently select the current world digest again before later operations. Ledger-bound resume is a separate command below and is composed by the retry-and-sleep runner above.

## Resume from the installed terminal ledger

```sh
target/debug/raiv-agent resume --receipt action-result-input.cbor --grant approved-result-grant.cbor --replies result-replies.cbor --pin approved-main.pin.toml --world-pin <approved-current-terminal-world-digest> --config apps/raiv-agent/raiv-agent.local.toml
```

In this supported offline subset, `--receipt` names the **whole canonical ActivationInput** for ActionResult (fresh event/position/observation/grant plus exact terminal Receipt), not bare Receipt hex or JSON. Explicit command selection fixes the required cause; an ActorWake cannot enter resume. The separately reviewed BudgetAdmission must exactly equal the input grant. Reply-byte script and source/pin caps are the same as wake.

The native application adopts the independently approved current pair, requires an already installed Terminal Receipt for the exact current pending action, and compares all receipt bytes **before** obtaining fresh slot correlation. Ready/Submitted, a matching-ID but changed result/outcome/cost, historical/foreign/imported DATA or missing world cannot create Ready or authorize admission. Only `ScriptedWorld::admit_action_result` selects and approves its immutable installed Receipt; printed hex never grants execution provenance by itself.

The shared native advancement owner (`host/src/app/advance.rs`) then persists exact Input/Reserved/Attempted/Launched, feeds a fresh actual guest through its retained Activation and commits one verified boundary. Core owns ActionResult same-Episode accounting/Memory/context: a reply may Finish explicitly or Yield another action, but the host never chooses this outcome. Previous boundaries/Episode identity and accepted model/action counts survive; no budget reset, forgotten Memory, transcript restoration or accepted effect replay.

Resume leaves primary world/Receipt/balance bytes unchanged. Repeating consumed/recovered IDs, replaying a prior Receipt after a new action, or invoking ActionResult at AwaitActor refuses without a new admission or boundary. Failed/uncertain launched runs remain blocked by existing gates, never an automatic relaunch. Current local unit-ledger provenance is not authentic external execution, real pricing, lifetime funding or full W06; the fixed runner above composes this path without changing those authority boundaries.

## Linux block-store checks

Disk tests use isolated child directories under `std::env::temp_dir()`. `/tmp` may be tmpfs; choose an actual filesystem with `TMPDIR` when evaluating local block persistence. The tested syscall/reopen boundary includes SIGKILL of owned test subprocesses at metadata-file write/sync/rename stages, but does not simulate power loss or complete world/Receipt atomicity. Owned-child process tests are opt-in and must run separately from other FD/lock tests. A controlled fork/pipe-fence counterexample proves that a child retains an inherited directory flock after the parent's store is dropped, until the child closes/exits; CLOEXEC does not prevent this pre-exec window. This explains a concrete interference mechanism for the previously observed parallel WouldBlock, not a trace proving that exact incident. Ordinary native/disk and module suites now run in parallel without spawning these children; execute the explicit isolated process selection as well, not merely the default tests. All three fixed metadata slots (main, executor DATA, scripted world) run their five interruption stages separately from ordinary/module FD tests.

```sh
mkdir -p target/raiv-disk-test-root
stat -f -c '%T' target/raiv-disk-test-root
TMPDIR="$PWD/target/raiv-disk-test-root" cargo +1.86.0 test --locked -p raiv-agent-host disk::
TMPDIR="$PWD/target/raiv-disk-test-root" cargo +1.86.0 test --locked -p raiv-agent-host --lib disk::manifest::tests::process_ -- --ignored --test-threads=1
TMPDIR="$PWD/target/raiv-disk-test-root" RAIV_GUEST_WASM="$PWD/target/wasm32v1-none/release/raiv-agent.wasm" cargo +1.86.0 test --locked -p raiv-agent-host activation::disk -- --ignored
```

## Current ABI

Version 1 canonical CBOR uses required `abi_version`, `type`, `payload` fields. `initialize` accepts exact `agent_id` bytes32, `definition` bytes and explicit `memory` bytes/null; `initialized` emits fixed binary root and complete `{cid, body}` blocks. Validation/error emits `error` with a bounded static UTF-8 `code`. Records stay opaque exact DRISL bytes; no state/hash/version changes or generic RPC registry.

Guest validation bounds the whole frame to 8,388,608 bytes, depth 64 and 65,536 aggregate container entries before ownership/schema processing. Existing reference caller and Definition limits are not widened. `activate` validates bounded exact closure/input data and prepares a live Activation using compiled registrations, emitting `infer`. Matching `inference_error` terminates without a candidate; wrong request IDs and malformed frames preserve the pending Activation. `inference_reply` checks outer and normalized-body request IDs, advances registered Core RAM work, yields `infer` on Continue/correction and emits `boundary` only after successful sealing and complete candidate validation. Malformed outer frames preserve pending state; a matching malformed normalized reply is fatal. Complete zero-call replies select intrinsic Finish; invalid calls and failed inference do not. Host transport enforces module/frame byte caps, typed exports, pointer arithmetic/bounds/non-overlap, finite VM limits and borrowed closed output-schema/integrity preflight; Run now routes infer/boundary/error phases with opaque originating-instance/run/request tickets and verifies source-relative closure/single-boundary joins before retaining a sealed candidate snapshot. A separate RAM-only local Registry now explicitly registers genesis and approves the program/Definition/domain pin; a RAM-only AdmissionBook now pins native ActorWake/grant approval and a single run launch. SealedRun handoff and accept_ram now couple full volatile readback with revision/root CAS and consumed-event metadata; durable policy/provenance/recovery and external effects remain open.

The same instance owns the initial live Activation and its pending request; input buffers can be freed between steps. After a live run starts, another activate/initialize is rejected, including after terminal failure; fresh-instance recovery must start from externally accepted data. Request IDs are VM-scoped; the private zero Core instance label is not global uniqueness or authenticated routing. Native Run tickets now check originating-instance/run correlation before guest advancement; durable registry policy, external provenance, admission recovery/consumption and trusted lifecycle acceptance/effects remain open. Sealed/failed instances reject further replies/activate; the host copies candidate bytes for persistence retry and may destroy/reinstantiate from externally accepted checkpoints.

The scripted compiled profiles match the synthetic Veto/Survivor Definition descriptors and `fixture-model`/`scripted` parameters exactly, including demo authority labels 0x11/0x22/0x33/0x44. The closed OpenCode live profiles bind endpoint/model/selected thinking and effort with the same compiled tools; native bootstrap creates a separate Definition and cannot rebind prior data. They are not production authority. All execution quotes remain fixed synthetic reference units, not authentic provider/backend prices. Unsupported live parameter shapes are rejected; native bounded HTTPS advancement is wired, but broad provider/model/option compatibility remains unproven. Yield planners preserve arbitrary bounded model-map arguments permitted by these synthetic schemas; they execute nothing and establish no successful effect. `boundary.candidate` carries the eight closed Core contract fields, fixed binary roots, exact CID-sorted source-relative new-block delta and typed AwaitActor/AwaitAction disposition. It contains no trajectory/provider IDs/text or previous-root state link. Broader native/UI operational evidence, durable registry/authority/availability manifest policy, trusted fresh run/input admission, authenticated quote/admission provenance, durable acceptance manifest/recovery/retention/readiness/execution remain open; bounded native normalization/SSE/HTTPS and exact guest request mapping are implemented library ports; the selected live evidence above does not close these broader operational/authority obligations.
