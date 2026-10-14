//! Scripted receipt provenance/late storage refusal must preserve every observable book.
use super::*;
use crate::admission::AdmissionError;
fn request(receipt: &raiv::receipt::Receipt, event: u8, grant: u8) -> LedgerActionResult {
    let input = action_result::input(receipt, event, grant);
    let approved_grant = input.budget_admission();
    LedgerActionResult {
        input,
        approved_grant,
    }
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn ledger_admission_refuses_uninstalled_forged_foreign_replay_caps_and_late_storage_failures_atomically(
) {
    let wasm = wasm();
    let b = bounds();
    let mut registry = super::super::admission::registry(&wasm);
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
    let input = super::super::admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = admissions.admit_actor_wake(reg, input, grant).unwrap();
    commit(&wasm, &mut registry, &mut admissions, &mut agent, &token);
    let reg = registry.agent([0xb0; 32]).unwrap();
    let ticket = world
        .authorize_pending(reg, &admissions, &mut agent, &b)
        .unwrap();
    let fabricated = action_result::receipt(reg.snapshot(), Outcome::Failed);
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&fabricated, 0x61, 0x71),
            &b
        ),
        Err(WorldError::NotTerminal)
    ));
    world
        .begin(ticket, reg, &admissions, &mut agent, &b)
        .unwrap();
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&fabricated, 0x61, 0x71),
            &b
        ),
        Err(WorldError::NotTerminal)
    ));
    let plan = ScriptedResult {
        outcome: Outcome::Failed,
        result: &[0xa0],
        debit: 1,
    };
    world.fail_after_publish(true);
    assert!(world
        .settle(ticket, reg, &admissions, &mut agent, plan, &b)
        .is_err());
    // A file-supplied terminal alone cannot grant admission while native receipt is absent.
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&fabricated, 0x61, 0x71),
            &b
        ),
        Err(WorldError::NotTerminal)
    ));
    assert_eq!(admissions.len(), 1);
    world.fail_after_publish(false);
    world
        .settle(ticket, reg, &admissions, &mut agent, plan, &b)
        .unwrap();
    let receipt = raiv::receipt::Receipt::decode(
        world.receipt(ticket).unwrap().unwrap().as_bytes(),
        &b.caller,
    )
    .unwrap();
    let exact = world.state().encode(&b.executor).unwrap();
    let record = world.record(ticket).unwrap();
    let old_admission = admissions.record([0xb0; 32], [0x60; 32]);
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&fabricated, 0x61, 0x71),
            &b
        ),
        Err(WorldError::Simulator(SimulatorError::Admission(
            AdmissionError::ReceiptMismatch
        )))
    ));
    let mut wrong = request(&receipt, 0x61, 0x71);
    wrong.approved_grant.admission_id = [0xff; 32];
    assert!(matches!(
        world.admit_action_result(ticket, reg, &mut admissions, &mut agent, wrong, &b),
        Err(WorldError::Simulator(SimulatorError::Admission(
            AdmissionError::GrantMismatch
        )))
    ));
    for (event, grant) in [(0x60, 0x71), (0x61, 0x70)] {
        assert!(world
            .admit_action_result(
                ticket,
                reg,
                &mut admissions,
                &mut agent,
                request(&receipt, event, grant),
                &b
            )
            .is_err());
    }
    let mut other = crate::simulator::ExecutorBook::new(2).unwrap();
    let foreign = other
        .authorize_pending(reg, &admissions, &mut agent, &b.manifest, &b.caller)
        .unwrap();
    assert!(matches!(
        world.admit_action_result(
            foreign,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        ),
        Err(WorldError::Simulator(SimulatorError::ForeignTicket))
    ));
    let unrelated = registry.agent([0xb1; 32]).unwrap();
    assert!(matches!(
        world.admit_action_result(
            ticket,
            unrelated,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        ),
        Err(WorldError::Binding)
    ));
    let tight = LocalExecutorLimits {
        caller: Limits {
            max_receipt_bytes: 0,
            ..b.caller
        },
        ..bounds()
    };
    assert!(world
        .admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &tight
        )
        .is_err());
    // Same approved source data, full destination admission capacity: no late reservation.
    let records: Vec<_> = admissions.records_for(reg).unwrap().collect();
    let mut full = AdmissionBook::new(1).unwrap();
    let staged = full.stage_recovered(reg, &records).unwrap();
    full.publish_recovered(staged);
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut full,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        ),
        Err(WorldError::Simulator(SimulatorError::Admission(
            AdmissionError::Capacity
        )))
    ));
    let path = world_dir.0.join("raiv-world.cbor");
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        ),
        Err(WorldError::Missing)
    ));
    std::fs::write(&path, [0xff]).unwrap();
    assert!(world
        .admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        )
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), [0xff]);
    // Valid-but-different primary seed/balance is DATA, not renewed funding approval.
    let mut changed = decode(&exact);
    let Value::Array(fields) = &mut changed else {
        panic!()
    };
    fields[3] = 2.into();
    fields[4] = 1.into();
    let changed = encode(&changed);
    std::fs::write(&path, &changed).unwrap();
    assert!(matches!(
        world.admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        ),
        Err(WorldError::Conflict)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), changed);
    // Repairs restore ONLY the exact owned-test prior bytes; production never repairs.
    std::fs::write(&path, &exact).unwrap();
    let source_path = agent_dir.0.join(reg.anchor().root.to_string());
    let bytes = std::fs::read(&source_path).unwrap();
    std::fs::remove_file(&source_path).unwrap();
    assert!(world
        .admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        )
        .is_err());
    std::fs::write(&source_path, &bytes).unwrap();
    let main_path = agent_dir.0.join("raiv-manifest.cbor");
    let main = std::fs::read(&main_path).unwrap();
    std::fs::remove_file(&main_path).unwrap();
    assert!(world
        .admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x61, 0x71),
            &b
        )
        .is_err());
    assert!(!main_path.exists());
    std::fs::write(&main_path, &main).unwrap();
    assert_eq!(world.state().encode(&b.executor).unwrap(), exact);
    assert_eq!(world.record(ticket).unwrap(), record);
    assert_eq!(world.state().available_units(), 0);
    assert_eq!(admissions.len(), 1);
    assert_eq!(admissions.record([0xb0; 32], [0x60; 32]), old_admission);
    assert_eq!(admissions.record([0xb0; 32], [0x61; 32]), None);
    let value = request(&receipt, 0x61, 0x71);
    let input_bytes = value.input.as_bytes().to_vec();
    let approved = value.approved_grant;
    let token = world
        .admit_action_result(ticket, reg, &mut admissions, &mut agent, value, &b)
        .unwrap();
    assert_eq!(token.packet().input().as_bytes(), input_bytes);
    assert_eq!(token.packet().input().budget_admission(), approved);
    assert_eq!(admissions.len(), 2);
    assert_eq!(
        admissions
            .record([0xb0; 32], [0x61; 32])
            .unwrap()
            .launch_attempts,
        0
    );
    assert_eq!(std::fs::read(&main_path).unwrap(), main);
    assert_eq!(std::fs::read(&path).unwrap(), exact);
    drop(token);
    assert!(world
        .admit_action_result(
            ticket,
            reg,
            &mut admissions,
            &mut agent,
            request(&receipt, 0x62, 0x72),
            &b
        )
        .is_err());
    assert_eq!(admissions.len(), 2);
    assert_eq!(world.state().encode(&b.executor).unwrap(), exact);
}
