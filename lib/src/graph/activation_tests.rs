use super::*;
use crate::{
    activation::{
        prepare, ActivationError, ExecutionQuotes, InferenceError, InferencePhase, RequestSequence,
        RunPolicy,
    },
    definition::ToolControl,
};

pub(super) fn incoming(snapshot: &RestoredAgent) -> Cbor {
    let mut source = full_context::incoming(snapshot);
    let grant = map_mut(&mut source).get_mut("budget_admission").unwrap();
    map_mut(grant).insert("activation_units".into(), Cbor::Integer(10.into()));
    map_mut(grant).insert("remaining_episode_units".into(), Cbor::Integer(20.into()));
    source
}

pub(super) fn policy() -> RunPolicy {
    RunPolicy {
        caller: Limits::default(),
        quotes: ExecutionQuotes {
            model: 1,
            continue_tool: 1,
            yield_tool: 1,
            finish: 1,
        },
    }
}

#[test]
fn owned_prepare_captures_all_positive_snapshots_and_first_request_matches_independent_context_without_callbacks_or_repeated_debits(
) {
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let definition =
            Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
        let registered = preparation::registrations(&definition);
        let tools =
            crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
        let source = incoming(&snapshot);
        let packet = preparation::packet(&snapshot, &source);
        let input_ptr = packet.input().as_bytes().as_ptr();
        let before = snapshot
            .blocks()
            .map(|(cid, b)| (cid, b.as_ptr()))
            .collect::<BTreeMap<_, _>>();
        let activation = prepare(
            snapshot,
            packet,
            &tools,
            policy(),
            RequestSequence::new([93; 32]),
        )
        .unwrap_or_else(|error| panic!("prepare: {}", error.reason));
        let request = activation.request().unwrap();
        assert_eq!(request.binding().request_id, 1);
        assert_eq!(request.binding().instance_id, [93; 32]);
        assert_eq!(request.projection_version(), 1);
        assert_eq!(
            request.context_bytes(),
            encode(&full_context::oracle(
                activation.snapshot(),
                &source,
                &Cbor::Array(vec![])
            ))
        );
        assert_eq!(
            request.model_parameters_bytes(),
            definition.inference_profile_bytes()
        );
        assert_eq!(
            request.tools().collect::<Vec<_>>(),
            definition.tools().collect::<Vec<_>>()
        );
        let pointer = request.context_bytes().as_ptr();
        for _ in 0..3 {
            let view = activation.request().unwrap();
            assert_eq!(view.binding(), request.binding());
            assert_eq!(view.context_bytes().as_ptr(), pointer);
        }
        assert_eq!(activation.session().budget().used().model_calls, 1);
        assert_eq!(activation.units().reserved_units(), 1);
        assert_eq!(activation.trajectory_bytes(), &[0x80]);
        assert_eq!(activation.input().input().as_bytes().as_ptr(), input_ptr);
        assert_eq!(
            activation
                .snapshot()
                .blocks()
                .map(|(cid, b)| (cid, b.as_ptr()))
                .collect::<BTreeMap<_, _>>(),
            before
        );
        match (activation.snapshot().memory(), activation.working_memory()) {
            (None, None) => {}
            (Some(checkpoint), Some(working)) => {
                assert_eq!(checkpoint.as_bytes(), working.as_bytes());
                assert_ne!(checkpoint.as_bytes().as_ptr(), working.as_bytes().as_ptr());
            }
            _ => panic!("no fictitious memory"),
        }
    }
}

