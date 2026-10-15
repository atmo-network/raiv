# Raiv 0.0.2

## Specification of the portable Rust library and isolated WebAssembly reference agent

**Date:** 15 October 2026. **Revision:** 0.0.2 — provider profiles, readable Memory and pinned local sessions on the isolated WASM reference application; 0.0.1 (12 October 2026) was the first release. **Status:** normative implementation specification, not an implementation report; progress and evidence live in the changelog and backlog.
**Pronunciation:** /reɪv/, like *rave*.
**Components:** `raiv` — portable library; `raiv-agent` — native host + isolated guest; `raiv-agent.wasm` — compiled agent logic.
**State format:** `raiv-state/1`, DASL CID + DRISL; CAR is host-side transport.

> Raiv preserves an agent's behavioral continuity across short executions. A trusted execution host provides execution and canonical acceptance of transitions; a blockchain anchors the root; external storage retains the actual data.

**MUST / MUST NOT** denote requirements; **SHOULD** denotes a recommendation whose deviation must be documented; **MAY** denotes an option. Requirement owners: **core** (library), **host** (accepting environment), **adapter** (an integration module), **reference application** (`raiv-agent`, with guest and native host responsibilities stated separately).

This is a self-contained specification for the first Raiv version. It does not implement a JAM service or perform real financial operations. The `raiv` crate and reference application implement it; this document neither reports their progress nor treats a passing build as conformance.

## Contents

1. Decisions in revision 0.0.2
2. Trust and guarantee model
3. Deliverables and scope
4. Terminology and invariants
5. Lifecycle
6. Memory, facts, and history
7. RAM loop and model-facing tools
8. The `raiv-state/1` profile: DASL CID and DRISL
9. State data
10. Commands, receipts, and stable IDs
11. On-chain anchoring and agent registration
12. Preparation, acceptance, and delivery
13. Availability of actual data
14. CAR and the IPFS adapter
15. Library API and four external ports
16. Rust, dependencies, and WebAssembly integration
17. Errors and resource limits
18. Reference agent outside a blockchain
19. Acceptance scenarios
20. Mandatory test matrix
21. Implementation validation and compatibility boundaries
22. Implementation order and completion criteria
23. Final contract
24. Sources and verification status

## 1. Decisions in revision 0.0.2

Raiv separates three boundaries: execution trust, on-chain anchoring, and off-chain availability.

**Organic agent runtime:** agent-facing semantic editing is cooperative, not an all-or-nothing database transaction interface. The runtime MUST preserve useful independently admissible semantic work despite recoverable mistakes, provide explicit actionable feedback, and let the agent continue when the necessary work is already done. Forgiveness MUST NOT silently invent intent, expand authority, weaken resource bounds, or reinterpret uncertain external effects as success or failure. The concrete plane-level memory-patch contract is defined in §6.2; canonical persistence and authorized external-effect acceptance remain separate safety boundaries.

| Area | Decision |
| --- | --- |
| Execution | The reference agent runs in isolated Wasmi 2.x without WASI; the native host remains trusted, not cryptographically proven |
| Canonical state | In the blockchain profile, the accepting network stores the current revision and root digest |
| Content | A DRISL graph in external storage; IPFS is a target adapter, not an embedded node |
| Checkpoint | At an authorized external command or a complete validated no-tool reply (Finish) |
| Continuation | A receipt resumes the same episode; ActorWake after `Finish` starts another |
| Text session | Guest RAM for one activation; ephemeral native terminal history is presentation, not agent memory |
| Persistent memory | Optional sparse `intents` / `strategy` / `tactics` object; one agent memory |
| History | Commands, outcomes, and states grouped up to `Finish` |
| Excluded | Persistent transcript, `response`, `lazy`, reasoning archive, task scheduler |
| Inference | Native OpenAI-compatible Chat Completions adapter with closed provider profiles (§18.3); complete normalized replies enter the guest through a versioned CBOR ABI |
| Presentation | Hosts may render accepted model values as compact JSON for people; presentation never becomes state, context or authority (§8.4) |
| Local sessions | A reference session pins its guest, configuration and grant plan; newer builds never rebind it (§18.6) |
| Portability | Required `no_std + alloc` for core/runtime dependencies; adapters remain outside core |
| Availability | A contract for the entire required graph, including retention, recovery, and failures |

The document version is 0.0.2. Relative to 0.0.1 it adds provider profiles, value presentation, actionable patch feedback, failure categories and local-session pinning; it changes no state format, structural record, hash domain or golden fixture, so 0.0.1 checkpoints remain valid 0.0.2 checkpoints. The first state format is `raiv-state/1`. The sparse `intents` / `strategy` / `tactics` contract below defines the first memory standard, not a migration or a second Memory version. Structural record versions remain 1. Implementations MUST NOT rename stored keys, reinterpret incompatible blocks, or rewrite existing CIDs or Command bindings to make data pass validation. Changing format strings and hash domains changes content, IDs, and roots.

## 2. Trust and guarantee model

### 2.1. Trusted execution is a premise

Raiv's baseline profile assumes that the host enforces execution of an **authorized code version**, integrity of its live continuation, and acceptance of the output of that execution. Locally, this is a trusted native host running the authorized WASM module in Wasmi 2.x. Isolation restricts guest capabilities; it does not prove execution against a malicious host. A distributed deployment may obtain this property from an execution-verifying protocol, such as a JAM-style service environment. This does not mean trusting an arbitrary RPC node or assuming a hardware TEE.

**CORE MUST NOT require its own consensus, commit–reveal, quorum, fraud proofs, or execution ZK proofs.** When the host already supplies execution assurance, a duplicate layer is unnecessary. JAM denotes a target class of hosts; its exact ABI, admission mechanism, and handling of external inputs require a separate integration. Official JAM material separates processing work from accumulating results into state; this specification does not present that as a completed Raiv integration. [S8]

The LLM authors mutable memory. Core does not judge the truth of a strategy or require proof of every semantic conclusion. However, the model does **not** author an arbitrary system checkpoint: Raiv code controls phases, commands, IDs, receipts, links, and serialization.

### 2.2. Responsibility allocation

| Property | Owner |
| --- | --- |
| Execution of authorized code | Trusted host and its protocol |
| Valid actions, schemas, limits, and phases | Core + host authority boundary |
| A single accepted lineage | Acceptance host / blockchain |
| Binding the root to exact bytes | DASL CID and digest verification |
| Retention and timely delivery of every required block | Storage deployment |
| Provenance of observations and receipts | Host / executor |
| Admissible inference source | Selected host policy |
| Behavioral quality | Model, mandate, and environment; not a codec guarantee |

Trust in Raiv execution does not automatically authenticate every external LLM response. The host explicitly selects a trusted source or admits its result as an external input under its policy. Re-verifying model weights is not a core responsibility. Core determinism is relative to **identical admitted inputs and replies**, not identical prompts across different API calls.

### 2.3. Checks that remain mandatory

A separate execution proof is removed, not data-safety checks. Core MUST validate CIDs, canonicality, schemas, bounds, continuation, and the whitelist. The host MUST bind a candidate to an authorized run. It MUST NOT accept an arbitrary storage-node root as if it were the output of a trusted VM.

The blockchain anchors accepted content and transition order without necessarily retaining every DRISL block. Retaining actual data is therefore a separate operational responsibility even with fully trusted execution. [S1][S5]

## 3. Deliverables and scope

**The `raiv` library:** a pure RAM core, bounded types, memory patches, tool validation, deterministic projection, DRISL checkpoints, restoration, and testable transitions. It is implemented and tested without networking first.

**The `raiv-agent` application:** an isolated Rust `no_std + alloc` guest (`raiv-agent.wasm`, target `wasm32v1-none`) and a native Rust `std` host embedding Wasmi 2.x, without WASI. The guest owns agent behavior, RAM activation, context construction, model-facing tools, decisions, and Raiv state transitions. The host owns inference APIs, storage/CAR, checkpoint acceptance, simulated Executor/Actor, terminal I/O, and WASM lifecycle; it MUST NOT independently implement or override agent decision logic. Ratatui with Crossterm supplies the minimal interactive frontend; headless scripted operation remains mandatory. IPFS is a separate host adapter. Baseline CI needs neither network access nor an API key. Raiv remains a library, not a monolithic agent framework.

**Out of scope:** an IPFS/P2P node or blockchain consensus; a wallet/keys in core; mainnet actions; DAG schedulers, subagents, multi-scope memory, CRDT/fork/merge; shell/browser/filesystem tools; online learning; MASL/RASL/FISL; arbitrary hash/codec registries; automatic migration; a complete JAM adapter. Pi Durable's entire feature set need not be ported: inspiration is limited to preserving continuation before an external effect.

Workspace structure (informative; module names may evolve without changing requirements):

```text
raiv/
  Cargo.toml                   # Workspace; default member: lib
  run.sh                       # Linux interactive launcher for local sessions (§18.6)
  lib/                         # raiv library package: Core, no_std + alloc
  apps/raiv-agent/
    guest/src/                 # Rust, wasm32v1-none, no_std + alloc: ABI, runtime, profiles
    host/src/                  # Native Rust, std: configuration, inference, terminal, disk store,
                               # checkpoint manifest, admission, registry, recovery, simulator
    raiv-agent.*.toml          # Example configurations and grant plans
  fixtures/                    # Conformance vectors, generators and upstream DASL cases
  docs/raiv.spec.en.md
```

The guest links `raiv`; host adapters remain outside it. No intermediate framework crates or general-purpose RPC layer are required.

## 4. Terminology and invariants

**Agent:** one identity, mandate, and canonical memory lineage; not a process or a version CID.
**Episode:** a session from ActorWake to an accepted `Finish`.
**Activation:** the RAM segment of an episode until an external command, `Finish`, or error.
**Boundary:** the persisted input and outcome of an accepted activation.
**Checkpoint:** state for semantic continuation, not a stack or serialized future.
**Command:** an execution instruction; **Receipt:** its terminal outcome.
**CommitCandidate:** an unaccepted root and new blocks.
**Required closure:** every typed block reachable from the root and required for restoration.

| ID | Mandatory invariant |
| --- | --- |
| INV-01 | One scope, one current root, at most one pending command |
| INV-02 | Dispatch only after accepting the checkpoint containing the command |
| INV-03 | The model changes only memory, not read-only facts or control fields |
| INV-04 | Memory is optional; control continuation is mandatory |
| INV-05 | Model replies and local tools do not create individual persistent checkpoints |
| INV-06 | Only an accepted `Finish` closes an episode |
| INV-07 | After a durable boundary, construct a new projection rather than restore the transcript; inference yields retain the live activation |
| INV-08 | Materialize and validate every required input before inference |
| INV-09 | Identical code/definition/admitted inputs/replies produce identical bytes and commands |
| INV-10 | A CID or publication of blocks is not HEAD acceptance |
| INV-11 | Episode budgets survive ActionResult; the host accounts for lost attempts |
| INV-12 | Pending/unknown is not failure and does not authorize blind retry |
| INV-13 | A candidate originates in trusted execution; no separate Raiv proof is required |
| INV-14 | Root, revision, consumed input, and dispatch authorization are accepted coherently |
| INV-15 | Untrusted storage blocks are checked; invalid content never replaces memory |
| INV-16 | Acceptance requires the declared storage policy for the entire required closure |
| INV-17 | Blockchain history is not assumed to provide an indefinitely accessible block archive |
| INV-18 | Restoring an old root neither rolls back the world nor grants execution authority |

## 5. Lifecycle

### 5.1. Two persistent continuations

```rust
pub enum Continuation {
    AwaitActor,
    AwaitAction { action_id: ActionId },
}
```

```text
ActorWake
  -> RAM: inference / patch_memory / inference
  -> command A -> checkpoint -> accept -> dispatch A
  -> receipt A -> RAM: continue the SAME episode
  -> command B -> checkpoint -> accept -> dispatch B
  -> receipt B -> RAM: continue the SAME episode
  -> Finish -> checkpoint -> accept -> close episode
  -> AwaitActor
```

