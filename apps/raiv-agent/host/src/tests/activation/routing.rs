//! Native process-local live routing; pins/data are not fresh admission or acceptance.

use super::*;
use crate::{
    output::Output,
    run::{Phase, ProgramBinding, Run, RunError},
};
use raiv::{
    format::{DaslCid, DaslCodec},
    graph::RestoredAgent,
    input::ActivationInput,
    limits::Limits,
    preparation::{InputBinding, PreparedInput},
};

pub(super) fn source() -> RestoredAgent {
    let bytes = encode(&genesis(1));
    let Output::Initialized { root, blocks } = Output::decode(&bytes, &Limits::default()).unwrap()
    else {
        panic!()
    };
    RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap()
}

pub(super) fn prepared(source: &RestoredAgent) -> PreparedInput {
    let input = ActivationInput::decode(&encode(&packet()), &Limits::default()).unwrap();
    PreparedInput::new(
        InputBinding {
            agent_id: source.state().agent_id(),
            execution_domain: source.definition().execution_domain(),
            expected_root: source.root(),
            expected_revision: source.state().revision(),
            admitted_event_id: input.event_id(),
            admitted_input_digest: input.cid().digest(),
        },
        input,
    )
    .unwrap()
}

pub(super) fn pin(wasm: &[u8], source: &RestoredAgent) -> ProgramBinding {
    ProgramBinding {
        module_digest: DaslCid::for_block(DaslCodec::Raw, wasm).digest(),
        definition: source.state().definition_link(),
        execution_domain: source.definition().execution_domain(),
    }
}

pub(super) fn normalized(id: u64, tool: &str, args: Value) -> Vec<u8> {
    let wire = decode(&progression::reply(
        id,
        id,
        if tool.is_empty() {
            vec![]
        } else {
            vec![progression::call(tool, args)]
        },
    ));
    encode(field(field(&wire, "payload"), "reply"))
}

