# Raiv 0.0.0
## Specification of the minimal Rust library and reference agent

**Date:** 10 October 2026. **Status:** implementation specification, not an implementation report.
**Pronunciation:** /reɪv/, like *rave*.
**Components:** `raiv` — library; `raiv-demo` — application.
**State format:** `raiv-state/1`, DASL CID + DRISL; CAR is host-side transport.

> Raiv preserves an agent's behavioral continuity across short executions. A trusted execution host provides execution and canonical acceptance of transitions; a blockchain anchors the root; external storage retains the actual data.

**MUST / MUST NOT** denote requirements; **SHOULD** denotes a recommendation whose deviation must be documented; **MAY** denotes an option. Requirement owners: **core** (library), **host** (accepting environment), **adapter** (an integration module), **demo** (reference application).

This is a self-contained specification for the first Raiv version. It does not publish a crate, reserve a name, implement a JAM service, or perform real financial operations.

## Contents

1. Decisions in revision 0.0.0
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
16. Rust, dependencies, and VM integration
17. Errors and resource limits
18. Reference agent outside a blockchain
19. Acceptance scenarios
20. Mandatory test matrix
21. Implementation validation and compatibility boundaries
22. Implementation order and completion criteria
23. Final contract
24. Sources and verification status

## 1. Decisions in revision 0.0.0

Raiv separates three boundaries: execution trust, on-chain anchoring, and off-chain availability.

| Area | Decision |
|---|---|
| Execution | A trusted host runs the selected Raiv code; core contains no separate execution-proof protocol |
| Canonical state | In the blockchain profile, the accepting network stores the current revision and root digest |
| Content | A DRISL graph in external storage; IPFS is a target adapter, not an embedded node |
| Checkpoint | At an authorized external command and explicit `no_op` |
| Continuation | A receipt resumes the same episode; ActorWake after `no_op` starts another |
| Text session | RAM of one activation only |
| Persistent memory | Optional `strategy` + `intent`; one scope |
| History | Commands, outcomes, and states grouped up to `no_op` |
| Excluded | Persistent transcript, `response`, `lazy`, reasoning archive, task scheduler |
| Inference | External input through the host, with no built-in model consensus |
| Portability | `no_std + alloc` as a tested target; adapters remain outside core |
| Availability | A contract for the entire required graph, including retention, recovery, and failures |

The document version is 0.0.0. The first state format is `raiv-state/1`. Changing format strings and hash domains changes content, IDs, and roots.

## 2. Trust and guarantee model

### 2.1. Trusted execution is a premise

Raiv's baseline profile assumes that the host enforces execution of an **authorized code version**, integrity of its live continuation, and acceptance of the output of that execution. Locally, this is a trusted process. A distributed deployment may obtain this property from an execution-verifying protocol, such as a JAM-style service environment. This does not mean trusting an arbitrary RPC node or assuming a hardware TEE.

**CORE MUST NOT require its own consensus, commit–reveal, quorum, fraud proofs, or execution ZK proofs.** When the host already supplies execution assurance, a duplicate layer is unnecessary. JAM denotes a target class of hosts; its exact ABI, admission mechanism, and handling of external inputs require a separate integration. Official JAM material separates processing work from accumulating results into state; this specification does not present that as a completed Raiv integration. [S8]

The LLM authors mutable memory. Core does not judge the truth of a strategy or require proof of every semantic conclusion. However, the model does **not** author an arbitrary system checkpoint: Raiv code controls phases, commands, IDs, receipts, links, and serialization.

### 2.2. Responsibility allocation

| Property | Owner |
|---|---|
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

**The `raiv-demo` application:** a local host, disk blocks, CAR, scripted inference, then a live DeepSeek adapter, and simulated executor and actor. IPFS is a separate module. Baseline CI does not depend on a particular gateway or a paid API.

**Out of scope:** an IPFS/P2P node or blockchain consensus; a wallet/keys in core; mainnet actions; DAG schedulers, subagents, multi-scope memory, CRDT/fork/merge; shell/browser/filesystem tools; online learning; MASL/RASL/FISL; arbitrary hash/codec registries; automatic migration; a complete JAM adapter. Pi Durable's entire feature set need not be ported: inspiration is limited to preserving continuation before an external effect.

Expected workspace structure:

```text
raiv/
  crates/raiv/src/
    lib.rs  activation.rs  model.rs  tools.rs  memory.rs
    state.rs  projection.rs  format.rs  limits.rs  error.rs
  apps/raiv-demo/src/
    main.rs  config.rs  host.rs
    storage/{memory,disk,ipfs}.rs
    transport/car.rs
    inference/{scripted,deepseek}.rs
    executor/simulator.rs
    actor/simulator.rs
  profiles/{veto,survivor}/
  fixtures/{format,lifecycle,failures}/
  specs/
```