| Phase | Input | Reaction |
| --- | --- | --- |
| AwaitActor | New authorized ActorWake | Start a new episode |
| AwaitAction(A) | Terminal ActionResult(A) | Resume the current episode |
| AwaitAction(A) | ActorWake | Busy; queue/coalesce outside core |
| AwaitAction(A) | ActionResult(B), B ≠ A | Reject |
| AwaitActor | Old ActionResult | Duplicate/stale; do not activate |

`AwaitInference` is not a persistent lifecycle state. A RAM activation MAY yield to the inference port without a durable checkpoint. The reference WASM instance MUST retain that live Activation, including its RAM trajectory and counters, across external inference calls. It ends at a Command, intrinsic `Finish`, or fatal error; a command's receipt arrives in a new prepared packet, not as a native tool-result appended to the old transcript. Neither an inference request nor displayed/provisional text is a durable boundary. A full valid zero-call reply only creates a Finish candidate; authorized acceptance is still required.

### 5.2. Initialization and admission

An authorized host creates genesis: `revision = 0`, `AwaitActor`, `current_episode = null`, `history = []`, the selected Definition, and initial Memory or `null`. Initialization neither invokes the model nor invents an episode. Repeated initialization does not replace an existing agent.

The host pins a particular root, event ID, observation snapshot, budget, and receipt. Recovery uses the same admitted input. It MUST NOT silently reevaluate the event against another snapshot. A refreshed observation after failure requires an explicit admission decision, not an incidental wall-clock read.

### 5.3. External calls and completion

One valid external tool call produces a Command, Boundary, updated Episode manifest, and AgentState/AwaitAction. The Activation is sealed. The command does not execute before acceptance, and no fictitious `prepared` model result is returned. A local validation error is returned to the model without a checkpoint.

A receipt reporting success, failure, or confirmed non-execution resumes the episode. A deliberate retry is a new command, not redelivery of the old one.

A complete validated ModelReply with zero tool calls selects intrinsic `Finish`: commit the final Memory, close the Episode and enter AwaitActor after acceptance. It neither clears Memory, sends an empty extrinsic nor removes the existing schedule. Text may be empty; a separate `no_op` tool is not registered or advertised. Timeout, truncation, refusal, malformed/inconsistent replies and budget exhaustion are failures, never Finish. The native adapter admits only successful complete provider replies (normal stop with no calls, or complete tool-call termination with calls); SSE additionally requires DONE and successful reader EOF. The guest/Core decides Finish from the correlated complete reply; the host MUST NOT manufacture a finish call or turn missing/invalid calls into an empty successful reply.

## 6. Memory, facts, and history

### 6.1. Ownership separation

Definition contains the mandate and policy. Observation contains current world data. Receipt records an outcome. History contains accepted prior states and actions. These are model-read-only but have distinct provenance and instruction priority. An external document does not become a system instruction merely because it is immutable.

The standard mutable memory is a sparse object whose only permitted top-level keys are exactly `intents`, `strategy`, and `tactics`. Human-facing labels are **Intents**, **Strategy**, and **Tactics**. Keys are case-sensitive; singular, pluralized, or capitalized alternatives are not aliases. Every plane is optional; a present plane MUST be an object. Enabled empty memory is `{}`, not disabled memory. Decode, restore, and projection MUST preserve absent planes rather than manufacture empty ones. The veto profile MAY disable memory entirely; `patch_memory` is then absent and structural memory links are null.

- **Intents:** chosen goals, commitments, unfinished tasks, and their completion conditions: what the agent intends to achieve.
- **Strategy:** developed approaches, behavioral rules, reasons, material uncertainties, and revision conditions: how and why the agent chooses a way to act.
- **Tactics:** concrete ways to implement intentions, situational assumptions, and conditional continuations: how to act under current circumstances.

The fixed top-level vocabulary is a **low-cost semantic discipline**: it anchors the agent's organization of goals, approaches, and situational continuations without prescribing their internal representation. The runtime MUST NOT accommodate synonyms by remapping roots, expanding the plane set, or guessing which supported plane an unsupported name meant. Tolerant handling of an unsupported root does not make that root part of the schema.

These labels guide model use, not an entry schema or universal planner. The agent chooses arbitrary internal keys and values within the model-data and resource restrictions. No mandatory `goal`, `rule`, `basis`, ownership, scope, reference-resolution, or semantic cross-entry deletion mechanism is introduced. Identically named entries in different planes are independent.

Mandate, system rules, observations, history, Commands, Receipts, and phase remain outside editable memory. Tactics are semantic notes, not the accepted command queue: deleting them MUST NOT cancel a Command, bypass AwaitAction, or establish success. Runtime records action facts; the model interprets them. Revising an interpretation does not alter a historical receipt. An important rule must survive in current memory before its originating episode leaves the window. There are no additional `response`, `lazy`, `reasoning`, or `transcript` planes; those words MAY occur as ordinary nested keys or notes.

Materialized Memory MUST contain no null at any depth and MUST satisfy the normalization rules below. This restriction belongs to semantic Memory, not DRISL generally: optional runtime fields, observations, Receipts, other tools, and transient error/results may still contain schema-permitted null.

### 6.2. Patching

#### Arguments and merge

The arguments to `patch_memory` are the object patch itself, without an `operations` envelope, operation list, or path syntax:

```json
{
  "intents": {"next": "Inspect the next action receipt before retrying."},
  "strategy": {"obsolete": null},
  "tactics": {"steps": ["Inspect receipt", "Choose retry parameters"]}
}
```

The argument root MUST be an object, including an allowed empty `{}`. Supported plane names are exactly `intents`, `strategy`, and `tactics`; an applicable plane patch value MUST be an object or null. A well-formed bounded request is handled optimistically at **top-level root selection**: unknown/alias plane names are skipped with explicit reasons, rather than invalidating patches under supported roots. This tolerance concerns membership in the supported schema, not whether a supported plane is currently present in Memory, and does not weaken validation of content under selected roots. Unsupported names MUST NOT be materialized or interpreted as aliases. A supported plane absent from current enabled Memory is not unsupported: an object patch may create it, and null deletion remains a successful no-effect operation. Non-object roots, invalid object/null shape or invalid patch content under a supported root, duplicate keys, forbidden model-data encodings, and complete-input resource-limit failures reject the request without mutation. Provider JSON parsing MUST detect duplicate keys before normalization into canonical CBOR; reducing them to last-write-wins is forbidden.

The advertised root schema has `type: "object"`, `additionalProperties: false`, optional properties `intents`, `strategy`, and `tactics`, each accepting object or null, and no required planes. This describes the intended vocabulary, not permission for a generic schema validator to reject an entire bounded request before optimistic plane selection. New provider-facing Definitions SHOULD express each plane's object-or-null as `anyOf: [{type: "object"}, {type: "null"}]` rather than a `type` array, because some models serialize a `type`-array plane as text containing JSON; Definitions created earlier keep their exact schema bytes, since changing them changes the Definition. The runtime validator additionally enforces recursive merge, array-data restrictions, normalization, bounds, and explicit skipped-plane feedback. Tool descriptions, system instructions, and scripted examples MUST distinguish supported input from tolerant handling of recoverable plane errors.

For each field of an object patch:

1. An omitted field preserves its previous value, subject to whole-memory normalization.
2. Null deletes the field and its subtree. A null plane deletes that entire plane. Deleting an absent field succeeds without inventing ancestors.
3. An object recursively merges into the existing object, or into `{}` if the previous value is absent or not an object. Thus objects can replace old scalar/array values; crossing a non-object is not a path-traversal error.
4. A non-null scalar replaces the previous value. An array replaces the whole previous value, never by index, splice, append, or element-deletion operations.

Nested names are literal string keys. Dots, slashes, `[0]`, and `$ref` have no path, index, reference, or dependency semantics. An empty object patch is an empty merge, NOT a request to clear an existing nonempty object. To delete a field, subtree, or plane, supply null at its object-field position.

An array supplied as replacement data under a selected supported root MUST contain no null anywhere, including inside objects or nested arrays. `[null]`, `[{"x":null}]`, and deeper equivalents reject the selected patch **before normalization**, leaving working Memory unchanged. Unsupported roots are not interpreted as patches, but their raw data still obeys complete-input model-data and resource bounds. The runtime MUST NOT salvage arbitrary nested fragments of invalid supported-plane content or remove array positions to guess intent. Null is a deletion marker only in recursively merged object fields; it is not an array placeholder or element-deletion instruction.

#### Normalization and materialization

After merging, normalize the entire draft, including branches not mentioned by the patch:

- Recursively remove every object-valued field whose normalized value is `{}`. Structural emptiness propagates upward; empty planes disappear.
- Preserve the root object, even when its result is `{}`.
- Normalize objects contained in arrays by the same field-removal rule, but preserve array lengths and positions. An empty object occupying an array position remains `{}`; it is not removed.
- Preserve `false`, `0`, `""`, and `[]`; none is structurally empty for deletion purposes.

For example, `{"strategy":{"a":{"b":{}}}}` normalizes to `{}`, whereas `{"tactics":{"steps":[{"x":{}}]}}` normalizes to `{"tactics":{"steps":[{}]}}`. These illustrate draft normalization, not permission to persist unnormalized blocks.

Materialized Memory contains only the surviving plane objects and null-free model values. Encoders and decoders MUST reject null at any depth, unknown/non-object planes, and unnormalized empty object fields; they MUST NOT repair persisted bytes or silently change a CID. Canonical key ordering is independent of insertion order. Normalization is idempotent, and reapplying an accepted patch to its normalized result yields the same materialized Memory.

Representative merge cases (each starts from the specified working memory):

| Memory before | Patch | Accepted memory / outcome |
| --- | --- | --- |
| `{}` | `{}` | `{}`; memory remains enabled |
| `{"strategy":{"keep":1}}` | `{"strategy":{}}` | `{"strategy":{"keep":1}}` |
| `{}` | `{"tactics":{"missing":null}}` | `{}`; no ancestors are created |
| `{"strategy":{"x":1}}` | `{"strategy":{"x":{"a":2}}}` | `{"strategy":{"x":{"a":2}}}` |
| `{"tactics":{"steps":[1,2]}}` | `{"tactics":{"steps":[]}}` | `{"tactics":{"steps":[]}}` |
| `{"intents":{"x":1},"tactics":{"x":2}}` | `{"tactics":null}` | `{"intents":{"x":1}}` |
| `{}` | `{"tactics":{"steps":[{"x":null}]}}` | Reject invalid selected content; unchanged Memory |
| `{}` | `{"intents":{"next":"inspect"},"goals":{"x":1}}` | `{"intents":{"next":"inspect"}}`; goals skipped as unsupported |

#### Bounded optimistic selection and safe RAM publication

Preflight the complete input's representation, duplicate keys, model-data validity, and byte/depth/entry limits. Select supported top-level roots and skip unsupported names with bounded reasons. Validate patch semantics under all selected roots, then compute their draft without mutating working memory. An unsupported-root warning is feedback, not failure of the selected set; invalid content within that set remains an error. Normalize globally and validate the final no-null/schema invariants, actual embedded depth, encoded Memory size, full returned result including feedback, and combined trajectory/context bounds. Only after all final checks succeed may Core replace working memory. A global validation/allocation/output/context failure leaves its bytes, CID, and live assignment unchanged; it MUST NOT silently choose an arbitrary subset merely to fit a limit. This safe publication protects state integrity, not a database-style requirement that every requested semantic plane be applicable.

