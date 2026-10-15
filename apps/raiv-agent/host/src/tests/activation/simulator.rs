use super::*;
use crate::{
    acceptance::{accept_local, accept_ram},
    admission::{AdmissionBook, AdmissionStatus},
    checkpoint::ManifestLimits,
    disk::{tests::Directory, DiskBlockStore},
    launch::{launch_local, record_reserved},
    policy::initialize_local,
    simulator::{ExecutorBook, SimulatorError, Status},
};

use raiv::{boundary::Exit, format::DecodeLimits, limits::Limits, receipt::Outcome};

fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn only_committed_pending_command_enters_simulator_and_ram_only_anchor_cannot_substitute_disk_commit(
) {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut admissions = AdmissionBook::new(4).unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    initialize_local(
        registered,
        &admissions,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        executor.authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::WrongPhase)
    ));
    let value = admission::input(0x60, 0x70);
    let grant = value.budget_admission();
    let token = admissions
        .admit_actor_wake(registered, value, grant)
        .unwrap();
    record_reserved(
        registered,
        &admissions,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let mut run = launch_local(
        &wasm,
        registered,
        &mut admissions,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "trade", map(vec![])))
        .unwrap();
    let mut sealed = run.into_sealed().unwrap();
    assert!(matches!(
        executor.authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::WrongPhase)
    ));
    let ram = disk
        .readback(registered.snapshot(), &Limits::default())
        .unwrap();
    let mut next = ram;
    next.place(sealed.snapshot().unwrap()).unwrap();
    accept_ram(&mut registry, &mut admissions, &token, &mut sealed, &next).unwrap();
    let registered = registry.agent([0xb0; 32]).unwrap();
    assert!(matches!(
        executor.authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::Policy(_))
    ));
    assert!(executor.is_empty());
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn committed_slot_one_intent_terminal_receipt_routes_action_result_and_new_slot_without_replaying_prior_intent(
) {
    let (wasm, mut registry, mut admissions, mut disk, directory) =
        action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let tight = Limits {
        max_command_bytes: 0,
        ..Limits::default()
    };
    assert!(executor
        .authorize_pending(registered, &admissions, &mut disk, &caps(), &tight)
        .is_err());
    assert!(executor.is_empty());
    let root = registered.snapshot().root();
    let path = directory.0.join(root.to_string());
    std::fs::write(&path, [0x80]).unwrap();
    assert!(executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        )
        .is_err());
    assert!(executor.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), [0x80]);
    std::fs::remove_file(path).unwrap();
    let body = registered
        .snapshot()
        .blocks()
        .find(|(cid, _)| *cid == root)
        .unwrap()
        .1;
    disk.put(root, body).unwrap();
    let first = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    assert_eq!(
        executor
            .authorize_pending(
                registered,
                &admissions,
                &mut disk,
                &caps(),
                &Limits::default()
            )
            .unwrap(),
        first
    );
    let episode = registered.snapshot().current_episode().unwrap();
    let last = registered
        .snapshot()
        .boundary(episode.boundaries().last().unwrap())
        .unwrap();
    let Exit::AwaitAction(command) = last.exit() else {
        panic!()
    };
    provision_empty_data(registered, &admissions, &mut disk);
    crate::simulator::local::record_ready(
        first,
        registered,
        &admissions,
        &executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let mut lagging = ExecutorBook::new(4).unwrap();
    let stale = lagging
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    assert_eq!(
        crate::simulator::local::begin_local(
            first,
            registered,
            &admissions,
            &mut executor,
            &mut disk,
            &local_caps()
        )
        .unwrap()
        .as_bytes(),
        command
    );
    assert!(matches!(
        executor.begin(first),
        Err(SimulatorError::AlreadySubmitted)
    ));
    assert_eq!(executor.record(first).unwrap().status, Status::Submitted);
    assert!(executor.receipt(first).unwrap().is_none());
    let failed = action_result::receipt(registered.snapshot(), Outcome::Failed);
    crate::simulator::local::resolve_local_data(
        first,
        registered,
        &admissions,
        &mut executor,
        failed,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let immutable = executor.receipt(first).unwrap().unwrap();
    let value = action_result::input(immutable, 0x61, 0x71);
    let exact = value.as_bytes().to_vec();
    let grant = value.budget_admission();
    let token = admissions
        .admit_action_result(registered, value, immutable, grant)
        .unwrap();
    assert!(matches!(
        executor.authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::Busy)
    ));
    assert_eq!(executor.record(first).unwrap().submission_intents, 1);
    assert!(matches!(
        lagging.begin_current(
            stale,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::Busy)
    ));
    assert_eq!(lagging.record(stale).unwrap().submission_intents, 0);
    record_reserved(
        registered,
        &admissions,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let mut run = launch_local(
        &wasm,
        registered,
        &mut admissions,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "trade", map(vec![])))
        .unwrap();
    let mut sealed = run.into_sealed().unwrap();
    accept_local(
        &mut registry,
        &mut admissions,
        &token,
        &mut sealed,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let registered = registry.agent([0xb0; 32]).unwrap();
    assert!(matches!(
        lagging.begin_current(
            stale,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::Binding)
    ));
    assert_eq!(lagging.record(stale).unwrap().status, Status::Ready);
    assert_eq!(lagging.len(), 1);
    let second = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    assert_ne!(first, second);
    assert_ne!(
        executor.record(first).unwrap().action_id,
        executor.record(second).unwrap().action_id
    );
    assert_eq!(executor.record(first).unwrap().status, Status::Terminal);
    assert_eq!(executor.record(second).unwrap().submission_intents, 0);
    assert_eq!(token.packet().input().as_bytes(), exact);
    assert_eq!(
        admissions.record([0xb0; 32], [0x61; 32]).unwrap().status,
        AdmissionStatus::Consumed
    );
    crate::simulator::local::record_ready(
        second,
        registered,
        &admissions,
        &executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    crate::simulator::local::begin_local(
        second,
        registered,
        &admissions,
        &mut executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    crate::simulator::local::resolve_local_data(
        second,
        registered,
        &admissions,
        &mut executor,
        action_result::receipt(registered.snapshot(), Outcome::Succeeded),
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let historical = raiv::receipt::Receipt::decode(
        executor.receipt(first).unwrap().unwrap().as_bytes(),
        &Limits::default(),
    )
    .unwrap();
    let before = crate::simulator::checkpoint::file::ExecutorCheckpointFile::new(
        &mut disk,
        local_caps().executor,
    )
    .read()
    .unwrap()
    .unwrap();
    crate::simulator::local::resolve_local_data(
        first,
        registered,
        &admissions,
        &mut executor,
        historical,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    assert_eq!(
        crate::simulator::checkpoint::file::ExecutorCheckpointFile::new(
            &mut disk,
            local_caps().executor
        )
        .read()
        .unwrap()
        .unwrap()
        .as_bytes(),
        before.as_bytes()
    );
    assert_eq!(registered.anchor().revision, 2);
    let pin = crate::simulator::recovery::current_pin(
        registered,
        &admissions,
        &executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let mut tiny = ExecutorBook::new(1).unwrap();
    assert!(matches!(
        crate::simulator::recovery::adopt_book(
            registered,
            &admissions,
            &mut tiny,
            &mut disk,
            &pin,
            &local_caps()
        ),
        Err(crate::simulator::recovery::ExecutorRecoveryError::Simulator(SimulatorError::Capacity))
    ));
    assert!(tiny.is_empty());
    let mut rebuilt = ExecutorBook::new(3).unwrap();
    crate::simulator::recovery::adopt_book(
        registered,
        &admissions,
        &mut rebuilt,
        &mut disk,
        &pin,
        &local_caps(),
    )
    .unwrap();
    assert_eq!(rebuilt.len(), 2);
    assert!(matches!(
        rebuilt.record(second),
        Err(SimulatorError::ForeignTicket)
    ));
    let correlation = rebuilt
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    assert_eq!(
        rebuilt.record(correlation).unwrap(),
        executor.record(second).unwrap()
    );
    assert!(matches!(
        rebuilt.begin(correlation),
        Err(SimulatorError::AlreadySubmitted)
    ));
    assert_eq!(executor.len(), 2);
    assert_eq!(executor.record(first).unwrap().submission_intents, 1);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn captured_ticket_rechecks_storage_and_policy_before_intent_and_refusal_keeps_ready_without_new_slots(
) {
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut executor = ExecutorBook::new(2).unwrap();
    let ticket = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    let ready = executor.record(ticket).unwrap();
    let mut foreign = ExecutorBook::new(2).unwrap();
    assert!(matches!(
        foreign.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::ForeignTicket)
    ));
    let changed = Limits {
        max_receipt_bytes: 1,
        ..Limits::default()
    };
    assert!(matches!(
        executor.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &changed
        ),
        Err(SimulatorError::Binding)
    ));
    assert_eq!(executor.record(ticket).unwrap(), ready);
    let root = registered.snapshot().root();
    let path = directory.0.join(root.to_string());
    std::fs::write(&path, [0x80]).unwrap();
    assert!(matches!(
        executor.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::Policy(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), [0x80]);
    assert_eq!(executor.record(ticket).unwrap(), ready);
    std::fs::remove_file(path).unwrap();
    let body = registered
        .snapshot()
        .blocks()
        .find(|(cid, _)| *cid == root)
        .unwrap()
        .1;
    disk.put(root, body).unwrap();
    let frame = disk.manifest(caps().wire).read().unwrap().unwrap();
    disk.manifest(caps().wire)
        .replace(Some(&frame), &[0xa0])
        .unwrap();
    assert!(matches!(
        executor.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::Policy(_))
    ));
    assert_eq!(executor.record(ticket).unwrap(), ready);
    disk.manifest(caps().wire)
        .replace(Some(&[0xa0]), &frame)
        .unwrap();
    let command = executor
        .begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    assert_eq!(command.action_id(), ready.action_id);
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 1);
    assert!(matches!(
        executor.begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default()
        ),
        Err(SimulatorError::AlreadySubmitted)
    ));
    assert_eq!(executor.len(), 1);
    assert!(executor.receipt(ticket).unwrap().is_none());
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn executor_checkpoint_capture_is_bound_to_native_consumed_decision_and_preserves_all_states_as_data_only(
) {
    use crate::simulator::checkpoint::{
        ExecutorCheckpoint, ExecutorCheckpointError, ExecutorLimits,
    };
    let (_wasm, registry, admissions, mut disk, _directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let ticket = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    let limits = ExecutorLimits {
        wire: DecodeLimits::default(),
        max_actions: 10,
    };
    let empty = AdmissionBook::new(3).unwrap();
    assert!(matches!(
        ExecutorCheckpoint::capture(registered, &empty, &executor, &limits),
        Err(ExecutorCheckpointError::Binding)
    ));
    assert!(ExecutorCheckpoint::capture(
        registered,
        &admissions,
        &executor,
        &ExecutorLimits {
            max_actions: 0,
            ..limits
        }
    )
    .is_err());
    let ready = executor.record(ticket).unwrap();
    for phase in 0..3 {
        if phase == 1 {
            executor
                .begin_current(
                    ticket,
                    registered,
                    &admissions,
                    &mut disk,
                    &caps(),
                    &Limits::default(),
                )
                .unwrap();
        }
        if phase == 2 {
            executor
                .resolve(
                    ticket,
                    action_result::receipt(registered.snapshot(), Outcome::Failed),
                )
                .unwrap();
        }
        let data =
            ExecutorCheckpoint::capture(registered, &admissions, &executor, &limits).unwrap();
        let decoded = ExecutorCheckpoint::decode(data.as_bytes(), &limits).unwrap();
        assert_eq!(decoded.as_bytes(), data.as_bytes());
        assert_eq!(
            decoded.records().next().unwrap(),
            executor.record(ticket).unwrap()
        );
        assert_eq!(decoded.agent_id(), registered.anchor().agent_id);
        assert_eq!(decoded.program_binding(), registered.program_binding());
        assert_eq!(
            decoded.receipt_bytes(ready.action_id),
            executor.receipt(ticket).unwrap().map(|r| r.as_bytes())
        );
        assert!(!data
            .as_bytes()
            .windows(b"GUEST_RAM_ONLY".len())
            .any(|s| s == b"GUEST_RAM_ONLY"));
    }
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 1);
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor().revision, 1);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_committed_executor_data_file_reopens_without_head_changes_ticket_adoption_or_effect_replay()
{
    use crate::disk::manifest::Publication;
    use crate::simulator::checkpoint::{
        file::ExecutorCheckpointFile, ExecutorCheckpoint, ExecutorLimits,
    };
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let main = disk.manifest(caps().wire).read().unwrap().unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let ticket = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    let limits = ExecutorLimits {
        wire: DecodeLimits::default(),
        max_actions: 10,
    };
    let ready = ExecutorCheckpoint::capture(registered, &admissions, &executor, &limits).unwrap();
    let baseline = ExecutorCheckpointFile::new(&mut disk, limits)
        .read()
        .unwrap()
        .unwrap();
    ExecutorCheckpointFile::new(&mut disk, limits)
        .replace(Some(&baseline), &ready)
        .unwrap();
    executor
        .begin_current(
            ticket,
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    let submitted =
        ExecutorCheckpoint::capture(registered, &admissions, &executor, &limits).unwrap();
    ExecutorCheckpointFile::new(&mut disk, limits)
        .replace(Some(&ready), &submitted)
        .unwrap();
    executor
        .resolve(
            ticket,
            action_result::receipt(registered.snapshot(), Outcome::Succeeded),
        )
        .unwrap();
    let terminal =
        ExecutorCheckpoint::capture(registered, &admissions, &executor, &limits).unwrap();
    ExecutorCheckpointFile::new(&mut disk, limits)
        .replace(Some(&submitted), &terminal)
        .unwrap();
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), main);
    assert_eq!(registered.anchor().revision, 1);
    drop(disk);
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let mut file = ExecutorCheckpointFile::new(&mut disk, limits);
    let data = file.read().unwrap().unwrap();
    assert_eq!(data.as_bytes(), terminal.as_bytes());
    assert_eq!(
        data.records().next().unwrap(),
        executor.record(ticket).unwrap()
    );
    assert_eq!(
        data.receipt_bytes(executor.record(ticket).unwrap().action_id),
        executor.receipt(ticket).unwrap().map(|r| r.as_bytes())
    );
    assert_eq!(
        file.replace(Some(&submitted), &terminal).unwrap(),
        Publication::AlreadyPresent
    );
    assert!(ExecutorBook::new(4).unwrap().is_empty());
    assert!(matches!(
        executor.begin(ticket),
        Err(SimulatorError::AlreadySubmitted)
    ));
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 1);
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), main);
}