#[test]
fn program_and_source_packet_substitution_fail_before_wasm_instantiation() {
    let source = source();
    let original = prepared(&source);
    let bad_wasm = b"not a module";
    let correct = pin(bad_wasm, &source);
    for changed in [
        ProgramBinding {
            module_digest: [0; 32],
            ..correct
        },
        ProgramBinding {
            definition: source.root(),
            ..correct
        },
        ProgramBinding {
            execution_domain: [0; 32],
            ..correct
        },
    ] {
        assert!(matches!(
            Run::start(bad_wasm, changed, &source, &original, VmLimits::default()),
            Err(RunError::ProgramBinding)
        ));
    }
    let b = original.binding();
    for changed in [
        InputBinding {
            agent_id: [0; 32],
            ..b
        },
        InputBinding {
            execution_domain: [0; 32],
            ..b
        },
        InputBinding {
            expected_root: correct.definition,
            ..b
        },
        InputBinding {
            expected_revision: u64::MAX,
            ..b
        },
    ] {
        let input =
            ActivationInput::decode(original.input().as_bytes(), &Limits::default()).unwrap();
        let packet = PreparedInput::new(changed, input).unwrap();
        assert!(matches!(
            Run::start(bad_wasm, correct, &source, &packet, VmLimits::default()),
            Err(RunError::InputBinding)
        ));
    }
    // A correct explicit DATA pin still does not turn an invalid module into executable code.
    assert!(matches!(
        Run::start(bad_wasm, correct, &source, &original, VmLimits::default()),
        Err(RunError::Bridge(_))
    ));
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_runs_with_same_guest_request_id_refuse_cross_instance_stale_duplicate_and_malformed_replies(
) {
    let wasm = wasm();
    let source = source();
    let packet = prepared(&source);
    let program = pin(&wasm, &source);
    let mut a = Run::start(&wasm, program, &source, &packet, VmLimits::default()).unwrap();
    let mut b = Run::start(&wasm, program, &source, &packet, VmLimits::default()).unwrap();
    let (first_a, request) = a.pending().unwrap();
    let prefix = decode(request.context_bytes());
    let (first_b, _) = b.pending().unwrap();
    assert_eq!(first_a.request_id(), 1);
    assert_eq!(first_b.request_id(), 1);
    assert_ne!(first_a, first_b);
    assert_eq!(a.program_binding(), program);
    assert_eq!(a.input_binding(), packet.binding());
    assert_eq!(a.source().root(), source.root());
    let pointer = a.pending().unwrap().1.context_bytes().as_ptr();
    assert!(matches!(
        a.reply(first_b, &normalized(1, "", map(vec![]))),
        Err(RunError::Correlation)
    ));
    assert!(matches!(
        a.inference_error(first_b, "x"),
        Err(RunError::Correlation)
    ));
    assert!(matches!(
        a.reply(first_a, &normalized(2, "", map(vec![]))),
        Err(RunError::Correlation)
    ));
    assert!(matches!(a.reply(first_a, &[0xff]), Err(RunError::Reply(_))));
    assert!(matches!(
        a.inference_error(first_a, &"x".repeat(257)),
        Err(RunError::Frame)
    ));
    assert_eq!(a.pending().unwrap().1.context_bytes().as_ptr(), pointer);
    a.reply(first_a, &normalized(1, "patch_memory", map(vec![])))
        .unwrap();
    let (second_a, request) = a.pending().unwrap();
    assert_eq!(second_a.request_id(), 2);
    let Value::Array(old) = prefix else { panic!() };
    let Value::Array(new) = decode(request.context_bytes()) else {
        panic!()
    };
    assert_eq!(&old[..6], &new[..6]);
    assert_ne!(old[6], new[6]);
    assert!(matches!(
        a.reply(first_a, &normalized(1, "", map(vec![]))),
        Err(RunError::Correlation)
    ));
    assert!(matches!(
        a.reply(first_b, &normalized(1, "", map(vec![]))),
        Err(RunError::Correlation)
    ));
    a.reply(second_a, &normalized(2, "", map(vec![]))).unwrap();
    assert_eq!(a.phase(), Phase::Boundary);
    assert!(a.pending().is_none());
    let candidate = a.candidate().unwrap();
    assert_eq!(candidate.expected_root(), source.root());
    let root = candidate.new_root();
    assert_eq!(a.candidate_snapshot().unwrap().root(), root);
    let blocks: Vec<_> = candidate
        .new_blocks()
        .iter()
        .map(|b| (b.cid, b.body.to_vec()))
        .collect();
    assert!(matches!(
        a.reply(second_a, &normalized(2, "", map(vec![]))),
        Err(RunError::Phase)
    ));
    assert_eq!(a.candidate().unwrap().new_root(), root);
    assert_eq!(
        a.candidate()
            .unwrap()
            .new_blocks()
            .iter()
            .map(|b| (b.cid, b.body.to_vec()))
            .collect::<Vec<_>>(),
        blocks
    );
    // Native guest independently executes exactly the same frames; no host tool decisions.
    let mut native = guest::Guest::new();
    native
        .step(&activate(&genesis(1), &super::packet()))
        .unwrap();
    native
        .step(&progression::reply(
            1,
            1,
            vec![progression::call("patch_memory", map(vec![]))],
        ))
        .unwrap();
    let expected = native.step(&progression::reply(2, 2, vec![])).unwrap();
    let Output::Boundary(expected) = Output::decode(&expected, &Limits::default()).unwrap() else {
        panic!()
    };
    assert_eq!(expected.new_root(), root);
    assert_eq!(
        expected
            .new_blocks()
            .iter()
            .map(|b| (b.cid, b.body.to_vec()))
            .collect::<Vec<_>>(),
        blocks
    );
    // Cross-run refusal did not advance b; matching error terminates only that VM.
    assert!(matches!(
        b.inference_error(first_b, "SYNTHETIC_FAIL"),
        Err(RunError::GuestFailure)
    ));
    assert_eq!(b.phase(), Phase::Failed);
    assert!(b.pending().is_none());
    assert!(b.candidate().is_none());
    assert!(b.candidate_snapshot().is_none());
    assert!(matches!(
        b.output(),
        Some(Output::Error {
            code: "inference_failed"
        })
    ));
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_routing_yield_candidate_is_data_only_and_transport_exhaustion_has_no_candidate() {
    let wasm = wasm();
    let source = source();
    let packet = prepared(&source);
    let program = pin(&wasm, &source);
    let mut run = Run::start(&wasm, program, &source, &packet, VmLimits::default()).unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(
        ticket,
        &normalized(1, "trade", map(vec![("side", Value::Text("buy".into()))])),
    )
    .unwrap();
    assert_eq!(run.phase(), Phase::Boundary);
    let snapshot = run.candidate_snapshot().unwrap();
    assert_eq!(snapshot.root(), run.candidate().unwrap().new_root());
    let last = snapshot
        .current_episode()
        .unwrap()
        .boundaries()
        .last()
        .unwrap();
    let raiv::boundary::Exit::AwaitAction(command) = snapshot.boundary(last).unwrap().exit() else {
        panic!()
    };
    assert!(!command.is_empty());
    assert!(matches!(
        run.candidate().unwrap().disposition(),
        raiv::state::Continuation::AwaitAction { .. }
    ));
    assert!(matches!(
        run.inference_error(ticket, "x"),
        Err(RunError::Phase)
    ));
    // Too little fuel to activate fails explicitly, never yields a default Finish.
    assert!(matches!(
        Run::start(
            &wasm,
            program,
            &source,
            &packet,
            VmLimits {
                fuel_per_step: 1,
                ..VmLimits::default()
            }
        ),
        Err(RunError::Bridge(_))
    ));
}