A patch result MUST expose the complete compact accepted Memory, identify processed and skipped planes with reasons, and distinguish actual change from accepted no-effect work. Processed does not imply changed. Feedback MUST make clear which useful work survived and what, if anything, needs correction; a partial or all-skipped result MUST NOT force a corrective retry, implicit Finish, or replay of already useful work. The agent may continue with another decision. Feedback is bounded transient tool-result/trajectory data, not a new Memory plane, structural record, persistent journal, or authority. A rejected patch's error text MUST name the violated rule and the expected shape (for example, that a plane is an object of named entries rather than text containing JSON), stay within the ABI error-code bound, and keep a stable `invalid_patch` prefix. It MUST NOT repair, parse or reinterpret the rejected value.

The input, traversal metadata, intermediate draft, and output MUST all be bounded, using fallible allocation and checked arithmetic. `max_patch_entries` counts every map field and array element in the complete argument recursively, including plane fields and entries in replacement arrays; it does not count an operation list. Existing model-reply, depth, collection, Memory, block, and context caps also apply. A tiny final result never justifies an unbounded input or intermediate allocation. Model integers retain their restricted exact range; bytes, links, floats, and unauthorized values do not become editable memory data.

A normalized patch with no effect succeeds and returns the full unchanged Memory. It is Continue, not Finish; it creates no new semantic Memory version and reuses an identical block/CID at the next boundary. Actual inference and admitted local-tool work still consume their applicable count/unit budgets.

Patch arguments and intermediate drafts are ephemeral RAM, not structural blocks. Patching preserves the live Activation, immutable initial projection prefix, and RAM trajectory. Only existing Command/`Finish` boundaries persist the materialized Memory; there is no per-patch checkpoint, durable patch journal, operation log, database, or session restart. The reference application registers no separate `read_memory` tool.

### 6.3. History projection

A completed Episode is the unit of history, not an extrinsic. The current open Episode contains the action/result sequence and latest memory. An Episode with N commands finishes with N+1 Boundaries, including the final `Finish`.

By default, the model receives entry/final memory of two completed episodes and their commands and results, followed by the current action chain and memory. A narrow profile may disable historical projection. With `H = max(1, Definition.limits.history_episodes)`, persistent history retains the latest `min(H, completed episode count)` Episodes, ordered oldest to newest. Zero disables completed-history projection, not retention of the latest Boundary; genesis history is empty before the first completion. A full archive of old roots is external.

Earlier manifests do not link to one another. The required closure MUST NOT silently expand into an unlimited biography. Archive, prompt, and canonical current memory are different representations. There is no model-facing selective `read_history` or network hydration inside an activation.

## 7. RAM loop and model-facing tools

A local invocation has three control effects: `Continue` (patch/pure operation), `Yield` (Command), or `Finish` (complete reply with no tool calls). An application tool is a trusted pure planner: it receives bounded arguments and authorized context, validates the schema, and returns a command without signing or sending it. In native library tests, planner purity is a code contract, not a language-sandbox property. Reference planners execute inside the guest with no direct external capabilities; the native host executes only accepted Commands.

The Definition pins the whitelist, tool versions, and authority binding. The model cannot enlarge them through text or patches. A broad chain profile may expose many named operations; core need not offer `submit_any_extrinsic`. A compound operation with defined semantics does not imply automatic parallel tool execution.

A ModelReply may contain at most one tool call. Multiple calls are ALL rejected before any patch or Command; the adapter returns provider-correct error results for the rejected calls. A complete zero-call reply selects Finish, whether textual or silent, without a corrective inference. A valid local call remains Continue; a valid external call remains Yield. An invalid/unknown/disabled call is not an absent call and receives bounded correction; exhausted correction fails without a checkpoint.

Projection order is fixed: policy/mandate → tools → cause/observation/receipt → completed history → current action chain → memory → RAM trajectory. A machine event is not presented as a human request. A provider may encode it as user-role, but its content identifies a machine input. A post-checkpoint receipt is new input data, not an orphan tool message without an assistant call.

Patches do not rewrite the activation's initial prefix. New results append to RAM. Projection-v1 trajectory turns are model Maps `{kind: "turn", text: Text, calls: list<{call_id: Text, tool_id: Text, arguments: Map?}>, error: Text?, results: list<{ok: bool, error: Text?, result: Value, call_id: Text}>}`. Rejected/unselected arguments use explicit null; every rejected call retains its native ID/error result, and a no-call correction has a turn-level error, never an orphan tool result. The native adapter owns provider-correct encoding and may retain the original raw reply only transiently. Combined result/trajectory/context bounds validate before committing a staged Memory update. Admitted callback failures retain their count/unit charges; an invalid planned Command does not consume an accepted action slot. Text, native call IDs, and provider continuation fields disappear at a durable boundary. External diagnostic retention requires an explicit private host policy and is not necessary for restoration.

The initial memory projection contains the complete sparse semantic value, without synthesized planes. Accepted patch results expose that same complete value together with bounded processed/skipped-plane and actual-change feedback; diagnostic metadata is not editable semantic Memory. Declared tool schemas and instructions expose the object-patch contract even when the current enabled-tool subset is count/unit-filtered.

Messages and tools are validated before unsafe allocation. HTTP/network errors are not blockchain-action outcomes. Replies bind to the current request ID; stale or repeated replies are rejected.

## 8. The `raiv-state/1` profile: DASL CID and DRISL

### 8.1. Identifiers and links

The DASL CID/DRISL specifications dated 2026-10-01 apply. Before implementation stabilization, their snapshots and fixtures are pinned by hash; website changes do not silently modify the profile. [S1][S2]

All structural Raiv blocks use this binary CID:

```text
01 71 12 20 || SHA256(exact_complete_DRISL_block)
```

This is 36 bytes: CIDv1, codec `0x71`, SHA2-256, and a 32-byte digest. Text is `b` plus lowercase unpadded base32, with exact parse/format roundtrip. General DASL also permits raw `0x55`, but structural Raiv state permits only `0x71`. CIDv0, other codecs/hashes, wrong lengths, and trailing bytes are rejected.

A structural `Link<T>` is tag 42 over the bytestring `0x00 || binary_CID`, exactly 37 payload bytes. Embedded links are validated as strictly as the root. The target block's kind is additionally checked. CID strings, transaction hashes, or `$link` in arbitrary model notes do not become graph dependencies. Neither a UnixFS wrapper nor hidden chunking may change a state block's identity.

### 8.2. Types and canonicality

These are application restrictions on top of DRISL, not the full feature set of that standard. DRISL supplies tag-42 links and deterministic CBOR; Raiv uses only the required subset. [S2][S4]

A block contains exactly one complete object. Integer/length/tag encodings are minimal, lengths definite, map keys strings, and key ordering lexicographic over **CBOR-encoded keys** (`"b"` before `"aa"`). Duplicate keys, unknown structural fields, trailing bytes, invalid UTF-8, and unauthorized tags are rejected. Unicode is not normalized. Optional structural-record fields use explicit `null`, never missing keys. Sparse semantic Memory planes are optional Map entries instead: an absent plane is omitted, and a present plane is never null.

Raiv forbids all Float values, including finite float64; this is narrower than DRISL. Counters are overflow-checked `u64`. Every map/list has explicit limits. Model-authored values contain only null/bool/integers in `−(2^53−1)..(2^53−1)`/strings/lists/maps, with the additional no-null and normalization restrictions for materialized semantic Memory defined in §6. Bytes and Link belong only to runtime schemas, not arbitrary memory, observation, arguments, or result. Monetary quantities are decimal strings of integer base units; canonical unsigned form is `0` or `[1-9][0-9]*`. Tool schemas specify bounds, signedness, and units.

Decoding MUST preserve duplicate-key errors and enforce byte/depth/entry limits before allocation. Decode → canonical encode with exact byte comparison is useful defense in depth, not a substitute for bounded parsing. BTreeMap alone does not prove correct serialization order.

### 8.3. Canonical graph

Structural blocks are limited to the six kinds in §9. Core creates Links; a generic IPLD value/codec registry is not public API. Protocol IDs and evidence references are scalar bytes/text, not implicit dependencies on external proof archives.

The root has no `previous_root` link; an Episode never links to earlier versions of its own manifest. The required closure contains the definition, current memory, open episode, and bounded completed history. Decoding rejects cycles/repeated semantic positions and wrong types; an identical immutable block under one CID is loaded once.

### 8.4. Presentation of model values

A host, tool or developer surface MAY render a validated model value as **compact JSON**: canonical key order, no insignificant whitespace, decimal integers, `true`/`false`/`null`, and RFC 8259 string escaping (control characters escaped, other Unicode verbatim). Rendering MUST NOT recurse without bound over model-controlled depth. The rendering is presentation only: it is never parsed back into state, hashed, compared for identity, sent to the guest as a substitute for canonical bytes, or treated as authority. Canonical DRISL bytes and their CIDs remain the only identity. Accepted Memory rendered this way is the agent's current semantic state, not a transcript.

## 9. State data

The following schemas are normative for field membership, presence, and meaning. The notation describes records/maps, not executable Rust. Every listed structural-record key is required; `?` means `null` or the specified type. Within semantic Memory, the three plane keys are independently optional and, if present, object-valued; absence is not encoded as null. Complete byte fixtures for nested descriptors/profiles must precede encoder stabilization, without implicit Serde defaults.

`Id32` means exactly 32 raw bytes, not a Link. `Text` is bounded UTF-8. `Value` is the constrained JSON-compatible value in §8; `Map` has string keys and Value values. `Position` = `{domain: Id32, number: u64, reference: Id32?}`; it is a normalized host position, not necessarily a block number. `Usage` = `{model_calls: u64, local_tool_calls: u64, input_tokens: u64?, output_tokens: u64?, billed_units: u64?}`. Unknown cost is null, not zero.

`BudgetAdmission` = `{admission_id: Id32, activation_units: u64, remaining_episode_units: u64, remaining_model_calls: u64, remaining_actions: u64}`. A host profile defines the unit; it is not a universal token-to-money conversion.

### 9.1. Six structural blocks

```text
Definition {
  kind: "definition", version: 1,
  execution_domain: Id32,
  policy: Text, mandate: Text,
  memory_profile: "none" | "intents-strategy-tactics/1",
  tools: list<ToolDescriptor>,
  inference_profile: Map,
  projection_version: 1, normalizer_version: 1,
  limits: Limits
}

Memory {
  kind: "memory", version: 1,
  value: SemanticMemory  // sparse Map: optional intents, strategy, tactics Maps
}

ActivationInput {
  kind: "activation_input", version: 1,
  event_id: Id32,
  cause: "actor_wake" | "action_result",
  observed_at: Position,
  observation: Map,
  receipt: Receipt?,
  budget_admission: BudgetAdmission
}

Boundary {
  kind: "boundary", version: 1,
  ordinal: u64,
  input: Link<ActivationInput>,
  memory: Link<Memory>?,
  exit: {tag: "await_action", command: Command}
      | {tag: "await_actor"},
  usage: Usage
}

Episode {
  kind: "episode", version: 1,
  id: Id32, trigger_event_id: Id32,
  entry_memory: Link<Memory>?,
  boundaries: list<Link<Boundary>>,
  closed: bool
}

AgentState {
  kind: "agent_state", version: 1,
  format: "raiv-state/1",
  agent_id: Id32, revision: u64,
  definition: Link<Definition>,
  memory: Link<Memory>?,
  continuation: {tag: "await_actor"}
              | {tag: "await_action", action_id: Id32},
  current_episode: Link<Episode>?,
  history: list<Link<Episode>>
}
```

`SemanticMemory` is exactly the normalized, recursively null-free object defined in §6; it is not a general runtime `Value` registry. For `intents-strategy-tactics/1`, genesis and every accepted memory position have a non-null `Link<Memory>`, even when `Memory.value` is `{}`. The `none` profile instead has null memory links and no Memory block.

`ToolDescriptor` contains `id`, `version`, `control`, `description`, `arguments_schema`, and `authority_binding`. Every field is required; the schema is a bounded Map, and authority_binding is Id32 or null for local tools. Control is `continue`, `yield`, or `finish`. Names are unique and their order fixed; runtime planners match descriptors and the authorized code version. Core need not implement all of JSON Schema: the trusted application validator and advertised descriptor must agree.