## 4. Terminology and invariants

**Agent:** one identity, mandate, and canonical memory lineage; not a process or a version CID.
**Episode:** a session from ActorWake to an accepted `no_op`.
**Activation:** the RAM segment of an episode until an external command, `no_op`, or error.
**Boundary:** the persisted input and outcome of an accepted activation.
**Checkpoint:** state for semantic continuation, not a stack or serialized future.
**Command:** an execution instruction; **Receipt:** its terminal outcome.
**CommitCandidate:** an unaccepted root and new blocks.
**Required closure:** every typed block reachable from the root and required for restoration.

| ID | Mandatory invariant |
|---|---|
| INV-01 | One scope, one current root, at most one pending command |
| INV-02 | Dispatch only after accepting the checkpoint containing the command |
| INV-03 | The model changes only memory, not read-only facts or control fields |
| INV-04 | Memory is optional; control continuation is mandatory |
| INV-05 | Model replies and local tools do not create individual persistent checkpoints |
| INV-06 | Only an accepted `no_op` closes an episode |
| INV-07 | After suspension, construct a new projection rather than restore the transcript |
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
  -> no_op -> checkpoint -> accept -> close episode
  -> AwaitActor
```

| Phase | Input | Reaction |
|---|---|---|
| AwaitActor | New authorized ActorWake | Start a new episode |
| AwaitAction(A) | Terminal ActionResult(A) | Resume the current episode |
| AwaitAction(A) | ActorWake | Busy; queue/coalesce outside core |
| AwaitAction(A) | ActionResult(B), B ≠ A | Reject |
| AwaitActor | Old ActionResult | Duplicate/stale; do not activate |

`AwaitInference` is not a persistent lifecycle state. A RAM activation MAY yield to the inference port without a durable checkpoint. It ends at an external command; that command's result arrives in a new prepared packet, not as a native tool-result appended to the old transcript.

### 5.2. Initialization and admission

An authorized host creates genesis: `revision = 0`, `AwaitActor`, `current_episode = null`, `history = []`, the selected Definition, and initial Memory or `null`. Initialization neither invokes the model nor invents an episode. Repeated initialization does not replace an existing agent.

The host pins a particular root, event ID, observation snapshot, budget, and receipt. Recovery uses the same admitted input. It MUST NOT silently reevaluate the event against another snapshot. A refreshed observation after failure requires an explicit admission decision, not an incidental wall-clock read.

### 5.3. External calls and completion

One valid external tool call produces a Command, Boundary, updated Episode manifest, and AgentState/AwaitAction. The Activation is sealed. The command does not execute before acceptance, and no fictitious `prepared` model result is returned. A local validation error is returned to the model without a checkpoint.

A receipt reporting success, failure, or confirmed non-execution resumes the episode. A deliberate retry is a new command, not redelivery of the old one.

`no_op()` commits the final memory, closes the Episode, and enters AwaitActor. It neither sends an empty extrinsic nor removes the existing schedule. Empty text, timeout, truncation, and budget exhaustion are not `no_op`.

## 6. Memory, facts, and history

### 6.1. Ownership separation

Definition contains the mandate and policy. Observation contains current world data. Receipt records an outcome. History contains accepted prior states and actions. These are model-read-only but have distinct provenance and instruction priority. An external document does not become a system instruction merely because it is immutable.

The standard mutable memory is an object with required map planes `strategy` and `intent`. The agent chooses their internal keys within schema and size limits. `strategy` retains working rules, evidence, and revision conditions; `intent` retains unfinished goals and continuation purpose. The veto profile MAY disable memory entirely; `patch_memory` is then absent.

There are no dedicated `response`, `lazy`, `reasoning`, or `transcript` planes. This is not a ban on using a word inside a note. Runtime records action facts; the model interprets them. Revising an interpretation does not alter a historical receipt. An important rule must survive in current memory before its originating episode leaves the window.

### 6.2. Patching

`patch_memory` accepts `set`/`remove` operations whose paths are arrays of strings relative to memory:

```json
{"operations":[
  {"op":"set","path":["intent","next"],"value":"Inspect the next action receipt before retrying."},
  {"op":"remove","path":["strategy","obsolete"]}
]}
```

Empty paths, unknown planes, read-only targets, duplicate paths, and ancestor/descendant overlaps within a patch are forbidden. `set` replaces a value and may create missing maps; traversing a non-map is invalid. Removing a missing key succeeds; a required plane cannot be removed but may be cleared with `{}`. `null` is a value, not deletion. Arrays are replaced as a whole; paths do not index them.

The entire patch and resulting bounds are validated before mutating RAM. Failure leaves memory unchanged. A successful result contains the complete compact working memory; this is RAM acceptance, not persistence. The demo registers no separate `read_memory` tool.

### 6.3. History projection

A completed Episode is the unit of history, not an extrinsic. The current open Episode contains the action/result sequence and latest memory. An Episode with N commands finishes with N+1 Boundaries, including the final `no_op`.

By default, the model receives entry/final memory of two completed episodes and their commands and results, followed by the current action chain and memory. A narrow profile may disable historical projection. The persistent completed-Episode window retains at least one element after the first completion so the root retains its latest Boundary; genesis history is empty before that. A full archive of old roots is external.

Earlier manifests do not link to one another. The required closure MUST NOT silently expand into an unlimited biography. Archive, prompt, and canonical current memory are different representations. There is no model-facing selective `read_history` or network hydration inside an activation.

## 7. RAM loop and model-facing tools

A local invocation has three control effects: `Continue` (patch/pure operation), `Yield` (Command), or `Finish` (`no_op`). An application tool is a trusted pure planner: it receives bounded arguments and authorized context, validates the schema, and returns a command without signing or sending it. In native Rust, planner purity is a code contract, not a language-sandbox property.

The Definition pins the whitelist, tool versions, and authority binding. The model cannot enlarge them through text or patches. A broad chain profile may expose many named operations; core need not offer `submit_any_extrinsic`. A compound operation with defined semantics does not imply automatic parallel tool execution.

A ModelReply may contain at most one tool call. Multiple calls are ALL rejected before any patch or Command; the adapter returns provider-correct error results for the rejected calls. Plain text without a call gets a bounded corrective attempt; exhausting that limit produces `MissingControlDecision`, not `no_op`.

Projection order is fixed: policy/mandate → tools → cause/observation/receipt → completed history → current action chain → memory → RAM trajectory. A machine event is not presented as a human request. A provider may encode it as user-role, but its content identifies a machine input. A post-checkpoint receipt is new input data, not an orphan tool message without an assistant call.

Patches do not rewrite the activation's initial prefix. New results append to RAM. Text, native call IDs, and provider continuation fields disappear at a durable boundary. External diagnostic retention requires an explicit private host policy and is not necessary for restoration.

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

A block contains exactly one complete object. Integer/length/tag encodings are minimal, lengths definite, map keys strings, and key ordering lexicographic over **CBOR-encoded keys** (`"b"` before `"aa"`). Duplicate keys, unknown structural fields, trailing bytes, invalid UTF-8, and unauthorized tags are rejected. Unicode is not normalized. Optional schema fields use explicit `null`, never missing keys.

Raiv forbids all Float values, including finite float64; this is narrower than DRISL. Counters are overflow-checked `u64`. Every map/list has explicit limits. Model-authored values contain only null/bool/integers in `−(2^53−1)..(2^53−1)`/strings/lists/maps. Bytes and Link belong only to runtime schemas, not arbitrary memory, observation, arguments, or result. Monetary quantities are decimal strings of integer base units; canonical unsigned form is `0` or `[1-9][0-9]*`. Tool schemas specify bounds, signedness, and units.

Decoding MUST preserve duplicate-key errors and enforce byte/depth/entry limits before allocation. Decode → canonical encode with exact byte comparison is useful defense in depth, not a substitute for bounded parsing. BTreeMap alone does not prove correct serialization order.

### 8.3. Canonical graph

Structural blocks are limited to the six kinds in §9. Core creates Links; a generic IPLD value/codec registry is not public API. Protocol IDs and evidence references are scalar bytes/text, not implicit dependencies on external proof archives.

The root has no `previous_root` link; an Episode never links to earlier versions of its own manifest. The required closure contains the definition, current memory, open episode, and bounded completed history. Decoding rejects cycles/repeated semantic positions and wrong types; an identical immutable block under one CID is loaded once.

## 9. State data

The following schemas are normative for field membership, presence, and meaning. The notation describes records/maps, not executable Rust. Every listed key is required; `?` means `null` or the specified type. Complete byte fixtures for nested descriptors/profiles must precede encoder stabilization, without implicit Serde defaults.

`Id32` means exactly 32 raw bytes, not a Link. `Text` is bounded UTF-8. `Value` is the constrained JSON-compatible value in §8; `Map` has string keys and Value values. `Position` = `{domain: Id32, number: u64, reference: Id32?}`; it is a normalized host position, not necessarily a block number. `Usage` = `{model_calls: u64, local_tool_calls: u64, input_tokens: u64?, output_tokens: u64?, billed_units: u64?}`. Unknown cost is null, not zero.

`BudgetAdmission` = `{admission_id: Id32, activation_units: u64, remaining_episode_units: u64, remaining_model_calls: u64, remaining_actions: u64}`. A host profile defines the unit; it is not a universal token-to-money conversion.

### 9.1. Six structural blocks

```text
Definition {
  kind: "definition", version: 1,
  execution_domain: Id32,
  policy: Text, mandate: Text,
  memory_profile: "none" | "strategy-intent/1",
  tools: list<ToolDescriptor>,
  inference_profile: Map,
  projection_version: 1, normalizer_version: 1,
  limits: Limits
}

