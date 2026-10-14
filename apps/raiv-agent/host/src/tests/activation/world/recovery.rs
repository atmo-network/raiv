use super::*;
use crate::recovery::adopt_local;
use std::os::unix::fs::MetadataExt;
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn independently_paired_main_world_reopen_preserves_ready_submitted_terminal_and_new_origin_without_replay(
) {
    let wasm = wasm();
    let b = bounds();
    for status in [Status::Ready, Status::Submitted, Status::Terminal] {
        let mut registry = admission::registry(&wasm);
        let mut admissions = AdmissionBook::new(8).unwrap();
        let agent_dir = Directory::new();
        let world_dir = Directory::new();
        let mut agent =
            DiskBlockStore::open_existing(&agent_dir.0, DecodeLimits::default()).unwrap();
        let mut world_store =
            DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
        let reg = registry.agent([0xb0; 32]).unwrap();
        let source =
            initialize_local(reg, &admissions, &mut agent, &b.manifest, &b.caller).unwrap();
        let seed = WorldGenesis {
            world_id: [0xcc; 32],
            virtual_units: 1,
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
        let old = world
            .authorize_pending(reg, &admissions, &mut agent, &b)
            .unwrap();
        let plan = ScriptedResult {
            outcome: Outcome::Succeeded,
            result: &[0xa0],
            debit: 1,
        };
        if status != Status::Ready {
            world.begin(old, reg, &admissions, &mut agent, &b).unwrap();
        }
        if status == Status::Terminal {
            world
                .settle(old, reg, &admissions, &mut agent, plan, &b)
                .unwrap();
        }
        let pin = world.current_pin(reg, &admissions, &mut agent, &b).unwrap();
        let exact = world.state().encode(&b.executor).unwrap();
        let available = world.state().available_units();
        let recorded = world.record(old).unwrap();
        let receipt = world.receipt(old).unwrap().map(|r| r.as_bytes().to_vec());
        drop(world);
        drop(world_store);
        drop(agent);
        drop(admissions);
        drop(registry);
        let mut agent =
            DiskBlockStore::open_existing(&agent_dir.0, DecodeLimits::default()).unwrap();
        let mut store =
            DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
        let mut registry = Registry::new(1).unwrap();
        let mut admissions = AdmissionBook::new(8).unwrap();
        adopt_local(
            &wasm,
            &mut agent,
            &mut registry,
            &mut admissions,
            pin.source,
            &b.manifest,
            &b.caller,
        )
        .unwrap();
        let reg = registry.agent([0xb0; 32]).unwrap();
        let inode = std::fs::metadata(world_dir.0.join("raiv-world.cbor"))
            .unwrap()
            .ino();
        let mut world =
            ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &b).unwrap();
        assert_eq!(world.state().encode(&b.executor).unwrap(), exact);
        assert_eq!(world.state().available_units(), available);
        assert_eq!(world.state().ledger().records().next().unwrap(), recorded);
        assert_eq!(
            std::fs::metadata(world_dir.0.join("raiv-world.cbor"))
                .unwrap()
                .ino(),
            inode
        );
        assert!(matches!(
            world.record(old),
            Err(WorldError::Simulator(SimulatorError::ForeignTicket))
        ));
        assert!(matches!(
            world.begin(old, reg, &admissions, &mut agent, &b),
            Err(WorldError::Simulator(SimulatorError::ForeignTicket))
        ));
        // Even zero-available Submitted/Terminal may obtain correlation, NOT a new intent.
        let fresh = world
            .authorize_pending(reg, &admissions, &mut agent, &b)
            .unwrap();
        assert_ne!(old, fresh);
        assert_eq!(world.record(fresh).unwrap(), recorded);
        assert_eq!(
            world.receipt(fresh).unwrap().map(|r| r.as_bytes().to_vec()),
            receipt
        );
        if status == Status::Ready {
            world
                .begin(fresh, reg, &admissions, &mut agent, &b)
                .unwrap();
        } else {
            assert!(matches!(
                world.begin(fresh, reg, &admissions, &mut agent, &b),
                Err(WorldError::Simulator(SimulatorError::AlreadySubmitted))
            ));
        }
        world
            .settle(fresh, reg, &admissions, &mut agent, plan, &b)
            .unwrap();
        assert_eq!(world.state().available_units(), 0);
        let settled = world.state().encode(&b.executor).unwrap();
        world
            .settle(fresh, reg, &admissions, &mut agent, plan, &b)
            .unwrap();
        assert_eq!(world.state().encode(&b.executor).unwrap(), settled);
        let receipt = raiv::receipt::Receipt::decode(
            world.receipt(fresh).unwrap().unwrap().as_bytes(),
            &b.caller,
        )
        .unwrap();
        let input = action_result::input(&receipt, 0x61, 0x71);
        let approved_grant = input.budget_admission();
        assert!(matches!(
            world.admit_action_result(
                old,
                reg,
                &mut admissions,
                &mut agent,
                LedgerActionResult {
                    input,
                    approved_grant
                },
                &b
            ),
            Err(WorldError::Simulator(SimulatorError::ForeignTicket))
        ));
        let input = action_result::input(&receipt, 0x61, 0x71);
        let approved_grant = input.budget_admission();
        let token = world
            .admit_action_result(
                fresh,
                reg,
                &mut admissions,
                &mut agent,
                LedgerActionResult {
                    input,
                    approved_grant,
                },
                &b,
            )
            .unwrap();
        commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
        assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor().revision, 2);
        assert_eq!(world.state().encode(&b.executor).unwrap(), settled);
    }
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn world_adoption_pin_source_capacity_caps_missing_corruption_refuse_without_reseed_or_partial_session(
) {
    let wasm = wasm();
    let b = bounds();
    let mut registry = admission::registry(&wasm);
    let mut admissions = AdmissionBook::new(8).unwrap();
    let agent_dir = Directory::new();
    let world_dir = Directory::new();
    let mut agent = DiskBlockStore::open_existing(&agent_dir.0, DecodeLimits::default()).unwrap();
    let mut store = DiskBlockStore::open_existing(&world_dir.0, DecodeLimits::default()).unwrap();
    let reg = registry.agent([0xb0; 32]).unwrap();
    let source = initialize_local(reg, &admissions, &mut agent, &b.manifest, &b.caller).unwrap();
    let seed = WorldGenesis {
        world_id: [0xcc; 32],
        virtual_units: 1,
        capacity: 4,
        source,
    };
    let mut world =
        ScriptedWorld::bootstrap(reg, &admissions, &mut agent, &mut store, &seed, &b).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = admissions.admit_actor_wake(reg, input, grant).unwrap();
    commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
    let reg = registry.agent([0xb0; 32]).unwrap();
    let ticket = world
        .authorize_pending(reg, &admissions, &mut agent, &b)
        .unwrap();
    world
        .begin(ticket, reg, &admissions, &mut agent, &b)
        .unwrap();
    let pin = world.current_pin(reg, &admissions, &mut agent, &b).unwrap();
    let exact = world.state().encode(&b.executor).unwrap();
    world.fail_after_publish(true);
    let plan = ScriptedResult {
        outcome: Outcome::Succeeded,
        result: &[0xa0],
        debit: 1,
    };
    assert!(world
        .settle(ticket, reg, &admissions, &mut agent, plan, &b)
        .is_err());
    assert!(matches!(
        world.current_pin(reg, &admissions, &mut agent, &b),
        Err(WorldError::Conflict)
    ));
    world.fail_after_publish(false);
    world
        .settle(ticket, reg, &admissions, &mut agent, plan, &b)
        .unwrap();
    let newer = world.state().encode(&b.executor).unwrap();
    let terminal_pin = world.current_pin(reg, &admissions, &mut agent, &b).unwrap();
    drop(world);
    // Prior independently selected pending pin cannot select the now terminal primary.
    assert!(matches!(
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &b),
        Err(WorldError::Binding)
    ));
    let pin = terminal_pin;
    let mut wrong = pin;
    wrong.world_digest = [0xff; 32];
    assert!(matches!(
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &wrong, &b),
        Err(WorldError::Binding)
    ));
    wrong = pin;
    wrong.source.manifest_digest = [0xff; 32];
    assert!(matches!(
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &wrong, &b),
        Err(WorldError::Binding)
    ));
    let foreign = registry.agent([0xb1; 32]).unwrap();
    assert!(matches!(
        ScriptedWorld::adopt(foreign, &admissions, &mut agent, &mut store, 4, &pin, &b),
        Err(WorldError::Binding)
    ));
    let tight = LocalExecutorLimits {
        caller: Limits {
            max_command_bytes: 0,
            ..b.caller
        },
        ..bounds()
    };
    assert!(
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &tight).is_err()
    );
    let tight = LocalExecutorLimits {
        caller: Limits {
            max_receipt_bytes: 0,
            ..b.caller
        },
        ..bounds()
    };
    assert!(
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &tight).is_err()
    );
    assert!(ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 0, &pin, &b).is_err());
    let path = world_dir.0.join("raiv-world.cbor");
    assert_eq!(std::fs::read(&path).unwrap(), newer);
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &b),
        Err(WorldError::Missing)
    ));
    assert!(matches!(
        ScriptedWorld::bootstrap(reg, &admissions, &mut agent, &mut store, &seed, &b),
        Err(WorldError::NotGenesis)
    ));
    std::fs::write(&path, [0xff]).unwrap();
    assert!(ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &b).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), [0xff]);
    // Owned-test repair only: diagnostic refusal never rewrote this data.
    std::fs::write(&path, &newer).unwrap();
    let root = reg.anchor().root;
    let source_path = agent_dir.0.join(root.to_string());
    let source_bytes = std::fs::read(&source_path).unwrap();
    std::fs::remove_file(&source_path).unwrap();
    assert!(ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &b).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), newer);
    std::fs::write(&source_path, &source_bytes).unwrap();
    let mut restored =
        ScriptedWorld::adopt(reg, &admissions, &mut agent, &mut store, 4, &pin, &b).unwrap();
    assert_eq!(restored.state().available_units(), 0);
    let fresh = restored
        .authorize_pending(reg, &admissions, &mut agent, &b)
        .unwrap();
    assert!(restored.receipt(fresh).unwrap().is_some());
    assert_ne!(restored.state().encode(&b.executor).unwrap(), exact);
}