Definition contains no secret-bearing URLs, headers, or keys. A changed mandate, tool semantics, or logical model selection requires an administrative new Definition, not a memory patch. This revision does not support live upgrades during an episode. Limits are enumerated in §17; deployment cannot silently change them after genesis.

### 9.2. Consistency checks

Episode ordinals start at 0 and are contiguous. The first Boundary has cause `actor_wake` and a null receipt; subsequent boundaries have `action_result` and the previous command's matching terminal Receipt. Action index counts preceding `await_action` boundaries in the episode, starting at 0.

AwaitAction requires an open current Episode ending in the matching command. AwaitActor has current_episode = null; except at genesis, the last history Episode is closed and has a final `await_actor`. Only the last Boundary may contain `await_actor`. Root.memory equals the last accepted Boundary.memory; the next episode's entry_memory equals memory immediately before that episode. All memory links are null under memory_profile=none.

Unchanged Memory blocks are reused. Each accepted boundary increments revision by exactly 1; overflow is rejected. The sum of retained Boundary counts cannot exceed revision; if it is smaller, earlier completed Episodes were evicted and the retained history window must contain H Episodes (§6.3). Caller `history_episodes` is a materialization count cap, including zero; it may reject the snapshot but MUST NOT shrink the Definition-bound window. New blocks do not contain their future acceptance timestamp or their own acceptance-event hash, which would create a dependency on the future. The host retains those facts.

## 10. Commands, receipts, and stable IDs

`execution_domain` distinguishes deployment/network/service. The host assigns it and binds it to agent registration; transferring bytes to another network does not transfer authority. Registration assigns AgentId, which is not the mutable root. A local reference deployment may deterministically derive Id32 from a domain and human-readable label without a signer.

```text
Command {
  action_id: Id32,
  tool_id: Text, tool_version: u64,
  authority_binding: Id32,
  arguments: Map
}

Receipt {
  action_id: Id32,
  command_digest: Id32,
  outcome: "succeeded" | "failed" | "not_executed",
  result: Map,
  execution_reference: Text?,
  finalized_at: Position,
  costs: Map
}

episode_id = SHA256(DRISL([
  "raiv/episode/1", execution_domain, agent_id, trigger_event_id
]))

action_id = SHA256(DRISL([
  "raiv/action/1", execution_domain, agent_id, episode_id, action_index
]))

command_digest = SHA256(DRISL([
  "raiv/command/1", execution_domain, Command
]))
```

All digests above are Id32 scalars, not separate Links. ActionId identifies a logical slot, not a signed transaction hash. Competing pre-acceptance candidates may propose different commands for one slot; after acceptance, its command_digest binding is immutable. Provider call IDs, nonces, and transaction hashes do not replace ActionId.

`failed` reports a confirmed failure with any partial effects represented in result. `not_executed` means the attempt has terminated without execution and cannot later execute as that same attempt. An RPC interruption or lack of inclusion does not establish this. For an unknown outcome, the host keeps waiting and resolves the action; it must not invent an ordinary receipt.

The executor normalizes outcomes, including nested errors, event evidence, and costs under the tool's semantics. Core checks matching ID/digest/phase/bounds; the host supplies provenance. Model reasoning cannot create a Receipt. A deliberate retry following a terminal outcome receives a new ActionId.

`set_wake` or a DEOS Actor update is another external command with checkpoint and receipt. Intents and tactics do not schedule, execute, or cancel anything. After `Finish`, the previously accepted Actor condition applies. The survivor simulator starts with a fallback rule so failed rescheduling does not remove its next wake-up.

## 11. On-chain anchoring and agent registration

### 11.1. Anchor

The blockchain profile stores an authoritative compact commitment, not the entire memory. The proposed logical record is not a finalized SCALE layout:

```text
AgentAnchor {
  agent_id: Id32,
  revision: u64,
  state_profile: 1,
  root_digest: [u8; 32]
}

RegistryBinding {
  execution_domain: Id32,
  authorized_program: host_code_reference,
  definition_digest: [u8; 32],
  authority: host_authority_reference,
  availability_policy: host_policy_reference
}
```

The agent_id MAY be the storage-map key rather than a duplicated value field. `state_profile:1` unambiguously denotes `raiv-state/1` and prefix `01 71 12 20`, allowing reconstruction of the full CID from its digest. A service type may pin the profile instead of a separate field, but the profile is **not** guessed from arbitrary hash length. Changing codec/hash requires a new registry/profile binding. [S1]

RegistryBinding is host metadata, not a seventh state block. The Definition is bound to the authorized program; a different Definition inside a hash-valid root does not acquire authority. The reference native host represents both records in a local manifest.

### 11.2. Accepted transition

```text
accept_checkpoint(
  agent_id,
  expected_revision,
  expected_root_digest,
  admitted_input_digest,
  next_root_digest,
  trusted_run_output
)
```

This specifies a host operation, not a public extrinsic carrying an unverifiable `trusted=true` flag. The environment binds trusted run output to the actual run, source anchor, input, and Definition. Blind acceptance of a caller's payload does not implement this profile.

Acceptance atomically checks the current revision/root, binds the consumed event to the exact ActivationInput digest, installs revision+1/root, and authorizes only the command contained in the accepted state. Candidates explicitly carry the expected revision; hash comparison alone is insufficient as the general replay-control interface.

The trusted output may carry the command directly so the executor need not reload the DAG first. It MUST be a projection of the **same** Command inside Boundary; trusted execution binds both representations. Arguments cannot change between serialization and execution. Registering the instruction and root is one coherent host operation; the actual external effect occurs later.

For auditing, the host SHOULD publish an event with agent_id, old/new revision/root, admitted input digest, and optional action ID/digest. Indexing and retaining events is a separate service contract: blockchain history does not guarantee that every pruned node retains all old bytes.

### 11.3. Finality and competing branches

Dispatch requires the finality level specified by deployment. The reference chain profile requires final acceptance before dispatch, without speculative external effects followed by rollback. Competing outputs from a stale anchor are rejected. A late receipt for an unaccepted branch cannot activate the agent.

If a communication failure makes acceptance uncertain, the host first resolves the authoritative anchor and event consumption. It retries the same candidate or returns the known decision; it does not authorize another plan while prior acceptance may still be effective. Agent strategy does not replace this machine-enforced discipline.

## 12. Preparation, acceptance, and delivery

### 12.1. Before inference

The host obtains the authoritative anchor and event, reconstructs the CID under the pinned profile, loads the entire required closure, validates CIDs, format, kinds, identity, Definition, and graph invariants, and prepares the bounded observation/receipt and budget admission. Only then may `prepare` run.

A missing block produces `InputUnavailable`; wrong bytes produce `IntegrityFailure`; another format produces `UnsupportedVersion`. Empty memory, genesis, or a partial successful restore MUST NOT be substituted. Every storage provider is subject to the same exact-byte verification.

### 12.2. At a boundary

Core builds the candidate without I/O. The host checks its run/input/anchor binding and places new blocks. Existing dependencies must also remain covered by storage policy. Once the readiness gate in §13 is satisfied, the host accepts the root/event/instruction. Only then may the executor send the action.

IPFS writes and blockchain acceptance do not form a shared transaction. Placement precedes anchoring; a CAS conflict leaves orphan staging blocks, reclaimed by the host after a safe grace period. A storage timeout never authorizes dispatch. Core computes the root hash; it is not selected from a pinning service's response.

Accepting `AwaitActor` closes the Episode and waits for the existing Actor. Accepted `AwaitAction` contains a durable instruction recoverable after host failure. An outbox may cache it but is not a second, unbound source of truth.

### 12.3. Redelivery

The executor distinguishes ActionId, transport attempt, signed transaction, and effects. It returns an existing Receipt or resolves the fate of an already submitted action. A new nonce must not silently create a second economic action. Core promises no universal exactly-once behavior for arbitrary blockchains; adapters must define their own safe contract.

In the simulator, the world update and Receipt indexed by ActionId are atomic. Redelivery with the same digest returns the same Receipt; another digest under that ID fails. A crash after receipt arrival but before the next boundary reruns only continuation computation against the same admitted input.

## 13. Availability of actual data

### 13.1. Content integrity does not require a trusted store

A block source may be untrusted because CID matching is checked locally. Under the stated cryptographic assumptions, it cannot silently substitute content under the same verified root. It can nevertheless withhold responses or lose bytes. IPFS pinning retains content at a particular node; it is not an unlimited guarantee of every future read. [S5]

Raiv implements neither storage consensus, proof-of-storage, nor provider economics. It requires the host to **name and fulfill** a policy adequate for its deployment. The model cannot attest to storage readiness through a tool.

### 13.2. Minimum storage contract

A production policy MUST specify coverage of the **entire required closure**, replica operators/sources, placement acceptance conditions, retention and renewal, payment/funding arrangements, a recovery-time objective, failure handling, and release conditions for old blocks. Two URLs do not establish independent operators.

Deployments SHOULD maintain at least two independently administered full copies or an explicitly documented recoverable-fragment scheme. This is a deployment recommendation, not proof of decentralization. Erasure coding does not change state payloads or canonical roots; reconstruction precedes activation.

Storage readiness binds to the candidate, accepted policy, and exact dependency set. Acknowledging only the root block is insufficient. A local reference deployment may use verified crash-durable local writes, clearly labeled local-only. A replicated reference deployment requires complete readback and verification from two independent directories/adapters; this tests failures, not independent corporate ownership.

### 13.3. Availability after acceptance

A checkpoint may wait for an Actor indefinitely. A finite pin lifetime that does not cover that wait cannot support a promise of continuous existence. The host must renew storage leases independently of agent wake-ups or bound the agent's lifetime and explicitly report retention exhaustion. A block reachable from the current root or needed by an accepted pending action cannot be released merely as old data.

Responsibilities include periodic retrieval checks and replacing failed replicas to restore the required copy count. A placement receipt does not prove future availability. A stricter trust-minimized deployment may use verifiable storage/DA or protocol replication; core does not prescribe a specific mechanism.

Retention costs enter host accounting and the next observation. The survivor needs its total cost of existence, not only command fees. Lack of funding does not imply that remote data remains safely retained.

### 13.4. Failure and recovery

When data is unavailable, the host tries remaining sources; inference and new commands are prohibited until complete restoration. The last accepted on-chain digest remains current rather than being replaced with an empty checkpoint. The executor continues tracking already submitted actions: losing memory does not undo external effects.

If **all** copies are lost, the digest cannot reconstruct the preimage. The host explicitly marks the agent unavailable; reconstruction from sufficient archival material is a separate procedure that must recover the exact expected bytes. A new root built from guessed or summarized memory is not restoration of the old one.

Importing an archive into another host restores data, not authority, canonical HEAD, event consumption, or the submitted-action ledger. Operational continuation obtains those from the accepting protocol. Offline replay of an old root is allowed only in an isolated simulator without real effects.

## 14. CAR and the IPFS adapter

CAR is an external container, not a seventh state block or a source of canonical HEAD. The general CAR specification does not guarantee graph completeness; Raiv adds its own restore profile. [S3]

Export MUST contain header `{version:1, roots:[expected_root]}` and exactly the required closure without duplicates. The header is DRISL and records are length-prefixed; lengths and block count are bounded. Export sorts blocks by binary CID for reproducible files. Import is frame-order independent; hashing the entire CAR does not replace the state root.

Importers require an explicit expected root, match the header, check every length/CID/body, and validate all required links before acceptance. Exact duplicates may be deduplicated; conflicting bytes, truncation, integer overflow, missing blocks, or wrong kinds reject the import as a whole. Bounded extra blocks from a foreign CAR are not state or prompt data; their staging/GC belongs to the host.

