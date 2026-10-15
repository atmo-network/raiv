//! Actual Wasmi decisions -> independently held atomic scripted virtual world, not market execution.

mod ledger_admission;
mod recovery;

use super::*;
use crate::{
    acceptance::accept_local,
    admission::{AdmissionBook, AdmittedInput},
    checkpoint::ManifestLimits,
    disk::{tests::Directory, DiskBlockStore},
    launch::{launch_local, record_reserved},
    policy::initialize_local,
    registry::Registry,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{
            LedgerActionResult, ScriptedResult, ScriptedWorld, WorldError, WorldGenesis, WorldState,
        },
        SimulatorError, Status,
    },
};
use raiv::{format::DecodeLimits, limits::Limits, receipt::Outcome};

fn bounds() -> LocalExecutorLimits {
    LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: DecodeLimits::default(),
            max_actions: 10,
        },
        manifest: ManifestLimits {
            wire: DecodeLimits::default(),
            max_blocks: 100,
            max_admissions: 10,
        },
        caller: Limits::default(),
    }
}

fn commit(
    wasm: &[u8],
    registry: &mut Registry,
    admissions: &mut AdmissionBook,
    disk: &mut DiskBlockStore,
    token: &AdmittedInput,
) {
    let b = bounds();
    let registered = registry.agent([0xb0; 32]).unwrap();
    record_reserved(registered, admissions, token, disk, &b.manifest, &b.caller).unwrap();
    let mut run = launch_local(
        wasm,
        registered,
        admissions,
        token,
        disk,
        &b.manifest,
        &b.caller,
    )
    .unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "trade", map(vec![])))
        .unwrap();
    let mut sealed = run.into_sealed().unwrap();
    accept_local(
        registry,
        admissions,
        token,
        &mut sealed,
        disk,
        &b.manifest,
        &b.caller,
    )
    .unwrap();
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn independent_primary_world_failed_retry_success_debits_once_and_old_receipt_never_rolls_back() {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut admissions = AdmissionBook::new(8).unwrap();
    let agent_dir = Directory::new();
    let world_dir = Directory::new();
    let mut agent = DiskBlockStore::open_existing(&agent_dir.0, DecodeLimits::default()).unwrap();
    let mut world_store =
        DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
    let b = bounds();
    let reg = registry.agent([0xb0; 32]).unwrap();
    let source = initialize_local(reg, &admissions, &mut agent, &b.manifest, &b.caller).unwrap();
    let seed = WorldGenesis {
        world_id: [0xcc; 32],
        virtual_units: 2,
        capacity: 4,
        source,
    };
    let mut world =
        ScriptedWorld::bootstrap(reg, &admissions, &mut agent, &mut world_store, &seed, &b)
            .unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = admissions.admit_actor_wake(reg, input, grant).unwrap();
    commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
    let reg = registry.agent([0xb0; 32]).unwrap();
    let first = world
        .authorize_pending(reg, &admissions, &mut agent, &b)
        .unwrap();
    world
        .begin(first, reg, &admissions, &mut agent, &b)
        .unwrap();
    assert_eq!(world.state().available_units(), 1);
    let failed = ScriptedResult {
        outcome: Outcome::Failed,
        result: &[0xa0],
        debit: 1,
    };
    world
        .settle(first, reg, &admissions, &mut agent, failed, &b)
        .unwrap();
    assert_eq!(world.state().available_units(), 1);
    let retained = world.state().encode(&b.executor).unwrap();
    let other = registry.agent([0xb1; 32]).unwrap();
    assert!(matches!(
        world.authorize_pending(other, &admissions, &mut agent, &b),
        Err(WorldError::Binding)
    ));
    assert!(matches!(
        world.begin(first, other, &admissions, &mut agent, &b),
        Err(WorldError::Binding)
    ));
    assert!(matches!(
        world.settle(first, other, &admissions, &mut agent, failed, &b),
        Err(WorldError::Binding)
    ));
    let foreign = admission::registry(&output::invalid_output_module());
    let foreign = foreign.agent([0xb0; 32]).unwrap();
    assert!(matches!(
        world.authorize_pending(foreign, &admissions, &mut agent, &b),
        Err(WorldError::Binding)
    ));
    assert!(matches!(
        world.begin(first, foreign, &admissions, &mut agent, &b),
        Err(WorldError::Binding)
    ));
    assert!(matches!(
        world.settle(first, foreign, &admissions, &mut agent, failed, &b),
        Err(WorldError::Binding)
    ));
    assert_eq!(world.state().encode(&b.executor).unwrap(), retained);
    let receipt = raiv::receipt::Receipt::decode(
        world.receipt(first).unwrap().unwrap().as_bytes(),
        &b.caller,
    )
    .unwrap();
    let value = action_result::input(&receipt, 0x61, 0x71);
    let grant = value.budget_admission();
    let token = world
        .admit_action_result(
            first,
            reg,
            &mut admissions,
            &mut agent,
            LedgerActionResult {
                input: value,
                approved_grant: grant,
            },
            &b,
        )
        .unwrap();
    commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
    let reg = registry.agent([0xb0; 32]).unwrap();
    let stale_input = action_result::input(&receipt, 0x62, 0x72);
    let stale_grant = stale_input.budget_admission();
    assert!(matches!(
        world.admit_action_result(
            first,
            reg,
            &mut admissions,
            &mut agent,
            LedgerActionResult {
                input: stale_input,
                approved_grant: stale_grant
            },
            &b
        ),
        Err(WorldError::Binding)
    ));
    assert_eq!(admissions.len(), 2);
    let second = world
        .authorize_pending(reg, &admissions, &mut agent, &b)
        .unwrap();
    assert_ne!(first, second);
    world
        .begin(second, reg, &admissions, &mut agent, &b)
        .unwrap();
    assert_eq!(world.state().available_units(), 0);
    let succeeded = ScriptedResult {
        outcome: Outcome::Succeeded,
        result: &[0xa0],
        debit: 1,
    };
    world
        .settle(second, reg, &admissions, &mut agent, succeeded, &b)
        .unwrap();
    let exact = world.state().encode(&b.executor).unwrap();
    world
        .settle(first, reg, &admissions, &mut agent, failed, &b)
        .unwrap();
    world
        .settle(second, reg, &admissions, &mut agent, succeeded, &b)
        .unwrap();
    assert_eq!(world.state().encode(&b.executor).unwrap(), exact);
    assert!(matches!(
        world.begin(second, reg, &admissions, &mut agent, &b),
        Err(WorldError::Simulator(SimulatorError::AlreadySubmitted))
    ));
    assert!(matches!(
        world.settle(first, reg, &admissions, &mut agent, succeeded, &b),
        Err(WorldError::Simulator(SimulatorError::TerminalConflict))
    ));
    assert_eq!(
        world
            .authorize_pending(reg, &admissions, &mut agent, &b)
            .unwrap(),
        second
    );
    let receipt = raiv::receipt::Receipt::decode(
        world.receipt(second).unwrap().unwrap().as_bytes(),
        &b.caller,
    )
    .unwrap();
    let value = action_result::input(&receipt, 0x62, 0x72);
    let grant = value.budget_admission();
    let token = world
        .admit_action_result(
            second,
            reg,
            &mut admissions,
            &mut agent,
            LedgerActionResult {
                input: value,
                approved_grant: grant,
            },
            &b,
        )
        .unwrap();
    commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
    let reg = registry.agent([0xb0; 32]).unwrap();
    assert!(matches!(
        world.authorize_pending(reg, &admissions, &mut agent, &b),
        Err(WorldError::Exhausted)
    ));
    assert_eq!(world.state().encode(&b.executor).unwrap(), exact);
    let selected = world.current_pin(reg, &admissions, &mut agent, &b).unwrap();
    drop(world);
    drop(world_store);
    let mut reopened =
        DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
    let recovered = WorldState::read(&mut reopened, &b.executor).unwrap();
    assert_eq!(recovered.encode(&b.executor).unwrap(), exact);
    assert_eq!(recovered.available_units(), 0);
    assert!(matches!(
        ScriptedWorld::adopt(
            reg,
            &admissions,
            &mut agent,
            &mut reopened,
            1,
            &selected,
            &b
        ),
        Err(WorldError::Checkpoint(
            crate::simulator::checkpoint::ExecutorCheckpointError::Simulator(
                SimulatorError::Capacity
            )
        ))
    ));
    assert_eq!(
        std::fs::read(world_dir.0.join("raiv-world.cbor")).unwrap(),
        exact
    );
    assert_eq!(
        recovered
            .ledger()
            .records()
            .filter(|r| r.status == Status::Terminal)
            .count(),
        2
    );
    // Stale Core source / empty native reconstruction cannot re-seed the independent world.
    let stale = admission::registry(&wasm);
    let empty = AdmissionBook::new(8).unwrap();
    assert!(matches!(
        ScriptedWorld::bootstrap(
            stale.agent([0xb0; 32]).unwrap(),
            &empty,
            &mut agent,
            &mut reopened,
            &seed,
            &b
        ),
        Err(WorldError::Conflict)
    ));
    assert!(!agent_dir.0.join("raiv-world.cbor").exists());
    assert!(!world_dir.0.join("raiv-manifest.cbor").exists());
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn unknown_world_publication_keeps_reservation_terminal_conflicts_refuse_and_known_zero_releases_only_explicitly(
) {
    let wasm = wasm();
    for terminal_unknown in [false, true] {
        let mut registry = admission::registry(&wasm);
        let mut admissions = AdmissionBook::new(8).unwrap();
        let agent_dir = Directory::new();
        let world_dir = Directory::new();
        let mut agent =
            DiskBlockStore::open_existing(&agent_dir.0, DecodeLimits::default()).unwrap();
        let mut world_store =
            DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
        let b = bounds();
        let reg = registry.agent([0xb0; 32]).unwrap();
        let source =
            initialize_local(reg, &admissions, &mut agent, &b.manifest, &b.caller).unwrap();
        let seed = WorldGenesis {
            world_id: [0xcc; 32],
            virtual_units: 1,
            capacity: 4,
            source,
        };
        std::fs::write(world_dir.0.join("orphan"), [0]).unwrap();
        assert!(matches!(
            ScriptedWorld::bootstrap(reg, &admissions, &mut agent, &mut world_store, &seed, &b),
            Err(WorldError::NonEmptyDirectory)
        ));
        std::fs::remove_file(world_dir.0.join("orphan")).unwrap();
        let mut world =
            ScriptedWorld::bootstrap(reg, &admissions, &mut agent, &mut world_store, &seed, &b)
                .unwrap();
        let input = admission::input(0x60, 0x70);
        let grant = input.budget_admission();
        let token = admissions.admit_actor_wake(reg, input, grant).unwrap();
        commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
        let reg = registry.agent([0xb0; 32]).unwrap();
        let ticket = world
            .authorize_pending(reg, &admissions, &mut agent, &b)
            .unwrap();
        let ready = world.state().encode(&b.executor).unwrap();
        if !terminal_unknown {
            world.fail_after_publish(true);
        }
        let started = world.begin(ticket, reg, &admissions, &mut agent, &b);
        if terminal_unknown {
            started.unwrap();
        } else {
            assert!(matches!(started, Err(WorldError::Manifest(_))));
        }
        assert_eq!(world.record(ticket).unwrap().status, Status::Submitted);
        assert_eq!(world.state().available_units(), 0);
        assert!(world.receipt(ticket).unwrap().is_none());
        assert_ne!(world.state().encode(&b.executor).unwrap(), ready);
        assert!(matches!(
            world.begin(ticket, reg, &admissions, &mut agent, &b),
            Err(WorldError::Simulator(SimulatorError::AlreadySubmitted))
        ));
        world.fail_after_publish(terminal_unknown);
        let plan = ScriptedResult {
            outcome: Outcome::NotExecuted,
            result: &[0xa0],
            debit: 0,
        };
        let mut bad = plan;
        bad.debit = 1;
        assert!(matches!(
            world.settle(ticket, reg, &admissions, &mut agent, bad, &b),
            Err(WorldError::Schema)
        ));
        let submitted = world.state().encode(&b.executor).unwrap();
        let terminal = world.settle(ticket, reg, &admissions, &mut agent, plan, &b);
        if terminal_unknown {
            assert!(matches!(terminal, Err(WorldError::Manifest(_))));
            assert_eq!(world.state().encode(&b.executor).unwrap(), submitted);
            assert_eq!(world.state().available_units(), 0);
            assert!(world.receipt(ticket).unwrap().is_none());
            world.fail_after_publish(false);
            let conflicting = ScriptedResult {
                outcome: Outcome::Succeeded,
                result: &[0xa0],
                debit: 1,
            };
            assert!(matches!(
                world.settle(ticket, reg, &admissions, &mut agent, conflicting, &b),
                Err(WorldError::Conflict)
            ));
            assert_eq!(world.state().encode(&b.executor).unwrap(), submitted);
            world
                .settle(ticket, reg, &admissions, &mut agent, plan, &b)
                .unwrap();
        } else {
            terminal.unwrap();
        }
        assert_eq!(world.state().available_units(), 1);
        assert_eq!(world.record(ticket).unwrap().status, Status::Terminal);
        let exact = world.state().encode(&b.executor).unwrap();
        world
            .settle(ticket, reg, &admissions, &mut agent, plan, &b)
            .unwrap();
        assert_eq!(world.state().encode(&b.executor).unwrap(), exact);
        drop(world);
        drop(world_store);
        let mut store =
            DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
        assert_eq!(
            WorldState::read(&mut store, &b.executor)
                .unwrap()
                .encode(&b.executor)
                .unwrap(),
            exact
        );
    }
}
