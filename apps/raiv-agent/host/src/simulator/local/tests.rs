use super::*;
use crate::simulator::tests::{entry, receipt};
use raiv::format::DecodeLimits;
use raiv::receipt::Outcome;

fn caps() -> ExecutorLimits {
    ExecutorLimits {
        wire: DecodeLimits::default(),
        max_actions: 10,
    }
}

fn data(rows: &[&Entry]) -> ExecutorCheckpoint {
    let e = entry(1, 1);
    ExecutorCheckpoint::test_frame(e.anchor.agent_id, e.program, rows, &caps(), None)
}

#[test]
fn ready_extension_allows_only_one_new_ready_slot_and_preserves_every_prior_terminal_byte_and_policy(
) {
    let mut book = ExecutorBook::new(3).unwrap();
    let e = entry(1, 1);
    let index = book.stage(&e).unwrap();
    let ticket = book.publish(index, e);
    book.begin(ticket).unwrap();
    let r = receipt(&book, ticket, Outcome::Failed);
    book.resolve(ticket, r).unwrap();
    let a = &book.entries[0];
    let mut b = entry(1, 2);
    b.anchor.revision = 2;
    let old = data(&[a]);
    let next = data(&[a, &b]);
    assert!(ready_extension(&old, &next, b.command.action_id()));
    assert!(!ready_extension(&next, &next, b.command.action_id()));
    assert!(!ready_extension(&data(&[]), &next, b.command.action_id()));
    let mut changed = entry(1, 1);
    changed.intents = 1;
    changed.receipt = Some(receipt(&book, ticket, Outcome::Failed));
    changed.limits.max_context_bytes -= 1;
    assert!(!ready_extension(
        &old,
        &data(&[&changed, &b]),
        b.command.action_id()
    ));
    changed.limits = a.limits;
    changed.receipt = Some(receipt(&book, ticket, Outcome::Succeeded));
    assert!(!ready_extension(
        &old,
        &data(&[&changed, &b]),
        b.command.action_id()
    ));
}

#[test]
fn submitted_or_terminal_same_slot_never_becomes_ready_and_staged_payloads_leave_native_entries_unchanged(
) {
    let e = entry(1, 1);
    let ready = data(&[&e]);
    let submitted = ExecutorCheckpoint::test_frame(
        e.anchor.agent_id,
        e.program,
        &[&e],
        &caps(),
        Some(e.command.action_id()),
    );
    assert_eq!(
        submitted.records().next().unwrap().status,
        Status::Submitted
    );
    assert_eq!(e.intents, 0);
    assert!(e.receipt.is_none());
    assert_eq!(
        submitted.command_bytes(e.command.action_id()),
        ready.command_bytes(e.command.action_id())
    );
    assert_eq!(
        submitted.recorded_limits(e.command.action_id()),
        ready.recorded_limits(e.command.action_id())
    );
    assert!(!ready_extension(&submitted, &ready, e.command.action_id()));
    assert!(!ready_extension(&ready, &submitted, e.command.action_id()));
}

fn bootstrap_limits() -> LocalExecutorLimits {
    LocalExecutorLimits {
        executor: caps(),
        manifest: ManifestLimits {
            wire: DecodeLimits::default(),
            max_blocks: 100,
            max_admissions: 10,
        },
        caller: Limits::default(),
    }
}

#[test]
fn guarded_genesis_bootstrap_ack_unknown_without_actions_or_main_rewrite_and_refuse_stale_pins_caps_or_admissions(
) {
    use crate::{
        disk::tests::Directory,
        tests::activation::{admission, output},
    };
    use std::os::unix::fs::MetadataExt;
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut admissions = AdmissionBook::new(3).unwrap();
    let book = ExecutorBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let limits = bootstrap_limits();
    let pin = crate::policy::initialize_local(
        registered,
        &admissions,
        &mut disk,
        &limits.manifest,
        &limits.caller,
    )
    .unwrap();
    let main = disk.manifest(limits.manifest.wire).read().unwrap().unwrap();
    let mut bad = pin;
    bad.manifest_digest[0] ^= 1;
    assert!(matches!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &bad, &limits),
        Err(LocalExecutorError::Conflict)
    ));
    assert!(!directory.0.join("raiv-executor.cbor").exists());
    let mut tight = limits;
    tight.executor.wire.max_block_bytes = 1;
    assert!(initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &tight).is_err());
    assert!(!directory.0.join("raiv-executor.cbor").exists());
    disk.fail_manifest_after_publish = true;
    assert!(matches!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits),
        Err(LocalExecutorError::File(ExecutorFileError::Manifest(
            ManifestError::UnknownOutcome(_)
        )))
    ));
    assert!(book.is_empty());
    let path = directory.0.join("raiv-executor.cbor");
    let inode = std::fs::metadata(&path).unwrap().ino();
    assert_eq!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits).unwrap(),
        Publication::AlreadyPresent
    );
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        disk.manifest(limits.manifest.wire).read().unwrap().unwrap(),
        main
    );
    let value = admission::input(0x61, 0x71);
    let grant = value.budget_admission();
    admissions
        .admit_actor_wake(registered, value, grant)
        .unwrap();
    assert!(matches!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits),
        Err(LocalExecutorError::NotGenesis)
    ));
    assert!(book.is_empty());
}