The IPFS adapter retrieves/places exact codec-`0x71` DRISL blocks and satisfies the chosen pin/retention contract. Kubo block/get, block/put, or CAR import/export are possible implementations; the Raiv library does not depend on Kubo. [S11] A normal UnixFS file upload must not substitute its wrapper CID for the computed root. Root and nested blocks are checked after roundtrip; block-size limits are reconciled with transport without hidden re-encoding. [S12]

HTTP endpoints, gateway discovery, P2P, pinning authentication, encryption, and leases are outside core. The reference application uses public synthetic data. Content addressing does not confer confidentiality; real encryption requires a separate envelope/key-management profile without secrets in the on-chain root, prompt, or public logs. Changing transport encryption must not silently change canonical state.

## 15. Library API and four external ports

The API below specifies the shape of the contracts, not a compiled crate. The skeleton milestone makes the types executable without altering wire semantics.

```rust
impl RestoredAgent {
    pub fn restore(
        root: DaslCid,
        blocks: &impl MaterializedBlocks,
        limits: &Limits,
    ) -> Result<Self, GraphError>;
}

pub fn prepare<'tools>(
    restored: RestoredAgent,
    input: PreparedInput,
    tools: &ToolSet<'tools>,
    policy: RunPolicy,
    sequence: RequestSequence,
) -> Result<Activation<'tools>, ActivationPrepareError>;

impl Activation {
    // RAM-stage Sealed retains a read-only planned decision, not a CommitCandidate.
    pub fn advance_ram(&mut self, binding: RequestBinding, bytes: &[u8])
        -> Result<RamProgress, ActivationError>;
    pub fn ram_decision(&self) -> Option<&RamDecision>;
    // Pure data composition after successful sealing, not host acceptance.
    pub fn candidate(&self) -> Result<CommitCandidate, CandidateError>;
    pub fn request(&self) -> Result<ModelRequest<'_>, RunError>;
    pub fn advance(&mut self, reply: ModelReply)
        -> Result<Progress, RunError>;
}

pub enum Progress {
    InferAgain,
    Boundary(CommitCandidate),
}
```

```text
CommitCandidate {
  agent_id: Id32,
  expected_revision: u64,
  expected_root: DaslCid,
  admitted_event_id: Id32,
  admitted_input_digest: Id32,
  new_root: DaslCid,
  new_blocks: list<(DaslCid, bytes)>,
  disposition: AwaitActor | AwaitAction(ActionId)
}
```

`DaslCid` is a validated type, not an arbitrary CID alias. State roots additionally require codec `0x71`. `MaterializedBlocks` reads already available bytes without hidden I/O. `PreparedInput` is constructed at the trusted host boundary; its Rust type alone does not prove provenance. For prepared-packet and acceptance binding, `admitted_input_digest` is SHA-256 of the exact complete canonical `ActivationInput` DRISL bytes (the input CID digest).

`RunPolicy` carries raw caller limits and trusted profile upper-bound ExecutionQuotes; model text cannot supply or widen them. The runtime supplies the non-resettable live-instance RequestSequence. Preparation validates complete initial context/working-memory resource bounds before the first count/nonce/unit debit and returns that sequence unchanged on failure. Runnable preparation requires caller capacity for the Definition-promised next completed history window and a non-overflowing next revision; valid restored data alone need not be runnable. Enabled request descriptors are a registered count/unit-filtered subset, distinct from immutable declared-prefix data. Adapters must not duplicate descriptor data as extra model messages outside canonical context accounting.

`request()` provides a borrowed view and performs no inference. ModelRequest carries a request ID, versioned projection, descriptors, normalized model parameters, and limits. ModelReply contains reply-ID binding, text, a call list whose cardinality is validated, and Usage. A provider ID never becomes a persistent agent ID by default.

Normalizer version 1 uses a closed canonical-CBOR reply frame `{request_id: u64, text: Text, calls: list<Call>, usage: Usage}`; each Call has exactly `{call_id: Text, tool_id: Text, arguments: opaque canonical DRISL value}`. Text absence normalizes to empty text. Native call IDs are transient correlation text, never tool/version/authority grants. The whole frame obeys `max_model_reply_bytes`, depth and collection bounds, not the structural-block byte ceiling. Complete wire/schema/Usage metadata and matching request IDs are checked before consuming the pending reply. Control selection rejects all multiple calls before interpreting arguments and retains every call ID for adapter error results; only a single canonical model-Map argument can proceed to application validation. Malformed wire is an inference failure, while an admitted invalid single argument or multiple-call decision may take bounded correction without effects. Reported Usage never overrides Core-owned call counters. This frame adds no checkpoint kind or hash domain.

`candidate()` requires a successfully sealed RAM decision, including after request-sequence transfer. It binds the prepared source/event/input digest, increments revision once, appends the materialized Boundary/Input/Memory, preserves or opens the Episode identity, closes only on Finish and applies the promised history window. The complete resulting closure MUST validate under captured caller/Definition caps before return; failure leaves the source run unchanged. Candidate data exposes the exact unique new-block delta relative to the source closure and the required closure; an embedded Command projection MUST borrow the same bytes, not create an independently mutable instruction. Repeated construction is deterministic and does not perform inference/tool work; persistence retries SHOULD reuse the immutable returned candidate. Construction and data restoration do not establish host acceptance, authoritative HEAD, grant freshness or dispatch permission.

The adapter retains full provider HTTP payloads only in RAM when needed. Boundary seals the Activation against further replies; fatal errors discard it. Retrying persistence of the same candidate requires no new inference. `expected_root`/input binding belong to acceptance and do not link the new graph to an unbounded past chain.

| Port | Contract | Excluded responsibility |
| --- | --- | --- |
| Inference | ModelRequest → normalized ModelReply | Core model consensus/weight verification |
| Blocks | Exact CID bytes, materialization, and retention policy | Choosing the authoritative root |
| Acceptance | Trusted-run binding, anchor CAS, event consumption, dispatch permission | Semantic judgment of strategy |
| Executor + Actor | Deferred effects, receipts, wake conditions | Another scope or parallel episode |

`restore` validates a bounded typed data snapshot, not an Activation or authoritative HEAD. Caller `Limits` bound the complete closure and embedded records, not just single-block decoding; Definition policy cannot widen them. Application validators, execution-unit/lifetime policy, fresh admission and authorized code/Definition bindings remain prepare/host duties. Genesis construction is a separate pure helper; only the host can register it where no anchor exists.

## 16. Rust, dependencies, and WebAssembly integration

Minimum supported Rust is **1.85**. Core and its runtime dependencies MUST support `no_std + alloc`, with no filesystem, environment variables, wall clock, RNG, threads, async runtime, or network clients. Validate the MSRV with locked native tests, feature-graph inspection and library builds for `wasm32-unknown-unknown` and `thumbv7em-none-eabi`; `no_std` alone does not prevent std dependencies. [S7]

The minicbor-backed codec is private to `format`; typed schemas stay with their domain owners. IPLD/CID dependency crates are forbidden in normal/build/dev graphs; no universal runtime Value or codec registry is introduced. Ciborium supplies independent test-only byte evidence, not a runtime data model. Neither backend replaces strict Raiv canonicality, schema and resource checks. [S6]

Do not implement general CBOR or cryptography from scratch. Use established SHA-256/base32 primitives with minimal features. A narrow profile validator is permitted and required when the decoder is too permissive. Pin/lock dependencies; test MSRV/targets and measure binary size rather than promise portability.

### 16.1. Reference execution boundary

`raiv-agent.wasm` MUST be a Rust `no_std + alloc` application compiled for `wasm32v1-none` and instantiated by Wasmi 2.x in the native Rust `std` host. WASI is not used; the reference module has no imports. Filesystem, environment, clocks, RNG, threads, async runtimes, HTTP/JSON/SSE clients, and terminal dependencies MUST NOT enter Core or the guest. The guest supplies its bounded allocator/panic handling; the host sets finite linear-memory and per-step fuel limits before instantiation and treats exhaustion/traps as failed execution, not `Finish`.

The host pins the authorized guest module and ABI version to the run/Definition/anchor/input binding. It drives guest steps and handles only the external ports in §15, never rebuilding context, selecting tools, changing decisions, or bypassing guest validation. Core's Rust 1.85 contract remains; the host's selected dependencies/toolchain and exact Wasmi 2.x release must be locked and validated separately.

**RAM lifetime belongs to the runtime adapter.** The reference host MUST keep the same live instance and Activation across inference requests/replies; it MUST NOT serialize a transient activation or terminal transcript into `raiv-state/1`. At accepted Command/Finish boundaries it MAY destroy the instance and instantiate a fresh one from the accepted root and complete required closure. On a pre-boundary trap/loss, replay starts from the last accepted checkpoint with the same admitted input and host-accounted lost attempts, not guessed RAM. A sealed candidate is retried without new inference; uncertain acceptance is resolved under §11.3.

Other VM integrations may need an integrity-protected transient continuation: run/request binding and freshness are mandatory against substitution/replay, and it is not canonical state. A JAM deployment must separately define its ABI, admitted inference inputs, and covered execution guarantees; neither a WASM build nor Wasmi isolation supplies JAM readiness or a new consensus/proof layer.

### 16.2. Minimal versioned CBOR step ABI

The reference ABI uses one complete CBOR envelope `{abi_version: 1, type: Text, payload: map}` per step, with the exact type/payload fields below. All shown keys are required; `?` means explicit null or the specified value. It is transport, NOT DRISL state serialization or a general RPC framework: canonical state blocks and ActivationInput travel as opaque exact DRISL bytes, CIDs as their fixed 36-byte binary form. Normalized ModelRequest/ModelReply and CommitCandidate retain §15 meanings; encoding them in the envelope does not add a state kind or hash domain. `Block` below means `{cid: bytes, body: bytes}`.

| Direction/type | Payload and effect |
| --- | --- |
| Host → `initialize` | `{agent_id: Id32, definition: bytes, memory: bytes?}`; guest builds revision-zero data without inference; host registers only where no anchor exists |
| Host → `activate` | `{root: bytes, blocks: list<Block>, input: bytes}`; restore the complete authorized closure and prepare the admitted ActivationInput |
| Host → `inference_reply` | `{request_id: u64, reply: ModelReply}`; advance the live activation with a complete validated reply |
| Host → `inference_error` | `{request_id: u64, code: Text}`; terminate the failed inference/activation without a candidate |
| Guest → `initialized` | `{root: bytes, blocks: list<Block>}`; genesis data, not registration or dispatch permission |
| Guest → `infer` | `{request_id: u64, request: ModelRequest}`; yield for native inference while retaining RAM |
| Guest → `boundary` | `{candidate: CommitCandidate}`; seal activation; host applies placement/readiness/acceptance before effects |
| Guest → `error` | `{code: Text}`; explicit ABI/validation/run failure, never an implicit checkpoint. A failed activation SHOULD report its §17 category where known: `corrective_turns_exhausted`, `budget_exhausted` or `context_limit`, else `activation_failed` |

In the CBOR transport, `ModelRequest` is a closed map `{projection_version: u64, context: bytes, model_parameters: map, tools: list<ToolDescriptor>, limits: Limits}`. `context` holds the complete exact canonical model-compatible CBOR projection, not a state block or an additional message; `tools` contains only the enabled descriptors in Definition order. The outer `request_id` binds this request; it is not duplicated inside the map. Embedded model parameters, descriptors and Limits retain their owner schemas and bounds. `CommitCandidate` uses the eight fields shown in the Core contract, with fixed binary CIDs, `new_blocks: list<Block>` in binary-CID order and closed disposition `{tag: "await_actor"}` or `{tag: "await_action", action_id: Id32}`. This is the exact source-relative delta; placement and required-closure validation use it together with the source blocks, not a fabricated extra root/state kind.