fn local_caps() -> crate::simulator::local::LocalExecutorLimits {
    crate::simulator::local::LocalExecutorLimits {
        executor: crate::simulator::checkpoint::ExecutorLimits {
            wire: DecodeLimits::default(),
            max_actions: 10,
        },
        manifest: caps(),
        caller: Limits::default(),
    }
}

fn provision_empty_data(
    registered: &crate::registry::RegisteredAgent,
    admissions: &AdmissionBook,
    disk: &mut DiskBlockStore,
) {
    // Only acknowledge the exact guarded genesis baseline already present in the
    // fixture; this cannot invent empty state after a post-genesis ledger loss.
    let empty = crate::simulator::checkpoint::ExecutorCheckpoint::capture(
        registered,
        admissions,
        &ExecutorBook::new(4).unwrap(),
        &local_caps().executor,
    )
    .unwrap();
    let mut file = crate::simulator::checkpoint::file::ExecutorCheckpointFile::new(
        disk,
        local_caps().executor,
    );
    let present = file.read().unwrap().expect("guarded genesis baseline");
    assert_eq!(present.as_bytes(), empty.as_bytes());
    file.replace(Some(&present), &empty).unwrap();
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn local_intent_requires_recorded_ready_fsyncs_submitted_before_command_and_blocks_repeat_without_reading_file(
) {
    use crate::disk::manifest::Publication;
    use crate::simulator::{
        checkpoint::file::ExecutorCheckpointFile,
        local::{begin_local, record_ready, LocalExecutorError},
    };
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let main = disk.manifest(caps().wire).read().unwrap().unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let ticket = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    let ready = executor.record(ticket).unwrap();
    let baseline = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .read()
        .unwrap()
        .unwrap();
    std::fs::remove_file(directory.0.join("raiv-executor.cbor")).unwrap();
    assert!(matches!(
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut executor,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::MissingCheckpoint)
    ));
    assert!(matches!(
        record_ready(
            ticket,
            registered,
            &admissions,
            &executor,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::MissingCheckpoint)
    ));
    assert_eq!(executor.record(ticket).unwrap(), ready);
    // Explicit owned-test restoration of the previously known baseline, not fallback.
    ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .replace(None, &baseline)
        .unwrap();
    provision_empty_data(registered, &admissions, &mut disk);
    assert!(matches!(
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut executor,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Conflict)
    ));
    assert_eq!(
        record_ready(
            ticket,
            registered,
            &admissions,
            &executor,
            &mut disk,
            &local_caps()
        )
        .unwrap(),
        Publication::Replaced
    );
    assert_eq!(
        record_ready(
            ticket,
            registered,
            &admissions,
            &executor,
            &mut disk,
            &local_caps()
        )
        .unwrap(),
        Publication::AlreadyPresent
    );
    let command = begin_local(
        ticket,
        registered,
        &admissions,
        &mut executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap()
    .as_bytes()
    .to_vec();
    let data = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(
        data.records().next().unwrap(),
        executor.record(ticket).unwrap()
    );
    assert_eq!(data.command_bytes(ready.action_id).unwrap(), command);
    assert_eq!(executor.record(ticket).unwrap().status, Status::Submitted);
    assert!(executor.receipt(ticket).unwrap().is_none());
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), main);
    std::fs::remove_file(directory.0.join("raiv-executor.cbor")).unwrap();
    assert!(matches!(
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut executor,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Simulator(
            SimulatorError::AlreadySubmitted
        ))
    ));
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 1);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn unknown_ready_can_ack_without_intent_but_unknown_submitted_exposes_no_command_and_identical_file_blocks_fresh_book_replay(
) {
    use crate::disk::manifest::{ManifestError, Publication};
    use crate::simulator::{
        checkpoint::file::{ExecutorCheckpointFile, ExecutorFileError},
        local::{begin_local, record_ready, LocalExecutorError},
    };
    let (_wasm, registry, admissions, mut disk, _directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let ticket = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    provision_empty_data(registered, &admissions, &mut disk);
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        record_ready(
            ticket,
            registered,
            &admissions,
            &executor,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::File(ExecutorFileError::Manifest(
            ManifestError::UnknownOutcome(_)
        )))
    ));
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 0);
    assert_eq!(
        record_ready(
            ticket,
            registered,
            &admissions,
            &executor,
            &mut disk,
            &local_caps()
        )
        .unwrap(),
        Publication::AlreadyPresent
    );
    assert!(matches!(
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut executor,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::File(ExecutorFileError::Manifest(
            ManifestError::UnknownOutcome(_)
        )))
    ));
    assert_eq!(executor.record(ticket).unwrap().status, Status::Submitted);
    assert_eq!(
        ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
            .read()
            .unwrap()
            .unwrap()
            .records()
            .next()
            .unwrap()
            .submission_intents,
        1
    );
    let mut fresh = ExecutorBook::new(4).unwrap();
    let other = fresh
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    assert!(matches!(
        record_ready(
            other,
            registered,
            &admissions,
            &fresh,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Conflict)
    ));
    assert!(matches!(
        begin_local(
            other,
            registered,
            &admissions,
            &mut fresh,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Conflict)
    ));
    assert_eq!(fresh.record(other).unwrap().submission_intents, 0);
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 1);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn local_intent_foreign_policy_capacity_and_disk_refusals_publish_no_intent_or_frame_then_repair_allows_one_submission(
) {
    use crate::simulator::{
        checkpoint::file::ExecutorCheckpointFile,
        local::{begin_local, record_ready, LocalExecutorError},
    };
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut executor = ExecutorBook::new(4).unwrap();
    let ticket = executor
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    provision_empty_data(registered, &admissions, &mut disk);
    let baseline = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .read()
        .unwrap()
        .unwrap();
    let ready = executor.record(ticket).unwrap();
    let mut other = ExecutorBook::new(4).unwrap();
    assert!(matches!(
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut other,
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Simulator(SimulatorError::ForeignTicket))
    ));
    let mut tight = local_caps();
    tight.executor.max_actions = 0;
    assert!(record_ready(
        ticket,
        registered,
        &admissions,
        &executor,
        &mut disk,
        &tight
    )
    .is_err());
    tight = local_caps();
    tight.caller.max_receipt_bytes = 1;
    assert!(matches!(
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut executor,
            &mut disk,
            &tight
        ),
        Err(LocalExecutorError::Simulator(SimulatorError::Binding))
    ));
    let root = registered.anchor().root;
    let path = directory.0.join(root.to_string());
    std::fs::write(&path, [0x80]).unwrap();
    assert!(record_ready(
        ticket,
        registered,
        &admissions,
        &executor,
        &mut disk,
        &local_caps()
    )
    .is_err());
    assert_eq!(
        ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
            .read()
            .unwrap()
            .unwrap()
            .as_bytes(),
        baseline.as_bytes()
    );
    assert_eq!(executor.record(ticket).unwrap(), ready);
    std::fs::remove_file(path).unwrap();
    disk.put(
        root,
        registered
            .snapshot()
            .blocks()
            .find(|(cid, _)| *cid == root)
            .unwrap()
            .1,
    )
    .unwrap();
    record_ready(
        ticket,
        registered,
        &admissions,
        &executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let frame = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .read()
        .unwrap()
        .unwrap();
    std::fs::write(directory.0.join("raiv-executor.cbor"), [0xa0]).unwrap();
    assert!(begin_local(
        ticket,
        registered,
        &admissions,
        &mut executor,
        &mut disk,
        &local_caps()
    )
    .is_err());
    assert_eq!(
        std::fs::read(directory.0.join("raiv-executor.cbor")).unwrap(),
        [0xa0]
    );
    assert_eq!(executor.record(ticket).unwrap(), ready);
    std::fs::write(directory.0.join("raiv-executor.cbor"), frame.as_bytes()).unwrap();
    begin_local(
        ticket,
        registered,
        &admissions,
        &mut executor,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    assert_eq!(executor.record(ticket).unwrap().submission_intents, 1);
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn local_terminal_data_all_outcomes_are_fsynced_before_ram_known_duplicate_and_conflict_are_immutable_no_world_claim(
) {
    use crate::simulator::{
        checkpoint::file::ExecutorCheckpointFile,
        local::{begin_local, record_ready, resolve_local_data, LocalExecutorError},
    };
    for outcome in [Outcome::Failed, Outcome::Succeeded, Outcome::NotExecuted] {
        let (_wasm, registry, admissions, mut disk, _directory) = action_result::durable::pending();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let main = disk.manifest(caps().wire).read().unwrap().unwrap();
        let mut book = ExecutorBook::new(4).unwrap();
        let ticket = book
            .authorize_pending(
                registered,
                &admissions,
                &mut disk,
                &caps(),
                &Limits::default(),
            )
            .unwrap();
        provision_empty_data(registered, &admissions, &mut disk);
        assert!(matches!(
            resolve_local_data(
                ticket,
                registered,
                &admissions,
                &mut book,
                action_result::receipt(registered.snapshot(), outcome),
                &mut disk,
                &local_caps()
            ),
            Err(LocalExecutorError::Simulator(SimulatorError::NotSubmitted))
        ));
        record_ready(
            ticket,
            registered,
            &admissions,
            &book,
            &mut disk,
            &local_caps(),
        )
        .unwrap();
        begin_local(
            ticket,
            registered,
            &admissions,
            &mut book,
            &mut disk,
            &local_caps(),
        )
        .unwrap();
        let mut tight = local_caps();
        tight.executor.max_actions = 0;
        assert!(resolve_local_data(
            ticket,
            registered,
            &admissions,
            &mut book,
            action_result::receipt(registered.snapshot(), outcome),
            &mut disk,
            &tight
        )
        .is_err());
        assert!(book.receipt(ticket).unwrap().is_none());
        tight = local_caps();
        tight.caller.max_receipt_bytes = 1;
        assert!(matches!(
            resolve_local_data(
                ticket,
                registered,
                &admissions,
                &mut book,
                action_result::receipt(registered.snapshot(), outcome),
                &mut disk,
                &tight
            ),
            Err(LocalExecutorError::Simulator(SimulatorError::Binding))
        ));
        let terminal = resolve_local_data(
            ticket,
            registered,
            &admissions,
            &mut book,
            action_result::receipt(registered.snapshot(), outcome),
            &mut disk,
            &local_caps(),
        )
        .unwrap();
        assert_eq!(terminal.status, Status::Terminal);
        let data = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
            .read()
            .unwrap()
            .unwrap();
        assert_eq!(data.records().next().unwrap(), terminal);
        assert_eq!(
            data.receipt_bytes(terminal.action_id),
            book.receipt(ticket).unwrap().map(|r| r.as_bytes())
        );
        assert_eq!(
            resolve_local_data(
                ticket,
                registered,
                &admissions,
                &mut book,
                action_result::receipt(registered.snapshot(), outcome),
                &mut disk,
                &local_caps()
            )
            .unwrap(),
            terminal
        );
        let conflict = if outcome == Outcome::Succeeded {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        };
        assert!(matches!(
            resolve_local_data(
                ticket,
                registered,
                &admissions,
                &mut book,
                action_result::receipt(registered.snapshot(), conflict),
                &mut disk,
                &local_caps()
            ),
            Err(LocalExecutorError::Simulator(
                SimulatorError::TerminalConflict
            ))
        ));
        assert_eq!(
            ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
                .read()
                .unwrap()
                .unwrap()
                .as_bytes(),
            data.as_bytes()
        );
        assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), main);
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn unknown_terminal_preserves_submitted_until_identical_ack_conflicting_receipt_foreign_ticket_and_corrupt_known_file_refuse(
) {
    use crate::disk::manifest::ManifestError;
    use crate::simulator::{
        checkpoint::file::{ExecutorCheckpointFile, ExecutorFileError},
        local::{begin_local, record_ready, resolve_local_data, LocalExecutorError},
    };
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = ExecutorBook::new(4).unwrap();
    let ticket = book
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    provision_empty_data(registered, &admissions, &mut disk);
    record_ready(
        ticket,
        registered,
        &admissions,
        &book,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    begin_local(
        ticket,
        registered,
        &admissions,
        &mut book,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let submitted = book.record(ticket).unwrap();
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        resolve_local_data(
            ticket,
            registered,
            &admissions,
            &mut book,
            action_result::receipt(registered.snapshot(), Outcome::Failed),
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::File(ExecutorFileError::Manifest(
            ManifestError::UnknownOutcome(_)
        )))
    ));
    assert_eq!(book.record(ticket).unwrap(), submitted);
    assert!(book.receipt(ticket).unwrap().is_none());
    let data = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(data.records().next().unwrap().status, Status::Terminal);
    assert!(matches!(
        resolve_local_data(
            ticket,
            registered,
            &admissions,
            &mut book,
            action_result::receipt(registered.snapshot(), Outcome::Succeeded),
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Conflict)
    ));
    assert_eq!(book.record(ticket).unwrap(), submitted);
    assert_eq!(
        ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
            .read()
            .unwrap()
            .unwrap()
            .as_bytes(),
        data.as_bytes()
    );
    let terminal = resolve_local_data(
        ticket,
        registered,
        &admissions,
        &mut book,
        action_result::receipt(registered.snapshot(), Outcome::Failed),
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    assert_eq!(terminal.status, Status::Terminal);
    assert_eq!(terminal.submission_intents, 1);
    let mut foreign = ExecutorBook::new(4).unwrap();
    assert!(matches!(
        resolve_local_data(
            ticket,
            registered,
            &admissions,
            &mut foreign,
            action_result::receipt(registered.snapshot(), Outcome::Failed),
            &mut disk,
            &local_caps()
        ),
        Err(LocalExecutorError::Simulator(SimulatorError::ForeignTicket))
    ));
    std::fs::write(directory.0.join("raiv-executor.cbor"), [0xa0]).unwrap();
    assert!(resolve_local_data(
        ticket,
        registered,
        &admissions,
        &mut book,
        action_result::receipt(registered.snapshot(), Outcome::Failed),
        &mut disk,
        &local_caps()
    )
    .is_err());
    assert_eq!(book.record(ticket).unwrap(), terminal);
    assert_eq!(
        std::fs::read(directory.0.join("raiv-executor.cbor")).unwrap(),
        [0xa0]
    );
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn paired_pin_reopen_adopts_ready_submitted_terminal_into_new_origins_without_tokens_reset_or_submission_replay(
) {
    use crate::simulator::{
        local::{begin_local, record_ready, resolve_local_data, LocalExecutorError},
        recovery::{adopt_book, current_pin},
    };
    for state in 0..3 {
        let (wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let mut original = ExecutorBook::new(4).unwrap();
        let ticket = original
            .authorize_pending(
                registered,
                &admissions,
                &mut disk,
                &caps(),
                &Limits::default(),
            )
            .unwrap();
        provision_empty_data(registered, &admissions, &mut disk);
        record_ready(
            ticket,
            registered,
            &admissions,
            &original,
            &mut disk,
            &local_caps(),
        )
        .unwrap();
        if state >= 1 {
            begin_local(
                ticket,
                registered,
                &admissions,
                &mut original,
                &mut disk,
                &local_caps(),
            )
            .unwrap();
        }
        if state == 2 {
            resolve_local_data(
                ticket,
                registered,
                &admissions,
                &mut original,
                action_result::receipt(registered.snapshot(), Outcome::Failed),
                &mut disk,
                &local_caps(),
            )
            .unwrap();
        }
        let before = original.record(ticket).unwrap();
        let pin =
            current_pin(registered, &admissions, &original, &mut disk, &local_caps()).unwrap();
        drop(original);
        drop(disk);
        drop(registry);
        drop(admissions);
        let mut disk =
            DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
        let mut registry = crate::registry::Registry::new(2).unwrap();
        let mut admissions = AdmissionBook::new(10).unwrap();
        crate::recovery::adopt_local(
            &wasm,
            &mut disk,
            &mut registry,
            &mut admissions,
            pin.source,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let mut recovered = ExecutorBook::new(4).unwrap();
        adopt_book(
            registered,
            &admissions,
            &mut recovered,
            &mut disk,
            &pin,
            &local_caps(),
        )
        .unwrap();
        assert_eq!(recovered.len(), 1);
        assert!(matches!(
            recovered.record(ticket),
            Err(SimulatorError::ForeignTicket)
        ));
        let other = recovered
            .authorize_pending(
                registered,
                &admissions,
                &mut disk,
                &caps(),
                &Limits::default(),
            )
            .unwrap();
        assert_ne!(ticket, other);
        assert_eq!(recovered.record(other).unwrap(), before);
        if state == 0 {
            begin_local(
                other,
                registered,
                &admissions,
                &mut recovered,
                &mut disk,
                &local_caps(),
            )
            .unwrap();
            assert_eq!(recovered.record(other).unwrap().submission_intents, 1);
        } else {
            assert!(matches!(
                begin_local(
                    other,
                    registered,
                    &admissions,
                    &mut recovered,
                    &mut disk,
                    &local_caps()
                ),
                Err(LocalExecutorError::Simulator(
                    SimulatorError::AlreadySubmitted
                ))
            ));
            assert_eq!(recovered.record(other).unwrap(), before);
        }
        if state == 1 {
            assert!(recovered.receipt(other).unwrap().is_none());
            resolve_local_data(
                other,
                registered,
                &admissions,
                &mut recovered,
                action_result::receipt(registered.snapshot(), Outcome::NotExecuted),
                &mut disk,
                &local_caps(),
            )
            .unwrap();
        }
        if state == 2 {
            assert_eq!(
                recovered.receipt(other).unwrap().unwrap().outcome(),
                Outcome::Failed
            );
            resolve_local_data(
                other,
                registered,
                &admissions,
                &mut recovered,
                action_result::receipt(registered.snapshot(), Outcome::Failed),
                &mut disk,
                &local_caps(),
            )
            .unwrap();
        }
        assert_eq!(registered.anchor(), pin.source.anchor);
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn executor_adoption_digest_source_policy_capacity_and_missing_corrupt_dependencies_refuse_atomically_without_repair(
) {
    use crate::simulator::{
        local::record_ready,
        recovery::{adopt_book, current_pin, ExecutorRecoveryError},
    };
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut original = ExecutorBook::new(4).unwrap();
    let ticket = original
        .authorize_pending(
            registered,
            &admissions,
            &mut disk,
            &caps(),
            &Limits::default(),
        )
        .unwrap();
    provision_empty_data(registered, &admissions, &mut disk);
    record_ready(
        ticket,
        registered,
        &admissions,
        &original,
        &mut disk,
        &local_caps(),
    )
    .unwrap();
    let pin = current_pin(registered, &admissions, &original, &mut disk, &local_caps()).unwrap();
    let mut next = ExecutorBook::new(4).unwrap();
    assert!(matches!(
        current_pin(registered, &admissions, &next, &mut disk, &local_caps()),
        Err(ExecutorRecoveryError::PinMismatch)
    ));
    let mut tight = local_caps();
    tight.caller.max_command_bytes = 1;
    assert!(adopt_book(registered, &admissions, &mut next, &mut disk, &pin, &tight).is_err());
    assert!(next.is_empty());
    tight = local_caps();
    tight.executor.max_actions = 0;
    assert!(adopt_book(registered, &admissions, &mut next, &mut disk, &pin, &tight).is_err());
    assert!(next.is_empty());
    let root = registered.anchor().root;
    let path = directory.0.join(root.to_string());
    let body = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(adopt_book(
        registered,
        &admissions,
        &mut next,
        &mut disk,
        &pin,
        &local_caps()
    )
    .is_err());
    assert!(next.is_empty());
    assert!(!path.exists());
    std::fs::write(&path, [0x80]).unwrap();
    assert!(adopt_book(
        registered,
        &admissions,
        &mut next,
        &mut disk,
        &pin,
        &local_caps()
    )
    .is_err());
    assert!(next.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), [0x80]);
    std::fs::remove_file(&path).unwrap();
    disk.put(root, &body).unwrap();
    let ledger = directory.0.join("raiv-executor.cbor");
    let frame = std::fs::read(&ledger).unwrap();
    std::fs::remove_file(&ledger).unwrap();
    assert!(matches!(
        adopt_book(
            registered,
            &admissions,
            &mut next,
            &mut disk,
            &pin,
            &local_caps()
        ),
        Err(ExecutorRecoveryError::MissingCheckpoint)
    ));
    assert!(!ledger.exists());
    assert!(next.is_empty());
    std::fs::write(&ledger, frame).unwrap();
    adopt_book(
        registered,
        &admissions,
        &mut next,
        &mut disk,
        &pin,
        &local_caps(),
    )
    .unwrap();
    assert_eq!(next.len(), 1);
    assert!(matches!(
        adopt_book(
            registered,
            &admissions,
            &mut next,
            &mut disk,
            &pin,
            &local_caps()
        ),
        Err(ExecutorRecoveryError::NotEmpty)
    ));
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn guarded_bootstrap_in_actual_fixture_precedes_actor_admission_and_late_missing_executor_never_recreates_empty_ledger(
) {
    use crate::simulator::{
        checkpoint::file::ExecutorCheckpointFile,
        local::{initialize_genesis, LocalExecutorError},
    };
    let (_wasm, registry, admissions, mut disk, directory) = action_result::durable::pending();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let book = ExecutorBook::new(4).unwrap();
    let source = crate::policy::current_pin(
        registered,
        &admissions,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let baseline = ExecutorCheckpointFile::new(&mut disk, local_caps().executor)
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(baseline.records().count(), 0);
    assert_eq!(registered.anchor().revision, 1);
    assert!(matches!(
        initialize_genesis(
            registered,
            &admissions,
            &book,
            &mut disk,
            &source,
            &local_caps()
        ),
        Err(LocalExecutorError::NotGenesis)
    ));
    std::fs::remove_file(directory.0.join("raiv-executor.cbor")).unwrap();
    assert!(matches!(
        initialize_genesis(
            registered,
            &admissions,
            &book,
            &mut disk,
            &source,
            &local_caps()
        ),
        Err(LocalExecutorError::NotGenesis)
    ));
    assert!(!directory.0.join("raiv-executor.cbor").exists());
    assert!(book.is_empty());
    assert_eq!(registered.anchor(), source.anchor);
}