#[test]
fn absent_executor_amid_any_non_source_name_or_corrupt_dependency_refuses_without_cleanup_or_baseline_fallback(
) {
    use crate::{
        disk::tests::Directory,
        tests::activation::{admission, output},
    };
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let admissions = AdmissionBook::new(3).unwrap();
    let book = ExecutorBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let limits = bootstrap_limits();
    let pin = crate::policy::initialize_local(
        registered,
        &admissions,
        &mut disk,
        &limits.manifest,
        &limits.caller,
    )
    .unwrap();
    for name in ["unrelated", ".raiv-manifest-interrupted", "BAF-invalid"] {
        let path = directory.0.join(name);
        std::fs::write(&path, [0xff]).unwrap();
        assert!(matches!(
            initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits),
            Err(LocalExecutorError::NonEmptyBaseline)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), [0xff]);
        assert!(!directory.0.join("raiv-executor.cbor").exists());
        std::fs::remove_file(path).unwrap();
    }
    let foreign = raiv::format::DaslCid::for_block(raiv::format::DaslCodec::Drisl, &[0x80]);
    disk.put(foreign, &[0x80]).unwrap();
    assert!(matches!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits),
        Err(LocalExecutorError::NonEmptyBaseline)
    ));
    std::fs::remove_file(directory.0.join(foreign.to_string())).unwrap();
    let path = directory.0.join(registered.anchor().root.to_string());
    std::fs::write(&path, [0x80]).unwrap();
    assert!(initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits).is_err());
    assert_eq!(std::fs::read(path).unwrap(), [0x80]);
    assert!(!directory.0.join("raiv-executor.cbor").exists());
    assert!(book.is_empty());
}

#[test]
fn corrupt_or_foreign_executor_frame_is_not_absence_and_source_namespace_check_is_confined_to_held_fd(
) {
    use crate::{
        disk::tests::Directory,
        tests::activation::{admission, output},
    };
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let admissions = AdmissionBook::new(3).unwrap();
    let book = ExecutorBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let limits = bootstrap_limits();
    let pin = crate::policy::initialize_local(
        registered,
        &admissions,
        &mut disk,
        &limits.manifest,
        &limits.caller,
    )
    .unwrap();
    let path = directory.0.join("raiv-executor.cbor");
    std::fs::write(&path, [0xa0]).unwrap();
    assert!(initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), [0xa0]);
    std::fs::remove_file(&path).unwrap();
    let wrong = ExecutorCheckpoint::test_frame(
        [0xff; 32],
        registered.program_binding(),
        &[],
        &limits.executor,
        None,
    );
    ExecutorCheckpointFile::new(&mut disk, limits.executor)
        .replace(None, &wrong)
        .unwrap();
    assert!(matches!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits),
        Err(LocalExecutorError::Conflict)
    ));
    std::fs::remove_file(&path).unwrap();
    let moved = directory.0.with_extension("moved");
    std::fs::rename(&directory.0, &moved).unwrap();
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::write(directory.0.join("stray"), [0xff]).unwrap();
    assert_eq!(
        initialize_genesis(registered, &admissions, &book, &mut disk, &pin, &limits).unwrap(),
        Publication::Replaced
    );
    assert!(moved.join("raiv-executor.cbor").exists());
    assert!(!path.exists());
    std::fs::remove_file(directory.0.join("stray")).unwrap();
    std::fs::remove_dir(&directory.0).unwrap();
    std::fs::rename(moved, &directory.0).unwrap();
}