`initialize` is allowed only on a fresh instance; `activate` requires no live unsealed activation or unresolved prior candidate acceptance. Each step runs synchronously until an inference yield, sealed boundary, or error. The host performs inference only for `infer`, then calls the next step with the matching reply/error. At most one inference request is outstanding. Transport request IDs are checked, monotonically increasing u64 values scoped to the live instance, bound to the ModelRequest/ModelReply correlation, and never persistent Episode/Action/provider IDs. Overflow fails; IDs are not reused within an instance. The host binds replies to the originating instance/run; both host and guest MUST reject stale/duplicate/wrong IDs without advancing the pending activation. Stream chunks are not ABI messages.

The guest exports `memory`, `raiv_alloc(len: u32) -> u32`, `raiv_dealloc(ptr: u32, len: u32)`, and `raiv_step(ptr: u32, len: u32) -> u64`. Allocation returns a nonzero guest offset or zero on failure. The host writes only to that allocated input range; step returns `(u64(output_ptr) << 32) | u64(output_len)`, both u32 guest offsets/lengths, never native pointers. The host frees its input allocation through `raiv_dealloc` after the step; the guest retains no references to freed input. Output is guest-owned, read-only to the host, valid until the next step; the host copies it before another step and MUST NOT deallocate it. One input and one output frame may be outstanding. The host/guest check allocation ownership, overflow and bounds against linear memory; invalid or overlapping internal/output ranges are rejected, never dereferenced. A trap or zero step result is an execution failure with no successful output.

Each frame is at most 8,388,608 bytes, depth 64 (allowing envelope nesting around existing depth-bounded values), and 65,536 aggregate container entries; all ABI error codes are at most 256 UTF-8 bytes. Lengths are checked before allocation/copy/parse, independently on both sides. The transport envelope uses canonical CBOR: minimal scalar/length encodings, definite collections and text keys ordered lexicographically by their complete CBOR encodings, with the same narrower value forms as the Raiv wire profile. This constrains envelope encoding, not state identity: embedded complete DRISL records stay opaque exact bytes. Duplicate/unknown fields, wrong types, invalid UTF-8, trailing bytes, malformed payloads and unsupported ABI versions are rejected before guest advancement. Existing narrower block/materialization/context/reply/tool/budget limits still apply and are never widened by this transport cap. A reference configuration that cannot fit these ABI bounds MUST fail validation, not silently truncate or alter Definition policy. ABI failures return `error` where a safe output buffer is possible; traps/fatal failures discard the activation, while mere stale correlation rejection leaves it pending. None change the accepted root or authorize effects.

## 17. Errors and resource limits

Categories: `InputUnavailable`, `IntegrityFailure`, `UnsupportedVersion`, `WrongAgent`, `WrongDomain`, `WrongPhase`, `StaleInput`, `UntrustedRun`, `UnknownTool`, `InvalidArguments`, `InvalidPatch`, `MultipleToolCalls`, `CorrectiveTurnsExhausted`, `InferenceFailure`, `ContextLimit`, `BudgetExhausted`, `CommitConflict`, `StorageFailure`, `AvailabilityNotReady`, `OutcomeUnknown`.

A recoverable local tool error may be returned to the next model turn. A fatal error never creates a no-op checkpoint; the last accepted root remains, while the host retains diagnostics and admitted input. Administrative suspension is not reported as voluntary successful episode completion.

Normative Limits fields and initial reference-application values:

```text
max_memory_bytes                  4096
max_observation_bytes             8192
max_receipt_bytes                 4096
max_command_bytes                 2048
max_block_bytes                  65536
max_materialized_bytes         4194304
max_blocks                        4096
max_depth                           32
max_collection_entries            4096
max_context_bytes               131072
max_model_reply_bytes            32768
max_patch_entries                   32
history_episodes                     2
max_actions_per_episode              8
max_model_calls_per_activation      16
max_model_calls_per_episode         64
max_local_tools_per_activation      16
max_corrective_turns                  2
```

All fields are u64 and checked before conversion to usize. These are test baselines, not optimal-performance claims. Receipt/command limits cover the entire record; context includes all messages and tool descriptors; adapters separately enforce HTTP-body and timeout limits before parsing.

For projection version 1, `max_context_bytes` measures the complete canonical model-compatible CBOR projection, including message/tool-descriptor data, the immutable initial prefix, appended RAM trajectory and encoding overhead. Runtime Id32 metadata in this model data uses lowercase hex text; u64 metadata uses canonical unsigned decimal text rather than lossy model integers. This is neither a token estimate nor a provider JSON/HTTP-body limit; adapters enforce their separate transport bounds. Projection data and phase/Receipt consistency alone do not establish fresh admission, registered/budget-enabled tools or a ready inference request.

Frame counts, definition sizes, DAG size, and projection MUST be validated together. Policy cannot silently truncate receipts/memory or drop episodes from the promised window. An oversized input fails preflight; changing the history window requires an explicit profile change.

Execution-unit admission uses trusted host-profile quotes/reservations before inference or tool effects, bounded by both `activation_units` and `remaining_episode_units`. A quote is a conservative upper-bound reservation, not an exact bill or a value chosen by model arguments. Unknown reported costs do not become zero or authorize a refund; the host separately owns exact/lifetime/lost-attempt accounting. Zero available units are exhausted even for a zero quote; with positive unit capacity, a free operation remains count-bounded.

An exhausted action/local-tool budget disables the corresponding tools, not intrinsic Finish while overall budget remains. A profile with no enabled tools may still infer and Finish. A system pause on overall exhaustion is not Finish.

Model-call and action counters accumulate across the current Episode; ActionResult does not reset them. `Usage.local_tool_calls` counts admitted Continue operations in that Activation; Yield consumes an action slot and Finish consumes neither local nor action count. Every admitted inference request consumes a model-call slot, including attempts that later fail; viewing the same pending request again is not another call. Corrective-turn limits apply to the whole Activation, not a streak reset by a successful Continue. The host separately accounts for failed paid API attempts and lost activations. Accepted checkpoint Usage is not the complete lifetime expense ledger. Token estimates are not reported as exact monetary charges.

## 18. Reference agent outside a blockchain

### 18.1. Profiles

**Veto:** a fixed mandate and prepared referendum, memory=none, and `veto`, `pass` tools. The simulator treats veto/pass as external commands with delayed outcomes; on-chain mappings are chosen later. After a receipt, the agent may finish or continue within policy.

**Survivor:** manage a virtual budget and preserve the ability to continue operating; sparse `intents`/`strategy`/`tactics`; `patch_memory`, `trade`, `set_wake`. Finish is intrinsic to complete zero-call replies. Tools provide no real funds, keys, Internet, or shell. The host performs network inference. Economic success is not a harness-correctness test.

### 18.2. Local durability

Unit tests use MemoryBlockStore. DiskBlockStore stores exact DRISL bytes indexed by CID. JSON and SQLite are not persistent agent state. One atomic host manifest contains the anchor, admission/consumption, and bindings; simulator atomicity binds world changes to the Receipt ledger.

The reference native Linux host locks acceptance, writes temporary files, fsyncs blocks, atomically replaces the manifest, and fsyncs its directory. Guarantees apply to the tested OS/filesystem; corruption is not hidden by new genesis. A portable filesystem transaction is not attributed to `rename` without testing.

Headless `wake` and `resume` drive the WASM guest to exactly one boundary and exit. `execute-pending` is separate; a runner may compose these steps without adding a core scheduler. Restoring the agent directory reconnects to the same executor ledger instead of creating a world that forgot prior spending.

### 18.3. Inference and authentication

ScriptedInference is mandatory first and exercises the same CBOR step interface without network access or credentials. The native host MUST implement an OpenAI-compatible Chat Completions adapter with configurable endpoint and exact model ID; DeepSeek is the initial example, not a hardcoded general interface. HTTPS, authentication, JSON, SSE framing/assembly, and provider-specific normalization occur outside WASM. Explicit provider adapters MAY handle extensions; fixtures pin their concrete API snapshot. [S9]

Only complete, validated normalized ModelReply data enters the guest. Streaming MAY progressively display text, but a failed, truncated, malformed, or interrupted stream MUST produce an inference failure, never a successful partial reply or checkpoint. Context/reply bytes, model-call counts, tool-call cardinality, and execution budgets in §§7/17 still apply; host HTTP/SSE limits apply before parsing. No stream chunk advances guest state.

Provider extensions are explicit adapter choices bound to the authorized Definition's inference profile, not inferred from endpoint/model strings by the generic interface. The live inference profile is a closed model Map with `provider`, `protocol: "openai-chat-completions"`, `base_url`, `model` and `thinking: bool`, plus `reasoning_effort` exactly when `thinking` is true. It names one provider family:

| Provider | Request extensions | Reasoning |
| --- | --- | --- |
| `opencode-go` | `thinking: {type: "disabled"}`, or `{type: "enabled"}` plus `reasoning_effort` for the pinned reasoning model | Enabled mode retains provider `reasoning_content` in live-activation RAM and passes it back on later tool-enabled requests, as DeepSeek requires [S10]; disabled mode rejects nonempty reasoning |
| `openai-compatible` | None; `thinking` is always false and `reasoning_effort` is absent | Provider reasoning text is transient adapter data: never replayed, stored, shown to the guest or treated as a reply |

A durable boundary always starts a new context without previous reasoning. Both families send the standard body (`model`, `messages`, `max_tokens`, `stream`, `n: 1`, tools with `tool_choice: "auto"` and `parallel_tool_calls: false` when tools are enabled, and `stream_options.include_usage` when streaming).

Provider responses are normalized strictly but interoperably. The reported model MUST equal the configured model, or be its dated snapshot `<model>-YYYY-MM-DD`; no other prefix, alias or case variant matches. In a streamed tool call, the first fragment for an index carries a nonempty `id`, `type: "function"` and the name; continuation fragments MAY repeat `type` and an empty `id`, while a nonempty different `id` is a framing error. Unknown delta members are ignored after bounded parsing; refusals, legacy `function_call`, conflicting terminal data, duplicate keys and malformed fragments still fail the reply.

The host MUST obtain the API key exclusively from the environment variable named by `api_key_env`; missing variables and unsupported protocols are explicit configuration errors before live inference. The `opencode-go` family fixes the name `OPENCODE_API_KEY`; `openai-compatible` accepts any valid variable name (`[A-Za-z_][A-Za-z0-9_]*`). Provider-routing headers required by a specific endpoint carry no credential and are not Definition data. API keys MUST NOT enter WASM memory, model context, Definition, Command, Receipt, checkpoints, CAR, or ordinary application logs. Browser/OAuth/subscription authentication is outside this revision. Endpoint changes must respect the authorized inference-source policy; a model/mandate/permission change requiring a Definition update cannot silently mutate an existing agent's authority.

### 18.4. Example configuration

`raiv-agent.toml` configures one local agent. Compact fields expand into the full Definition at initialization; afterwards the configuration is pinned by digest and mismatches are refused rather than applied (§18.6). Renamed example labels/domains apply to new registrations only; application renaming MUST NOT rewrite an existing execution_domain, identity, or Definition binding.

```toml
schema_version = 1
state_format = "raiv-state/1"

[agent]
profile = "survivor"                  # or "veto": memory disabled
id = "<64 lowercase hex>"
mandate = "Assist the local operator. Save lasting facts with patch_memory before answering."

[execution]
adapter = "local-trusted"

[runtime]
guest_module = "guest.wasm"
module_digest = "<SHA-256 of guest.wasm, 64 lowercase hex>"

[inference]
protocol = "openai-chat-completions"
provider = "openai-compatible"        # omit for the opencode-go profile
base_url = "https://opencode.ai/zen/go/v1"
model = "qwen3.8-flash"
api_key_env = "OPENCODE_API_KEY"      # a variable NAME, never the key
stream = true
max_tokens = 4096
max_request_bytes = 262144
max_response_bytes = 1048576
max_header_bytes = 16384
timeout_ms = 60000
max_stream_event_bytes = 65536
max_stream_events = 4096

[storage]
adapter = "disk"
directory = "blocks"

[acceptance]
adapter = "local"

[executor]                            # optional synthetic world
adapter = "scripted-unit-world"
directory = "world"
world_id = "<64 lowercase hex>"
virtual_units = "10"
capacity = 16
```