#[test]
fn no_tool_definition_finishes_with_empty_or_textual_complete_reply_without_corrective_or_local_work(
) {
    for text in ["", "FINAL_RAM_TEXT"] {
        let fixture = &fixtures()[0];
        let mut blocks = store(fixture);
        let root = root_of(fixture);
        let original = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let def = patch(&mut blocks, original.definition().cid(), |r| {
            map_mut(r).insert("tools".into(), Cbor::Array(vec![]));
        });
        let root = patch(&mut blocks, root, |r| {
            map_mut(r).insert("definition".into(), link(def));
        });
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let definition =
            Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
        let tools = crate::tools::ToolSet::bind(&definition, &[], &Limits::default()).unwrap();
        let input = incoming(&snapshot);
        let packet = preparation::packet(&snapshot, &input);
        let mut run = prepare(
            snapshot,
            packet,
            &tools,
            policy(),
            RequestSequence::new([93; 32]),
        )
        .unwrap_or_else(|e| panic!("{}", e.reason));
        assert_eq!(run.request().unwrap().tools().count(), 0);
        let key = run.request().unwrap().binding();
        let mut body = reply::frame(key.request_id, vec![]);
        map_mut(&mut body).insert("text".into(), Cbor::Text(text.into()));
        assert_eq!(
            run.advance_ram(key, &encode(&body)).unwrap(),
            crate::activation::RamProgress::Sealed
        );
        assert_eq!(
            run.ram_decision(),
            Some(&crate::activation::RamDecision::Finish)
        );
        assert_eq!(run.usage().model_calls, 1);
        assert_eq!(run.usage().local_tool_calls, 0);
        assert_eq!(run.session().budget().used().actions, 0);
        assert_eq!(run.session().budget().used().corrective_turns, 0);
        assert_eq!(run.units().reserved_units(), 2);
        let candidate = run.candidate().unwrap();
        assert_eq!(candidate.snapshot().state().revision(), 1);
        assert_eq!(
            candidate.snapshot().state().continuation(),
            crate::state::Continuation::AwaitActor
        );
        assert_eq!(candidate.snapshot().history().count(), 1);
        assert!(candidate
            .blocks()
            .all(|(_, bytes)| !bytes.windows(14).any(|b| b == b"FINAL_RAM_TEXT")));
        assert!(run.advance_ram(key, &encode(&body)).is_err());
    }
}

#[test]
fn model_request_enabled_descriptors_obey_counts_and_quotes_but_keep_immutable_declared_prefix_and_last_reply_finish(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let mut source = incoming(&snapshot);
    let grant = map_mut(&mut source).get_mut("budget_admission").unwrap();
    map_mut(grant).insert("activation_units".into(), Cbor::Integer(3.into()));
    map_mut(grant).insert("remaining_episode_units".into(), Cbor::Integer(5.into()));
    let packet = preparation::packet(&snapshot, &source);
    let selected = RunPolicy {
        caller: Limits {
            max_actions_per_episode: 0,
            max_local_tools_per_activation: 0,
            max_model_calls_per_activation: 1,
            ..Limits::default()
        },
        quotes: ExecutionQuotes {
            model: 1,
            continue_tool: 2,
            yield_tool: 3,
            finish: 1,
        },
    };
    let activation = prepare(
        snapshot,
        packet,
        &tools,
        selected,
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let request = activation.request().unwrap();
    assert_eq!(
        request.tools().collect::<Vec<_>>(),
        definition
            .tools()
            .filter(|tool| tool.control == ToolControl::Finish)
            .collect::<Vec<_>>()
    );
    assert_eq!(activation.session().budget().remaining().model_calls, 0);
    assert_eq!(activation.units().remaining_activation_units(), 2);
    assert_eq!(request.limits().max_model_calls_per_activation, 1);
    assert_eq!(
        request.limits().history_episodes,
        definition.limits().history_episodes
    );
    assert_eq!(
        request.context_bytes(),
        encode(&full_context::oracle(
            activation.snapshot(),
            &source,
            &Cbor::Array(vec![])
        ))
    );
}

#[test]
fn preparation_failures_preserve_the_unspent_instance_sequence_and_reject_inference_or_control_exhaustion_early(
) {
    let fixture = &roles()[0];
    let root = root_of(fixture);
    let blocks = store(fixture);
    let original = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(original.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = incoming(&original);
    let mut sequence = RequestSequence::new([93; 32]);
    for config in [
        RunPolicy {
            caller: Limits {
                max_model_calls_per_activation: 0,
                ..Limits::default()
            },
            ..policy()
        },
        RunPolicy {
            quotes: ExecutionQuotes {
                model: u64::MAX,
                ..policy().quotes
            },
            ..policy()
        },
        RunPolicy {
            quotes: ExecutionQuotes {
                continue_tool: u64::MAX,
                yield_tool: u64::MAX,
                finish: u64::MAX,
                ..policy().quotes
            },
            ..policy()
        },
        RunPolicy {
            caller: Limits {
                history_episodes: 0,
                ..Limits::default()
            },
            ..policy()
        },
    ] {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let packet = preparation::packet(&snapshot, &source);
        let failure = prepare(snapshot, packet, &tools, config, sequence)
            .err()
            .unwrap();
        assert_eq!(failure.sequence.last_id(), 0);
        assert_eq!(failure.sequence.instance_id(), [93; 32]);
        assert!(matches!(
            failure.reason,
            ActivationError::Inference(_)
                | ActivationError::NoEnabledControl
                | ActivationError::HistoryCapacity
                | ActivationError::Prepared(_)
        ));
        sequence = failure.sequence;
    }
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &source);
    let activation = prepare(snapshot, packet, &tools, policy(), sequence)
        .ok()
        .unwrap();
    assert_eq!(activation.request().unwrap().binding().request_id, 1);
}

#[test]
fn combined_context_limits_failure_happens_before_first_debit_and_exact_context_ceiling_succeeds() {
    let fixture = &roles()[0];
    let root = root_of(fixture);
    let blocks = store(fixture);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let activation = prepare(
        snapshot,
        packet,
        &tools,
        policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let exact = activation.request().unwrap().context_bytes().len() as u64;
    let prefix = activation.initial_prefix().len() as u64;
    assert!(exact > prefix);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &source);
    let config = RunPolicy {
        caller: Limits {
            max_context_bytes: prefix,
            ..Limits::default()
        },
        ..policy()
    };
    let failure = prepare(
        snapshot,
        packet,
        &tools,
        config,
        RequestSequence::new([93; 32]),
    )
    .err()
    .unwrap();
    assert_eq!(
        failure.reason,
        ActivationError::Projection(crate::projection::ProjectionError::ContextLimit)
    );
    assert_eq!(failure.sequence.last_id(), 0);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &source);
    let config = RunPolicy {
        caller: Limits {
            max_context_bytes: exact,
            ..Limits::default()
        },
        ..policy()
    };
    let activation = prepare(snapshot, packet, &tools, config, failure.sequence)
        .ok()
        .unwrap();
    assert_eq!(
        activation.request().unwrap().context_bytes().len() as u64,
        exact
    );
    assert_eq!(activation.units().reserved_units(), 1);
}