Memory {
  kind: "memory", version: 1,
  value: {strategy: Map, intent: Map}
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

`ToolDescriptor` contains `id`, `version`, `control`, `description`, `arguments_schema`, and `authority_binding`. Every field is required; the schema is a bounded Map, and authority_binding is Id32 or null for local tools. Control is `continue`, `yield`, or `finish`. Names are unique and their order fixed; runtime planners match descriptors and the authorized code version. Core need not implement all of JSON Schema: the trusted application validator and advertised descriptor must agree.

Definition contains no secret-bearing URLs, headers, or keys. A changed mandate, tool semantics, or logical model selection requires an administrative new Definition, not a memory patch. Version 0.0.0 does not support live upgrades during an episode. Limits are enumerated in §17; deployment cannot silently change them after genesis.

### 9.2. Consistency checks

Episode ordinals start at 0 and are contiguous. The first Boundary has cause `actor_wake` and a null receipt; subsequent boundaries have `action_result` and the previous command's matching terminal Receipt. Action index counts preceding `await_action` boundaries in the episode, starting at 0.

AwaitAction requires an open current Episode ending in the matching command. AwaitActor has current_episode = null; except at genesis, the last history Episode is closed and has a final `await_actor`. Only the last Boundary may contain `await_actor`. Root.memory equals the last accepted Boundary.memory; the next episode's entry_memory equals memory immediately before that episode. All memory links are null under memory_profile=none.

Unchanged Memory blocks are reused. Each accepted boundary increments revision by exactly 1; overflow is rejected. New blocks do not contain their future acceptance timestamp or their own acceptance-event hash, which would create a dependency on the future. The host retains those facts.

## 10. Commands, receipts, and stable IDs

`execution_domain` distinguishes deployment/network/service. The host assigns it and binds it to agent registration; transferring bytes to another network does not transfer authority. Registration assigns AgentId, which is not the mutable root. A local demo may deterministically derive Id32 from a domain and human-readable label without a signer.

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

`set_wake` or a DEOS Actor update is another external command with checkpoint and receipt. Intent text does not schedule anything. After `no_op`, the previously accepted Actor condition applies. The survivor simulator starts with a fallback rule so failed rescheduling does not remove its next wake-up.

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

RegistryBinding is host metadata, not a seventh state block. The Definition is bound to the authorized program; a different Definition inside a hash-valid root does not acquire authority. The demo represents both records in a local manifest.

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

Storage readiness binds to the candidate, accepted policy, and exact dependency set. Acknowledging only the root block is insufficient. A local demo may use verified crash-durable local writes, clearly labeled local-only. A replicated demo requires complete readback and verification from two independent directories/adapters; this tests failures, not independent corporate ownership.

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

HTTP endpoints, gateway discovery, P2P, pinning authentication, encryption, and leases are outside core. The demo uses public synthetic data. Content addressing does not confer confidentiality; real encryption requires a separate envelope/key-management profile without secrets in the on-chain root, prompt, or public logs. Changing transport encryption must not silently change canonical state.

## 15. Library API and four external ports

The API below specifies the shape of the contracts, not a compiled crate. The skeleton milestone makes the types executable without altering wire semantics.

```rust
pub fn restore(
    root: DaslCid,
    blocks: &impl MaterializedBlocks,
    limits: &DecodeLimits,
) -> Result<RestoredAgent, RestoreError>;

pub fn prepare(
    restored: RestoredAgent,
    input: PreparedInput,
    tools: &ToolSet,
) -> Result<Activation, PrepareError>;

impl Activation {
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

`DaslCid` is a validated type, not an arbitrary CID alias. State roots additionally require codec `0x71`. `MaterializedBlocks` reads already available bytes without hidden I/O. `PreparedInput` is constructed at the trusted host boundary; its Rust type alone does not prove provenance.

`request()` provides a borrowed view and performs no inference. ModelRequest carries a request ID, versioned projection, descriptors, normalized model parameters, and limits. ModelReply contains reply-ID binding, text, a call list whose cardinality is validated, and Usage. A provider ID never becomes a persistent agent ID by default.

The adapter retains full provider HTTP payloads only in RAM when needed. Boundary seals the Activation against further replies; fatal errors discard it. Retrying persistence of the same candidate requires no new inference. `expected_root`/input binding belong to acceptance and do not link the new graph to an unbounded past chain.

| Port | Contract | Excluded responsibility |
|---|---|---|
| Inference | ModelRequest → normalized ModelReply | Core model consensus/weight verification |
| Blocks | Exact CID bytes, materialization, and retention policy | Choosing the authoritative root |
| Acceptance | Trusted-run binding, anchor CAS, event consumption, dispatch permission | Semantic judgment of strategy |
| Executor + Actor | Deferred effects, receipts, wake conditions | Another scope or parallel episode |

`restore` validates a snapshot but does not make it authoritative. Genesis construction is a separate pure helper; only the host can register it where no anchor exists.

## 16. Rust, dependencies, and VM integration

Core targets `#![no_std]` plus `extern crate alloc`, with no filesystem, environment variables, wall clock, RNG, threads, async runtime, or network clients. `no_std` alone does not prevent std dependencies; feature-graph inspection and a dedicated build are required. [S7]

The codec is private to `format`. Candidates include a strict `serde_ipld_dagcbor` adapter or a compatible DASL implementation after testing. The former documents a no_std path, but this proves neither the entire Raiv build nor complete DRISL conformance. [S6] Crate naming does not define the wire contract. Internal `cid`/`ipld-core` dependencies are allowed if they do not broaden the public profile.

Do not implement general CBOR or cryptography from scratch. Use established SHA-256/base32 primitives with minimal features. A narrow profile validator is permitted and required when the decoder is too permissive. Pin/lock dependencies; determine MSRV, target, and binary size through the initial spike rather than promises.

**RAM lifetime belongs to the VM adapter.** Between model calls it must preserve the live Activation or an integrity-protected transient continuation. If transient RAM is handed to an untrusted service, the host must protect integrity, run/request binding, and freshness against replay; otherwise the execution-trust premise fails. This transient state is not part of `raiv-state/1`.

A VM that destroys memory while inference is external needs an appropriate ABI/host mechanism, not just a configuration switch. Losing transient state restarts the activation from the last accepted DRISL checkpoint. This does not justify adding a persistent transcript to the root.

A JAM deployment must define how inference replies/blocks enter execution and which operations its execution guarantee covers. Core neither performs HTTP in a consensus function nor assumes different executors independently obtain identical LLM replies. This is an integrator contract, not another Raiv consensus. Native and sandbox golden equality tests use identical admitted replies.

## 17. Errors and resource limits

Categories: `InputUnavailable`, `IntegrityFailure`, `UnsupportedVersion`, `WrongAgent`, `WrongDomain`, `WrongPhase`, `StaleInput`, `UntrustedRun`, `UnknownTool`, `InvalidArguments`, `InvalidPatch`, `MultipleToolCalls`, `MissingControlDecision`, `InferenceFailure`, `ContextLimit`, `BudgetExhausted`, `CommitConflict`, `StorageFailure`, `AvailabilityNotReady`, `OutcomeUnknown`.

A recoverable local tool error may be returned to the next model turn. A fatal error never creates a no-op checkpoint; the last accepted root remains, while the host retains diagnostics and admitted input. Administrative suspension is not reported as voluntary successful episode completion.

Normative Limits fields and initial demo values:

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
max_patch_operations                32
history_episodes                     2
max_actions_per_episode              8
max_model_calls_per_activation      16
max_model_calls_per_episode         64
max_local_tools_per_activation      16
max_corrective_turns                  2
```

All fields are u64 and checked before conversion to usize. These are test baselines, not optimal-performance claims. Receipt/command limits cover the entire record; context includes all messages and tool descriptors; adapters separately enforce HTTP-body and timeout limits before parsing.

Frame counts, definition sizes, DAG size, and projection MUST be validated together. Policy cannot silently truncate receipts/memory or drop episodes from the promised window. An oversized input fails preflight; changing the history window requires an explicit profile change.

An exhausted action budget disables yield tools, not the finish tool while overall budget remains. A system pause on overall exhaustion is not no_op.

Model-call and action counters accumulate across the current Episode; ActionResult does not reset them. The host separately accounts for failed paid API attempts and lost activations. Accepted checkpoint Usage is not the complete lifetime expense ledger. Token estimates are not reported as exact monetary charges.

## 18. Reference agent outside a blockchain

### 18.1. Profiles

**Veto:** a fixed mandate and prepared referendum, memory=none, and `veto`, `pass`, `no_op` tools. The simulator treats veto/pass as external commands with delayed outcomes; on-chain mappings are chosen later. After a receipt, the agent may finish or continue within policy.

**Survivor:** manage a virtual budget and preserve the ability to continue operating; `strategy`/`intent`; `patch_memory`, `trade`, `set_wake`, `no_op`. Tools provide no real funds, keys, Internet, or shell. The host performs network inference. Economic success is not a harness-correctness test.

### 18.2. Local durability

Unit tests use MemoryBlockStore. DiskBlockStore stores exact DRISL bytes indexed by CID. JSON and SQLite are not persistent agent state. One atomic host manifest contains the anchor, admission/consumption, and bindings; simulator atomicity binds world changes to the Receipt ledger.

The reference Linux implementation locks acceptance, writes temporary files, fsyncs blocks, atomically replaces the manifest, and fsyncs its directory. Guarantees apply to the tested OS/filesystem; corruption is not hidden by new genesis. A portable filesystem transaction is not attributed to `rename` without testing.

`wake` and `resume` run to exactly one boundary and exit. `execute-pending` is separate; a runner may compose these steps without adding a core scheduler. Restoring the agent directory reconnects to the same executor ledger instead of creating a world that forgot prior spending.

### 18.3. Inference and authentication

ScriptedInference is mandatory first. DeepSeekAdapter follows: configurable model ID, host-side HTTPS/bearer API key, and bounded Chat Completions requests/replies. Official tool-call support exists; integration fixtures pin the concrete API snapshot. [S9]

The initial live profile is non-streaming with explicit `thinking: {type: "disabled"}`. If thinking mode is added, provider-required continuation fields remain in RAM only; DeepSeek requires passing reasoning_content back in subsequent tool-enabled requests. A durable boundary starts a new context without the previous reasoning. [S10]

Credentials come from a named environment variable. They never enter Definition, prompt, Command, Receipt, blocks, CAR, or ordinary logs. Browser/OAuth/subscription authentication is outside v0.0.0. Changing an endpoint under the same authorized logical provider is deployment configuration; changing the model, mandate, or permissions requires an explicit Definition change.

### 18.4. Example configuration

This is the target application schema, not configuration for a released binary. Compact deployment fields expand into the full Definition/Limits at initialization; subsequent mismatches are checked rather than silently applied.

```toml
schema_version = 1
state_format = "raiv-state/1"

[agent]
profile = "survivor"
label = "raiv-survivor-demo-01"
mandate_file = "profiles/survivor/mandate.txt"

[execution]
adapter = "local-trusted"
domain = "raiv-demo/local-1"

[inference]
adapter = "deepseek"
base_url = "https://api.deepseek.com"
api_key_env = "DEEPSEEK_API_KEY"
model_env = "DEEPSEEK_MODEL"
thinking = false
stream = false

[storage]
adapter = "disk"
directory = "./var/blocks"

[availability]
policy = "local-crash-durable"

[acceptance]
adapter = "local"
manifest = "./var/host.drisl"

[executor]
adapter = "simulator"
ledger = "./var/world.drisl"
scenario = "scenarios/retry-and-sleep.toml"

[actor]
adapter = "simulator"
initial_wake_after_ticks = 1

[memory]
profile = "strategy-intent/1"
max_bytes = 4096
history_episodes = 2

[limits]
max_model_calls_per_activation = 16
max_model_calls_per_episode = 64
max_actions_per_episode = 8
max_materialized_bytes = 4194304
```

`local-trusted` names the actual selected integration; it does not disable checks. A future `jam-service` cannot be enabled by a string alone: its adapter, ABI, and security binding must exist. With identical Definition/input/replies, replacing storage or inference transport does not change state bytes.

### 18.5. CLI

```text
raiv-demo init --config demo.toml
raiv-demo status --config demo.toml
raiv-demo wake --event <file> --config demo.toml
raiv-demo execute-pending --config demo.toml
raiv-demo resume --receipt <file> --config demo.toml
raiv-demo export --root <cid> --out agent.car --config demo.toml
raiv-demo verify --car agent.car --expected-root <cid>
raiv-demo import --car agent.car --expected-root <cid> --config demo.toml
raiv-demo run-scenario retry-and-sleep --inference scripted
raiv-demo run-scenario provider-loss --inference scripted
```

CLI event/receipt files may use JSON for convenience, but admission produces exact canonical records. Import validates/places bytes and does not replace the anchor without a separate authorized decision. Status invokes no inference and reports root, revision, episode, phase, pending action, budget, storage readiness, and latest diagnostics.

## 19. Acceptance scenarios

### 19.1. Retry → success → sleep

1. Register survivor genesis and Wake W1.
2. Patch intent → trade A1 → checkpoint C1/AwaitAction; destroy RAM. No effects before acceptance.
3. The executor returns a scripted failure R1, recorded independently of the agent.
4. A new process restores C1, receives R1, changes parameters → A2 → C2; destroy RAM.
5. A2 succeeds → R2; the agent updates strategy and requests set_wake A3 → C3; destroy RAM.
6. R3 confirms the Actor update; the agent updates memory → no_op → C4/AwaitActor.
7. History contains one closed Episode with three commands/receipts and final state; transcript/reasoning/response are absent.
8. The Actor emits W2 under the new condition without a user prompt. The next Episode receives the preserved experience.

### 19.2. Primary-provider loss

Repeat with two independent Blocks adapters. After C2, delete the entire local agent directory and disable the primary source. Retain the authoritative anchor and independent executor ledger. Restore the same required closure from the second source, obtain the identical root, and continue using known R2. A2 must not execute again. Then make one nested block unavailable from all sources: inference must not start and the root must not change.

### 19.3. Compact on-chain anchor

The mock acceptance layer stores only revision/profile/root digest plus registry. Reconstruct the CID and compare it with the original root. Two candidates for one revision/input cannot both be accepted. The trusted output's Command cannot be substituted while retaining the root. Importing an old CAR must not overwrite the current anchor.

### 19.4. Live model

Run the same virtual-world scenarios through DeepSeek as a separate smoke test. Do not require the scripted model's exact decisions or profitable behavior. Require whitelist enforcement, correct phases, bounded execution, and restoration. Missing API access is skipped, not passed. IPFS smoke is also separate; local CAR roundtrip does not prove external-network operation.

## 20. Mandatory test matrix

There are 64 testable requirements. This is an acceptance plan, not a claim of passing Rust tests. C/L/H cover codec, lifecycle, and host behavior; T and D cover trust, anchoring, and availability.

### 20.1. Codec and graph

| ID | Property |
|---|---|
| C01 | Golden bytes/CIDs match across all supported targets |
| C02 | Map insertion order does not affect serialization |
| C03 | Duplicate keys, noncanonical encodings, and trailing bytes are rejected |
| C04 | Missing/wrong blocks or schemas cannot produce partial restoration |
| C05 | Depth/count/byte limits apply before unsafe allocation |
| C06 | Root.memory/phase/Episode agree, including genesis |
| C07 | A closed Episode has correct pairing and final no_op |
| C08 | Closure is bounded without a hidden previous-root chain |
| C09 | Memory is reused; local patches are not journaled |
| C10 | CAR and two Blocks adapters preserve exact roots |
| C11 | Unsupported CID/base/hash/lengths are rejected |
| C12 | Tag 42 and embedded CIDs are fully validated |
| C13 | A raw block cannot replace a structural kind |
| C14 | All floats are rejected, including finite float64 |
| C15 | UTF-8, b/aa ordering, and integer/tag/length boundaries are covered |
| C16 | A CAR header cannot substitute the expected root or closure |
| C17 | Frame order does not affect the root; legacy formats are rejected |
| C18 | Another codec implementation yields identical bytes; not just self-roundtrip |

### 20.2. Lifecycle

| ID | Property |
|---|---|
| L01 | The model/local-tool loop writes no checkpoints before a boundary |
| L02 | An external tool emits a candidate and seals Activation |
| L03 | Core never calls the executor |
| L04 | A receipt preserves EpisodeId; Actor after no_op changes it |
| L05 | Mismatched phases/inputs/receipts are rejected |
| L06 | Stale/duplicate replies are not accepted twice |
| L07 | No-op without commands closes a zero-action Episode |
| L08 | Text stop, timeout, and budget failure never become no_op |
| L09 | Invalid/overlapping patches leave RAM memory unchanged |
| L10 | Multiple calls are rejected before any effect |
| L11 | Read-only data and authority are inaccessible to memory tools |
| L12 | A memory-disabled profile creates no fictitious memory |
| L13 | Episode counters survive resume |
| L14 | A new activation requires no transcript/provider reasoning |
| L15 | A retry after terminal failure creates a new ActionId |

### 20.3. Host and crash handling

| ID | Property |
|---|---|
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
| H11 | History is grouped by no_op |
| H12 | Lost inference attempts remain in host accounting |
| H13 | Secrets are absent from state/export/ordinary logs |
| H14 | Restoration does not roll back external world or authority |
| H15 | A manifest crash leaves a complete old or new decision |

### 20.4. Trusted execution and on-chain anchoring

| ID | Property |
|---|---|
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
|---|---|
| D01 | Readiness covers the full closure, not only the root block |
| D02 | Primary failure after acceptance permits restoration from secondary |
| D03 | False/corrupt replica data is rejected without changing the anchor |
| D04 | Loss of all copies means unavailable, not new empty memory |
| D05 | Retention covers waiting; renewal does not depend on agent wake |
| D06 | GC never deletes the current root closure or pending-action dependencies |
| D07 | Storage outage does not erase the status of an executed command |
| D08 | Storage costs/status enter observation and host accounting |

## 21. Implementation validation and compatibility boundaries

Native unit/property tests, fuzzing of decoder/patch/tool replies/graphs, and a no_std build without hidden std features are mandatory. At least one constrained sandbox target compares canonical outputs with native execution. A Wasm smoke test does not prove JAM/PVM readiness.

Pin the DASL conformance suite by revision. It includes multiple, sometimes conflicting CBOR profiles; use relevant cases and stricter Raiv negative tests rather than requiring every unrelated row to pass. [S13] Self encode/decode roundtrips are insufficient; independent canonical bytes and CID vectors are required.

Golden fixtures MUST cover both genesis profiles, memory patch, action Boundary, terminal Receipt, closed Episode, full root, command/episode ID domains, compact-anchor reconstruction, and CAR. Fixture strings are data and must not be translated at test time.

CI requires no secrets or real financial systems. Live inference/IPFS tests are separate, with distinct skipped/failed/passed states. Results are reported only after actual execution. Checking this specification's text does not execute the C/L/H/T/D tests.

Measure RAM high-water marks, dependency graph/binary size, and restore/encode time against pinned profiles and targets. Minimalism comes from removing unnecessary subsystems, not an unverified kilobyte claim.

## 22. Implementation order and completion criteria

| Milestone | Deliverable | Gate |
|---|---|---|
| 1. Format contract | Six typed blocks, checked DaslCid, ID derivation, strict codec, genesis, exact fixtures | C01–C09, C11–C15, C18; no_std spike |
| 2. RAM core | Projection, patch, single-tool loop, yield/no_op, limits, scripted inference | L01–L15; no I/O inside the loop |
| 3. Local trusted host | Registry/anchor, admission, atomic acceptance, simulator ledger, Actor | H01–H15, T01–T07; crash scenario |
| 4. Availability and transport | CAR, second store, readiness/retention, primary failure | C10, C16–C17, D01–D08 |
| 5. Minimal live agent | DeepSeek authentication/normalization, profiles, CLI; IPFS module | Separate live smoke; secret-leak checks |
| 6. VM bridge | RAM-lifetime and trusted-run-binding adapter, same data/lifecycle contract | T08, native/sandbox equality |

A complete `raiv` 0.0.0 passes model-independent format/lifecycle tests, performs no I/O, retains no transcript, and has bounded restoration and phase/authority guards. A complete `raiv-demo` demonstrates action→failure→retry→success→set_wake→receipt→no_op with RAM destruction and restoration, then another ActorWake and the provider-loss scenario.

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
- **[S6] serde_ipld_dagcbor 0.7.0:** https://docs.rs/serde_ipld_dagcbor/0.7.0/serde_ipld_dagcbor/
- **[S7] Rust Reference, no_std:** https://doc.rust-lang.org/reference/names/preludes.html#the-no_std-attribute
- **[S8] Parity, JAM architecture:** https://www.parity.io/blog/JAM-demystified-explainer ; https://jamcha.in/tech
- **[S9] DeepSeek tool calls:** https://api-docs.deepseek.com/guides/tool_calls/
- **[S10] DeepSeek thinking mode:** https://api-docs.deepseek.com/guides/thinking_mode/
- **[S11] Kubo RPC:** https://docs.ipfs.tech/reference/kubo/rpc/
- **[S12] DASL IPFS interoperability:** https://dasl.ing/ipfs.html
- **[S13] DASL test matrix:** https://hyphacoop.github.io/dasl-testing/

**Delivery status:** design specification. This delivery does not implement Rust code or execute a no_std build, integration smoke tests, or the 64 acceptance checks; those determine implementation readiness.