Every section and key shown is required except `provider`, `reasoning_effort` (`opencode-go` only) and `[executor]`; unknown or duplicate keys and credential values are rejected. Paths resolve relative to the configuration file. Offline configurations use `protocol = "scripted"` with `model = "fixture-model"` and no live keys. Limits are the Definition's §17 values rather than configuration fields. Draft fields such as `label`, `mandate_file`, `domain`, `[availability]`, `[actor]`, `[memory]` and `[limits]` are not part of schema version 1.

`protocol` selects the host API adapter. In this revision, the selector `openai-responses` is a compatibility alias for `openai-chat-completions`; both select Chat Completions, NOT the separate OpenAI Responses API or `/responses` resource. `scripted` selects offline fixtures and performs no credential lookup. Unknown selectors fail configuration validation.

`base_url` is an HTTPS provider base URL without embedded credentials, query, or fragment. Strip trailing slashes, retain any configured version prefix (for example `/v1`), and append `/chat/completions` exactly once; if already present, use that resource unchanged. `model` is the exact provider identifier and MUST agree with the authorized Definition's inference profile. `api_key_env` is a variable NAME, never a secret value. `stream` controls provider streaming and host presentation, not guest admission or checkpoint boundaries. Headless `--inference scripted` explicitly selects offline fixtures instead of the live adapter.

`local-trusted` names the actual selected integration; it does not disable checks. A future `jam-service` cannot be enabled by a string alone: its adapter, ABI, and security binding must exist. With identical Definition/input/replies, replacing storage or inference transport does not change state bytes.

### 18.5. Minimal TUI and headless commands

The native interactive frontend MUST use Ratatui with Crossterm and exactly two primary regions: a scrollable conversation history of user prompts/assistant responses, and an editable prompt input area. The frontend MUST NOT add sidebars, dashboards, permanent status panels, metrics, animations, or complex navigation. Input supports ordinary text editing, submission, and continued interaction. History follows new responses when at the end; scrolling back preserves the user's position until they return to the end. Provider errors may appear as ordinary history entries.

The displayed history is ephemeral host-side presentation state, separate from the guest's bounded activation trajectory. It MUST NOT be checkpointed or archived merely to support scrolling; restored agent continuity on restart does not restore previous visual history. Optional streamed text is provisional presentation only (§18.3); it neither mutates guest RAM nor changes committed guest state.

History entries carry a visible role (operator, agent, Memory, status, error) that model text cannot imitate: continuation lines of a model reply never occupy the role position. On start the frontend MAY show the restored checkpoint and the accepted Memory (§8.4); after each accepted boundary it MAY add one line naming the revision, abbreviated root, model calls and remaining approved prompts, and, when Memory changed, the added, changed and removed entries. These are bounded history entries, not a permanent status panel, and they never restore conversation text. Messages that close the prompt state the reason and the operator's next step without implying acceptance, retry or replay.

A terminal prompt becomes an ActorWake only if the host's registered Actor/admission policy authorizes that terminal principal. The host assigns an event ID and pins the observation/snapshot/budget, then applies the same authority, phase, bounds, and consumption checks as any other event. User text is bounded observation data, not a mandate, system instruction, permission grant, or Receipt. AwaitAction remains busy; prompts may be queued/coalesced outside core, never used to bypass the pending action or start a parallel episode. Unsupported prompts are rejected, not granted unrestricted authority.

Every command that admits, adopts or executes takes explicitly selected pins rather than discovering a current head:

```text
raiv-agent init | status | world-status --config <file>
raiv-agent init-world --pin <main.toml> --config <file>
raiv-agent interactive --grants <plan.toml> --pin <main.toml> --world-pin <hex> --config <file>
raiv-agent wake --event <input.cbor> --grant <grant.cbor> [--replies <script.cbor>] --pin <main.toml> --world-pin <hex> --config <file>
raiv-agent execute-pending --plan <plan.toml> --pin <main.toml> --world-pin <hex> --config <file>
raiv-agent resume --receipt <input.cbor> --grant <grant.cbor> [--replies <script.cbor>] --pin <main.toml> --world-pin <hex> --config <file>
raiv-agent run-scenario retry-and-sleep --inference scripted --config <file>
```

The availability milestone adds `export --root <cid> --out <car>`, `verify --car <car> --expected-root <cid>`, `import --car <car> --expected-root <cid>` and `run-scenario provider-loss`. `--replies` selects scripted inference; without it, live inference uses the configured provider.

Ratatui is the interactive frontend, not a replacement for noninteractive initialization, tests, verification, CAR operations, or scripted scenarios. Headless event/receipt files are exact canonical records. Import validates/places bytes and does not replace the anchor without a separate authorized decision. Headless status invokes no inference and reports root, revision, episode, phase, pending action, budget, storage readiness, admission states and the accepted Memory as a single-line compact JSON `memory=` field (§8.4); it does not add a TUI status region.

### 18.6. Local sessions

A reference **local session** is one directory holding exactly one agent: its configuration and configuration digest, the pinned guest module, the finite grant plan, the block store and the synthetic world. A launcher creates a session only in an absent directory, initializes only an empty revision-zero namespace, and never infers a new session from an incomplete one.

A session is pinned. Reopening MUST run the session's own guest module, configuration and grant plan. A newer build, changed launcher default or toolchain update MUST NOT rebind, widen, migrate or orphan an existing session; the launcher MAY report that the current build differs. Using another provider, model, mandate or module requires a new session with a new Definition and registration, not a rewrite of the old one. A configuration whose digest no longer matches is refused.

Each launch shows the exact session, configuration and guest digests, accepted root and revision, world and finite plan, and requires explicit operator approval. Within that approval each submitted prompt is a separately admitted ActorWake, bounded by the plan's prompt count and per-prompt model-call, unit and action grants. A launch that ends with an unresolved admission (for example a failed activation after launch) leaves the session refusing further prompts until the admission is explicitly resolved; the launcher never retries, refunds or replays automatically.

The reference host derives a session's Definition from its compiled guest profiles. A guest change that alters an existing Definition's bytes therefore requires new sessions until the host reads the pinned Definition from the session closure; such a change MUST be additive, as new profiles are.

## 19. Acceptance scenarios

### 19.1. Retry → success → sleep

1. Register survivor genesis and Wake W1.
2. In the WASM guest, object-patch intents and tactics → trade A1 → checkpoint C1/AwaitAction; accept, then destroy the instance. Only normalized null-free Memory is stored; no effects before acceptance.
3. The executor returns a scripted failure R1, recorded independently of the agent.
4. A fresh WASM instance restores accepted C1, receives admitted R1, changes parameters → A2 → C2; accept, then destroy the instance.
5. A2 succeeds → R2; a fresh guest updates strategy and requests set_wake A3 → C3; accept, then destroy the instance.
6. R3 confirms the Actor update; the agent object-patches memory, removing completed tactics with null as appropriate → Finish → C4/AwaitActor. The other surviving planes remain; Finish does not clear memory.
7. History contains one closed Episode with three commands/receipts and final state; transcript/reasoning/response are absent.
8. The Actor emits W2 under the new condition without a user prompt. The next Episode receives the preserved experience.

### 19.2. Primary-provider loss

Repeat with two independent Blocks adapters. After C2, delete the entire local agent directory and disable the primary source. Retain the authoritative anchor and independent executor ledger. Restore the same required closure from the second source, obtain the identical root, and continue using known R2. A2 must not execute again. Then make one nested block unavailable from all sources: inference must not start and the root must not change.

### 19.3. Compact on-chain anchor

The mock acceptance layer stores only revision/profile/root digest plus registry. Reconstruct the CID and compare it with the original root. Two candidates for one revision/input cannot both be accepted. The trusted output's Command cannot be substituted while retaining the root. Importing an old CAR must not overwrite the current anchor.

### 19.4. Live model

Run the same virtual-world scenarios through live providers as a separate smoke test: DeepSeek through `opencode-go`, and at least one model of another family through `openai-compatible`. Do not require the scripted model's exact decisions or profitable behavior. Require whitelist enforcement, correct phases, bounded execution, and restoration. Missing API access is skipped, not passed. IPFS smoke is also separate; local CAR roundtrip does not prove external-network operation.

## 20. Mandatory test matrix

There are 96 testable requirements. This is an acceptance plan, not a claim of passing Rust tests. C/L/H cover codec, lifecycle, and host behavior; T and D cover trust, anchoring, and availability; W covers the WASM reference interface; M covers sparse semantic Memory and object-patch behavior.

### 20.1. Codec and graph

| ID | Property |
| --- | --- |
| C01 | Golden bytes/CIDs match across all supported targets |
| C02 | Map insertion order does not affect serialization |
| C03 | Duplicate keys, noncanonical encodings, and trailing bytes are rejected |
| C04 | Missing/wrong blocks or schemas cannot produce partial restoration |
| C05 | Depth/count/byte limits apply before unsafe allocation |
| C06 | Root.memory/phase/Episode agree, including genesis |
| C07 | A closed Episode has correct pairing and final Finish |
| C08 | Closure is bounded without a hidden previous-root chain |
| C09 | Memory is reused; local patches are not journaled |
| C10 | CAR and two Blocks adapters preserve exact roots |
| C11 | Unsupported CID/base/hash/lengths are rejected |
| C12 | Tag 42 and embedded CIDs are fully validated |
| C13 | A raw block cannot replace a structural kind |
| C14 | All floats are rejected, including finite float64 |
| C15 | UTF-8, b/aa ordering, and integer/tag/length boundaries are covered |
| C16 | A CAR header cannot substitute the expected root or closure |
| C17 | Frame order does not affect the root; unsupported formats are rejected |
| C18 | Another codec implementation yields identical bytes; not just self-roundtrip |
| C19 | Compact JSON presentation of model values parses with an independent JSON parser to the same value, keeps canonical key order, escapes controls, and never feeds back into state |

### 20.2. Lifecycle

| ID | Property |
| --- | --- |
| L01 | The model/local-tool loop writes no checkpoints before a boundary |
| L02 | An external tool emits a candidate and seals Activation |
| L03 | Core never calls the executor |
| L04 | A receipt preserves EpisodeId; Actor after Finish changes it |
| L05 | Mismatched phases/inputs/receipts are rejected |
| L06 | Stale/duplicate replies or replies for another request/instance do not advance activation |
| L07 | No-op without commands closes a zero-action Episode |
| L08 | Complete no-call replies Finish; partial text, refusal, timeout and budget failure never Finish |
| L09 | Invalid or over-limit object patches leave RAM memory unchanged |
| L10 | Multiple calls are rejected before any effect |
| L11 | Read-only data and authority are inaccessible to memory tools |
| L12 | A memory-disabled profile creates no fictitious memory |
| L13 | Episode counters survive resume |
| L14 | A new activation requires no transcript/provider reasoning |
| L15 | A retry after terminal failure creates a new ActionId |

### 20.3. Host and crash handling

| ID | Property |
| --- | --- |
| H01 | Crash before boundary: old root, zero commands |
| H02 | Upload failure: old root, retry the same candidate |
| H03 | After acceptance before dispatch: recover the instruction |
| H04 | After effect before receipt: recover the result, not repeat the effect |
| H05 | After receipt before boundary: reuse the same admitted input |
| H06 | Two candidates: one acceptance; the loser cannot dispatch |
| H07 | Redelivery cannot debit simulator balance again |
| H08 | Unknown outcome never becomes failure |
| H09 | Missing block: no inference or genesis substitution |
| H10 | Actor during AwaitAction does not create a second episode |
| H11 | History is grouped by Finish |
| H12 | Lost inference attempts remain in host accounting |
| H13 | Secrets are absent from guest memory/model context/state/export/ordinary logs |
| H14 | Restoration does not roll back external world or authority |
| H15 | A manifest crash leaves a complete old or new decision |

