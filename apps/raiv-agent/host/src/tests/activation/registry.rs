//! Explicit local administration, not fresh grant/provenance, storage or effect permission.
use super::*;
use crate::{
    registry::{Registry, RegistryError},
    run::{Phase, ProgramBinding, RunError},
};
use raiv::{graph::RestoredAgent, limits::Limits};
pub(super) fn different_agent() -> RestoredAgent {
    let mut init = decode(&frame(1));
    let mut payload = field(&init, "payload").clone();
    set(&mut payload, "agent_id", Value::Bytes(vec![0xb1; 32]));
    set(&mut init, "payload", payload);
    let mut guest = guest::Guest::new();
    let bytes = guest.step(&encode(&init)).unwrap();
    let crate::output::Output::Initialized { root, blocks } =
        crate::output::Output::decode(&bytes, &Limits::default()).unwrap()
    else {
        panic!()
    };
    RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap()
}
#[test]
fn administrative_registration_is_atomic_capacity_bounded_genesis_only_and_never_overwrites() {
    assert!(matches!(
        Registry::new(0),
        Err(RegistryError::InvalidCapacity)
    ));
    let wasm = output::invalid_output_module();
    let mut registry = Registry::new(1).unwrap();
    assert!(registry.is_empty());
    let source = routing::source();
    let pin = routing::pin(&wasm, &source);
    let id = source.state().agent_id();
    for bad in [
        ProgramBinding {
            module_digest: [0; 32],
            ..pin
        },
        ProgramBinding {
            definition: source.root(),
            ..pin
        },
        ProgramBinding {
            execution_domain: [0; 32],
            ..pin
        },
    ] {
        assert!(matches!(
            registry.register_genesis(&wasm, bad, routing::source(), VmLimits::default()),
            Err(RegistryError::Program(RunError::ProgramBinding))
        ));
        assert!(registry.is_empty());
        assert!(registry.agent(id).is_none());
    }
    assert!(matches!(
        registry.register_genesis(
            &wasm,
            pin,
            routing::source(),
            VmLimits {
                memory_bytes: 0,
                ..VmLimits::default()
            }
        ),
        Err(RegistryError::Bridge(_))
    ));
    assert!(registry.is_empty());
    // Import-free export/range conformance is not an assertion about future output correctness.
    registry
        .register_genesis(&wasm, pin, source, VmLimits::default())
        .unwrap();
    assert_eq!(registry.len(), 1);
    let anchor = registry.agent(id).unwrap().anchor();
    assert_eq!(anchor.revision, 0);
    assert_eq!(registry.agent(id).unwrap().program_binding(), pin);
    assert_eq!(registry.agent(id).unwrap().snapshot().root(), anchor.root);
    assert!(matches!(
        registry.register_genesis(&wasm, pin, routing::source(), VmLimits::default()),
        Err(RegistryError::AlreadyRegistered)
    ));
    let other = different_agent();
    assert!(matches!(
        registry.register_genesis(
            &wasm,
            routing::pin(&wasm, &other),
            other,
            VmLimits::default()
        ),
        Err(RegistryError::Capacity)
    ));
    assert_eq!(registry.agent(id).unwrap().anchor(), anchor);
    assert_eq!(registry.len(), 1);
    let mut empty = Registry::new(1).unwrap();
    let source = routing::source();
    let packet = routing::prepared(&source);
    let mut guest = guest::Guest::new();
    guest
        .step(&activate(&genesis(1), &super::packet()))
        .unwrap();
    let response = guest.step(&progression::reply(1, 1, vec![])).unwrap();
    let crate::output::Output::Boundary(candidate) =
        crate::output::Output::decode(&response, &Limits::default()).unwrap()
    else {
        panic!()
    };
    let post = crate::candidate::restore(candidate, &source, &packet, &Limits::default()).unwrap();
    assert!(matches!(
        empty.register_genesis(&wasm, pin, post, VmLimits::default()),
        Err(RegistryError::NotGenesis)
    ));
    assert!(empty.is_empty());
}
#[test]
fn self_supplied_valid_pins_malformed_modules_imports_and_wrong_exports_do_not_approve_code() {
    let mut registry = Registry::new(1).unwrap();
    for module in [
        b"not wasm".to_vec(),
        b"\0asm\x01\0\0\0".to_vec(),
        b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x02\x07\x01\x01m\x01f\0\0".to_vec(),
    ] {
        let source = routing::source();
        let pin = routing::pin(&module, &source);
        assert!(matches!(
            registry.register_genesis(&module, pin, source, VmLimits::default()),
            Err(RegistryError::Bridge(_))
        ));
        assert!(registry.is_empty());
    }
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn registered_anchor_launches_only_its_approved_module_source_and_retains_no_dispatch_or_head_update(
) {
    let wasm = wasm();
    let source = routing::source();
    let pin = routing::pin(&wasm, &source);
    let id = source.state().agent_id();
    let packet = routing::prepared(&source);
    let mut registry = Registry::new(2).unwrap();
    registry
        .register_genesis(&wasm, pin, source, VmLimits::default())
        .unwrap();
    let registered = registry.agent(id).unwrap();
    let anchor = registered.anchor();
    assert!(matches!(
        registered.start_run(b"other module", &packet),
        Err(RegistryError::Program(RunError::ProgramBinding))
    ));
    let other = different_agent();
    let other_packet = routing::prepared(&other);
    assert!(matches!(
        registered.start_run(&wasm, &other_packet),
        Err(RegistryError::Program(RunError::InputBinding))
    ));
    let mut run = registered.start_run(&wasm, &packet).unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "trade", map(vec![])))
        .unwrap();
    assert_eq!(run.phase(), Phase::Boundary);
    assert_ne!(run.candidate_snapshot().unwrap().root(), anchor.root);
    assert_eq!(registered.anchor(), anchor);
    assert_eq!(registered.anchor().revision, 0);
    assert!(matches!(
        run.inference_error(ticket, "late"),
        Err(RunError::Phase)
    ));
    // Administrative registration did not consume/admit the event. No freshness claim is made:
    // a second computation is possible until the admission/acceptance ledger is implemented.
    let second = registered.start_run(&wasm, &packet).unwrap();
    assert_eq!(second.pending().unwrap().0.request_id(), 1);
    assert_eq!(registered.anchor(), anchor);
    // Program identity is the pinned byte digest, never file name/path or ABI version alone.
    let mut modified = wasm.clone();
    modified.push(0);
    assert!(matches!(
        registered.start_run(&modified, &packet),
        Err(RegistryError::Program(RunError::ProgramBinding))
    ));
}