#[test]
fn wrong_or_matching_inference_failures_preserve_or_close_the_owned_request_without_checkpoint_or_nonce_reset(
) {
    let fixture = &roles()[0];
    let root = root_of(fixture);
    let blocks = store(fixture);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut activation = prepare(
        snapshot,
        packet,
        &tools,
        policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let key = activation.request().unwrap().binding();
    let before = activation.initial_prefix().to_vec();
    assert_eq!(
        activation.inference_error(crate::activation::RequestBinding {
            request_id: 0,
            ..key
        }),
        Err(ActivationError::Inference(InferenceError::WrongRequest))
    );
    assert_eq!(activation.request().unwrap().binding(), key);
    assert!(activation.release_sequence().is_err());
    activation.inference_error(key).unwrap();
    assert_eq!(activation.session().phase(), InferencePhase::Failed);
    assert!(activation.request().is_err());
    assert_eq!(activation.snapshot().root(), root);
    assert_eq!(activation.initial_prefix(), before);
    assert_eq!(activation.units().reserved_units(), 1);
    let sequence = activation.release_sequence().unwrap();
    assert_eq!(sequence.last_id(), 1);
    assert!(activation.release_sequence().is_err());
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &source);
    let next = prepare(snapshot, packet, &tools, policy(), sequence)
        .ok()
        .unwrap();
    assert_eq!(next.request().unwrap().binding().request_id, 2); // Recovery authority/lost-attempt accounting remains a host prerequisite.
}

#[test]
fn zero_caller_history_can_materialize_genesis_but_not_prepare_an_unretainable_next_completed_window(
) {
    let fixture = &fixtures()[0];
    let blocks = store(fixture);
    let root = root_of(fixture);
    let snapshot = RestoredAgent::restore(
        root,
        &blocks,
        &Limits {
            history_episodes: 0,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(snapshot.state().revision(), 0);
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let failure = prepare(
        snapshot,
        packet,
        &tools,
        RunPolicy {
            caller: Limits {
                history_episodes: 0,
                ..Limits::default()
            },
            ..policy()
        },
        RequestSequence::new([93; 32]),
    )
    .err()
    .unwrap();
    assert_eq!(failure.reason, ActivationError::HistoryCapacity);
    assert_eq!(failure.sequence.last_id(), 0);
}

#[test]
fn full_width_last_revision_is_valid_data_but_cannot_prepare_a_runnable_boundary_increment() {
    let (mut blocks, root) = super::super::super::history::completed_pair();
    let root = patch(&mut blocks, root, |record| {
        map_mut(record).insert("revision".into(), Cbor::Integer(u64::MAX.into()));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = crate::tools::ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let failure = prepare(
        snapshot,
        packet,
        &tools,
        policy(),
        RequestSequence::new([93; 32]),
    )
    .err()
    .unwrap();
    assert_eq!(failure.reason, ActivationError::RevisionOverflow);
    assert_eq!(failure.sequence.last_id(), 0);
}