### 20.4. Trusted execution and on-chain anchoring

| ID | Property |
| --- | --- |
| T01 | TrustedRun binds program, Definition, anchor, and input |
| T02 | A storage-supplied root is not accepted as trusted output |
| T03 | A 32-byte digest plus pinned profile reconstructs the exact CID |
| T04 | Revision/root CAS and input consumption are coherent |
| T05 | The output command matches Command inside Boundary |
| T06 | Dispatch cannot precede required finality |
| T07 | Another execution_domain/Definition does not gain authority |
| T08 | Transient VM continuation resists substitution/replay; loss restarts activation |

### 20.5. Availability and recovery

| ID | Property |
| --- | --- |
| D01 | Readiness covers the full closure, not only the root block |
| D02 | Primary failure after acceptance permits restoration from secondary |
| D03 | False/corrupt replica data is rejected without changing the anchor |
| D04 | Loss of all copies means unavailable, not new empty memory |
| D05 | Retention covers waiting; renewal does not depend on agent wake |
| D06 | GC never deletes the current root closure or pending-action dependencies |
| D07 | Storage outage does not erase the status of an executed command |
| D08 | Storage costs/status enter observation and host accounting |

### 20.6. WASM reference application

| ID | Property |
| --- | --- |
| W01 | Rust guest compiles to raiv-agent.wasm for wasm32v1-none with no_std + alloc and no imports |
| W02 | Wasmi 2.x executes without WASI; finite memory/fuel limits and traps never create checkpoints |
| W03 | Versioned CBOR ABI rejects malformed/version/type/length/depth/count/ownership/range errors before advancement |
| W04 | Identical code/Definition/admitted inputs/replies yield equivalent native/WASM transitions, Commands and exact checkpoint bytes/CIDs |
| W05 | Multi-call inference yields retain one live Activation and RAM patches/trajectory/counters without durable boundaries |
| W06 | Destroy/reinstantiate after accepted Command/Finish restores continuation and receipts without transient RAM or transcript serialization |
| W07 | Two-region Ratatui/Crossterm supports editing/submission, scroll-back position and latest-follow; restart retains agent continuity, not visual history |
| W08 | Host SSE presentation never advances guest with partial data; interruption/malformed completion fails without a successful reply/checkpoint |
| W09 | Protocol aliases, endpoint/path resolution, exact model/Definition agreement and unsupported-configuration errors are tested |
| W10 | Live credentials come exclusively from api_key_env; missing variables fail and secrets never reach guest memory/context/state/CAR/logs |
| W11 | Offline scripted scenarios, prompt authority/phase rejection and TUI tests use the same guest ABI with no network or API key |
| W12 | Provider families send only their own extensions; dated model snapshots match while other names fail; empty-id stream continuations are accepted and different ids rejected; generic reasoning is never replayed |
| W13 | Local sessions reopen with their own pinned guest, configuration and plan; newer builds or defaults never rebind or widen them; changed configuration and unresolved admissions refuse |
| W14 | Restored Memory and per-boundary Memory changes are shown as role-labelled presentation; status prints compact `memory=` JSON; no conversation text is restored |

### 20.7. Sparse memory and object patches

| ID | Property |
| --- | --- |
| M01 | Only exact intents/strategy/tactics keys are allowed; each optional present plane is an object; restore/projection never create absent planes |
| M02 | Recursive merge preserves omitted fields; scalars/whole arrays replace; objects merge into an object or replace a prior non-object |
| M03 | Object-field null deletes fields/subtrees/planes; absent deletion succeeds without creating ancestors |
| M04 | Empty object merge does not clear nonempty objects; whole-draft normalization removes empty object fields, including unmentioned branches, and cascades to planes |
| M05 | False, zero, empty text and empty arrays survive; null anywhere in selected replacement array data rejects before normalization, including nested object fields |
| M06 | Objects inside arrays normalize recursively without removing array positions; resulting empty object elements remain |
| M07 | Persisted Memory roundtrip/negative fixtures enforce no null at any depth and normalized object fields; other schemas retain their permitted nulls |
| M08 | Unsupported/alias top-level roots are skipped with reasons, never remapped; selected supported roots apply or may be created; invalid selected content/roots/duplicates reject without mutation |
| M09 | Nested dots/slashes/[0]/$ref are literal keys; identical keys in different planes have no cross-plane effect |
| M10 | Normalization and accepted patch replay are idempotent; key insertion order does not affect canonical bytes/CIDs |
| M11 | Global invalid input or late output/result/context failure preserves original working bytes/CID/assignment and accepted root; unsupported top-level roots do not reject admissible supported-root patches |
| M12 | Full input/traversal/draft/output byte/depth/entry bounds apply before unsafe allocation, even when normalization would produce a tiny result |
| M13 | Results expose complete accepted Memory, processed/skipped reasons and actual change; partial/all-skipped/no-effect patches remain Continue without forced retry, reuse equal bytes/CID and debit admitted work |
| M14 | Enabled empty memory is a linked Memory with value {}; disabled memory has null links and no patch capability or fabricated planes |
| M15 | Patches retain the live Activation, immutable prefix and RAM trajectory; only existing boundaries store materialized Memory, with no durable patch log |
| M16 | Finish preserves surviving memory into the next Episode; deleting tactics neither cancels accepted Commands nor substitutes for AwaitAction/Receipt continuation |
| M17 | Rejected patches return bounded `invalid_patch` feedback naming the violated rule and expected shape, without repairing or reinterpreting the value |

## 21. Implementation validation and compatibility boundaries

Native unit/property tests, fuzzing of decoder/patch/tool replies/graphs and ABI frames, and no_std builds without hidden std features are mandatory. Seeded mutation of golden vectors inside the ordinary suite is a useful reproducible baseline; it does not replace coverage-guided fuzzing of ABI frames and graphs. CI MUST compile the Rust guest for `wasm32v1-none` and run it under Wasmi 2.x without WASI, including native/WASM transition and checkpoint equality for identical admitted inputs/replies. Library MSRV and existing `wasm32-unknown-unknown`/bare-metal gates remain. A WASM smoke test does not prove JAM/PVM readiness.

Pin the DASL conformance suite by revision. It includes multiple, sometimes conflicting CBOR profiles; use relevant cases and stricter Raiv negative tests rather than requiring every unrelated row to pass. [S13] Self encode/decode roundtrips are insufficient; independent canonical bytes and CID vectors are required.

Golden fixtures MUST cover both genesis profiles, memory patch, action Boundary, terminal Receipt, closed Episode, full root, command/episode ID domains, compact-anchor reconstruction, and CAR. Fixture strings are data and must not be translated at test time.

CI requires no secrets or real financial systems. Live inference/IPFS tests are separate, with distinct skipped/failed/passed states. Results are reported only after actual execution. Offline scripted and TUI tests exercise the guest ABI without network or credentials. Checking this specification's text does not execute the C/L/H/T/D/W/M tests.

Measure RAM high-water marks, dependency graph/binary size, and restore/encode time against pinned profiles and targets. Minimalism comes from removing unnecessary subsystems, not an unverified kilobyte claim.

## 22. Implementation order and completion criteria

| Milestone | Deliverable | Gate |
| --- | --- | --- |
| 1. Format contract | Six typed blocks, checked DaslCid, ID derivation, strict codec, genesis, exact fixtures, value presentation | C01–C09, C11–C15, C18–C19; no_std spike |
| 2. RAM core | Projection, sparse object-patch memory, single-tool loop, yield/Finish, limits, scripted inference | L01–L15, M01–M17; no I/O inside the loop |
| 3. WASM guest and step bridge | Rust guest, Wasmi 2.x, bounded CBOR ABI, scripted replies, live activation | W01–W05; scripted native/WASM equality, no WASI |
| 4. Local trusted host | Registry/anchor, admission, atomic acceptance, simulator ledger, Actor, fresh-instance recovery | H01–H15, T01–T08, W06; crash scenario |
| 5. Availability and transport | CAR, second store, readiness/retention, primary failure | C10, C16–C17, D01–D08 |
| 6. Minimal interactive/live agent | Veto/Survivor, two-region Ratatui/Crossterm frontend, headless commands, provider profiles, local sessions; IPFS module | W07–W14; separate live smoke, credential/stream checks |

A complete `raiv` passes model-independent format/lifecycle tests, performs no I/O, retains no transcript, and has bounded restoration and phase/authority guards. A complete `raiv-agent` passes W01–W14 and demonstrates action→failure→retry→success→set_wake→receipt→Finish inside Wasmi, with live activation retained across inference and fresh-instance restoration at accepted checkpoints, then another ActorWake and provider loss. Both Veto and Survivor remain supported. The two-region TUI and headless scripted paths share guest semantics; live inference is configurable and credentials stay native. Intrinsic no-tool completion changes the decision trigger, not one-scope memory/history policy, identifiers/hash domains, state schemas, `raiv-state/1`, DASL/DRISL/CAR, Command/Finish boundaries, acceptance-before-effects, durable receipt continuation, or the separation of integrity, availability, and execution trust.

A blockchain deployment additionally requires a concrete host ABI, registration/authority mapping, finality, external inference inputs, and storage policy. A no_std build alone does not make it ready. None of these integrations require changing the nature of memory or adding a core verification network.

## 23. Final contract

**The agent authors memory and decisions. Core turns them into a bounded reproducible checkpoint. The trusted host accepts execution output. The blockchain defines the canonical root sequence. Storage makes the corresponding bytes available.**

Raiv adds no separate verification layer for its own sake. Execution trust nevertheless cannot replace data retention or eliminate checking that restoration recovered the exact anchored bytes.

## 24. Sources and verification status

Design requirements are defined in this document; they are not presented as existing external-library features. The primary sources below were checked on 10 October 2026 and support limited format/interface statements. No complete implementation audit, ecosystem audit, or provider-SLA assessment was performed.

DASL CID, DRISL, and CAR publish revisions dated 2026-10-01. The implementation repository must pin exact snapshots/revisions and fixtures rather than dynamic URLs. JAM is an architectural reference, not a claim that a Raiv service exists. Pi Durable is conceptual inspiration, not a dependency or API contract for this implementation.

- **[S1] DASL CID:** https://dasl.ing/cid.html
- **[S2] DRISL:** https://dasl.ing/drisl.html
- **[S3] DASL CAR:** https://dasl.ing/car.html
- **[S4] RFC 8949, deterministic encoding:** https://www.rfc-editor.org/rfc/rfc8949.html#section-4.2.1
- **[S5] IPFS persistence:** https://docs.ipfs.tech/concepts/persistence/
- **[S6] CBOR backends:** https://docs.rs/minicbor/2.3.0/minicbor/ ; https://docs.rs/ciborium/0.2.2/ciborium/
- **[S7] Rust Reference, no_std:** https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-attribute
- **[S8] Parity, JAM architecture:** https://www.parity.io/blog/JAM-demystified-explainer ; https://jamcha.in/tech
- **[S9] DeepSeek tool calls:** https://api-docs.deepseek.com/guides/tool_calls/
- **[S10] DeepSeek thinking mode:** https://api-docs.deepseek.com/guides/thinking_mode/
- **[S11] Kubo RPC:** https://docs.ipfs.tech/reference/kubo/rpc/
- **[S12] DASL IPFS interoperability:** https://dasl.ing/ipfs.html
- **[S13] DASL test matrix:** https://hyphacoop.github.io/dasl-testing/

**Implementation status:** this document is normative. Implementation progress, evidence and open work live in CHANGELOG.md and BACKLOG.md; only executed C/L/H/T/D/W/M checks, not this text, determine implementation readiness.
