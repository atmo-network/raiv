use super::*;
use crate::{
    input::ActivationInput,
    preparation::{InputBinding, InputCompatibilityError, PrepareError, PreparedInput},
    tools::{PlanningContext, ToolFailure, ToolImplementation, ToolRegistration, ToolSet},
};
use core::sync::atomic::{AtomicUsize, Ordering};

static CALLBACKS: AtomicUsize = AtomicUsize::new(0);
fn validate(_: &[u8]) -> Result<(), ToolFailure> {
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
    Err(ToolFailure::InvalidArguments)
}
fn plan(_: PlanningContext<'_>, _: &[u8]) -> Result<crate::format::ModelValue, ToolFailure> {
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
    Err(ToolFailure::PlanningFailed)
}
pub(super) fn registrations(definition: &Definition) -> Vec<ToolRegistration<'_>> {
    definition
        .tools()
        .map(|descriptor| ToolRegistration {
            descriptor,
            implementation: match descriptor.control {
                crate::definition::ToolControl::Continue => {
                    ToolImplementation::Pure { validate, plan }
                }
                crate::definition::ToolControl::Yield => {
                    ToolImplementation::Yield { validate, plan }
                }
                crate::definition::ToolControl::Finish => {
                    panic!("Finish is intrinsic, not a registered tool")
                }
            },
        })
        .collect()
}
fn binding(snapshot: &RestoredAgent, source: &Cbor) -> InputBinding {
    let Cbor::Bytes(event) = field(source, "event_id") else {
        panic!("event")
    };
    InputBinding {
        agent_id: snapshot.state().agent_id(),
        execution_domain: snapshot.definition().execution_domain(),
        expected_root: snapshot.root(),
        expected_revision: snapshot.state().revision(),
        admitted_event_id: event.as_slice().try_into().unwrap(),
        admitted_input_digest: Sha256::digest(encode(source)).into(),
    }
}
pub(super) fn packet(snapshot: &RestoredAgent, source: &Cbor) -> PreparedInput {
    PreparedInput::new(binding(snapshot, source), full_context::input(source)).unwrap()
}

#[test]
fn prepared_packets_bind_every_positive_snapshot_without_copying_payloads_or_invoking_tools() {
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let source = full_context::incoming(&snapshot);
        let owned = full_context::input(&source);
        let pointer = owned.as_bytes().as_ptr();
        let expected = binding(&snapshot, &source);
        let packet = PreparedInput::new(expected, owned).unwrap();
        assert_eq!(packet.input().as_bytes().as_ptr(), pointer);
        let registered = registrations(snapshot.definition());
        let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
        let before = snapshot
            .blocks()
            .map(|(cid, b)| (cid, b.as_ptr()))
            .collect::<BTreeMap<_, _>>();
        let checked = packet
            .validate(&snapshot, &tools, &Limits::default())
            .unwrap();
        assert_eq!(checked.binding(), expected);
        assert_eq!(checked.input().as_bytes().as_ptr(), pointer);
        assert_eq!(checked.snapshot().root(), root);
        assert_eq!(checked.caller_limits(), &Limits::default());
        assert_eq!(
            snapshot
                .blocks()
                .map(|(cid, b)| (cid, b.as_ptr()))
                .collect::<BTreeMap<_, _>>(),
            before
        );
    }
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn constructor_rejects_wrong_event_digest_and_raw_root_without_a_partial_packet() {
    let (blocks, root) = restored::valid_snapshots().remove(0);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let good = binding(&snapshot, &source);
    let cases = [
        (
            InputBinding {
                admitted_event_id: [99; 32],
                ..good
            },
            PrepareError::EventBindingMismatch,
        ),
        (
            InputBinding {
                admitted_input_digest: [99; 32],
                ..good
            },
            PrepareError::InputDigestMismatch,
        ),
        (
            InputBinding {
                expected_root: DaslCid::for_block(DaslCodec::Raw, b"x"),
                ..good
            },
            PrepareError::InvalidRootCodec,
        ),
    ];
    for (bad, error) in cases {
        assert_eq!(
            PreparedInput::new(bad, full_context::input(&source)).err(),
            Some(error)
        );
    }
}

#[test]
fn agent_domain_root_revision_and_toolset_definition_are_separate_exact_bindings() {
    let (blocks, root) = restored::valid_snapshots().remove(0);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let good = binding(&snapshot, &source);
    let registered = registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let cases = [
        (
            InputBinding {
                agent_id: [99; 32],
                ..good
            },
            PrepareError::WrongAgent,
        ),
        (
            InputBinding {
                execution_domain: [99; 32],
                ..good
            },
            PrepareError::WrongDomain,
        ),
        (
            InputBinding {
                expected_root: DaslCid::for_block(DaslCodec::Drisl, &[0xa0]),
                ..good
            },
            PrepareError::WrongRoot,
        ),
        (
            InputBinding {
                expected_revision: good.expected_revision + 1,
                ..good
            },
            PrepareError::WrongRevision,
        ),
    ];
    for (bad, error) in cases {
        let packet = PreparedInput::new(bad, full_context::input(&source)).unwrap();
        assert_eq!(
            packet.validate(&snapshot, &tools, &Limits::default()).err(),
            Some(error)
        );
    }
    let mut changed = cbor(snapshot.definition().as_bytes());
    map_mut(&mut changed).insert("policy".into(), Cbor::Text("other policy".into()));
    let changed = Definition::decode(&encode(&changed), &DecodeLimits::default()).unwrap();
    let registered = registrations(&changed);
    let tools = ToolSet::bind(&changed, &registered, &Limits::default()).unwrap();
    assert_eq!(
        packet(&snapshot, &source)
            .validate(&snapshot, &tools, &Limits::default())
            .err(),
        Some(PrepareError::WrongDefinition)
    );
}

#[test]
fn shared_phase_and_receipt_checks_preserve_projection_errors_and_reject_known_retained_events() {
    let fixture = &identity_graphs()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let registered = registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let actor_fixture = &roles()[0];
    let actor = RestoredAgent::restore(
        root_of(actor_fixture),
        &store(actor_fixture),
        &Limits::default(),
    )
    .unwrap();
    let wake = full_context::incoming(&actor);
    assert_eq!(
        packet(&snapshot, &wake)
            .validate(&snapshot, &tools, &Limits::default())
            .err(),
        Some(PrepareError::Compatibility(
            InputCompatibilityError::WrongPhase
        ))
    );
    for name in ["action_id", "command_digest"] {
        let mut bad = full_context::incoming(&snapshot);
        map_mut(map_mut(&mut bad).get_mut("receipt").unwrap())
            .insert(name.into(), Cbor::Bytes(vec![99; 32]));
        assert_eq!(
            packet(&snapshot, &bad)
                .validate(&snapshot, &tools, &Limits::default())
                .err(),
            Some(PrepareError::Compatibility(
                InputCompatibilityError::ReceiptMismatch
            ))
        );
    }
    let known = snapshot
        .blocks()
        .find_map(|(cid, _)| snapshot.input(cid).map(ActivationInput::event_id))
        .unwrap();
    let mut replay = full_context::incoming(&snapshot);
    map_mut(&mut replay).insert("event_id".into(), Cbor::Bytes(known.to_vec()));
    assert_eq!(
        packet(&snapshot, &replay)
            .validate(&snapshot, &tools, &Limits::default())
            .err(),
        Some(PrepareError::KnownEventReplay)
    );
}

#[test]
fn caller_aggregate_wire_and_nested_caps_are_rechecked_without_shrinking_definition_history() {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let packet = packet(&snapshot, &source);
    let registered = registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    assert_eq!(
        packet
            .validate(
                &snapshot,
                &tools,
                &Limits {
                    max_blocks: snapshot.block_count() - 1,
                    ..Limits::default()
                }
            )
            .err(),
        Some(PrepareError::Graph(GraphError::BlockCountLimit))
    );
    assert_eq!(
        packet
            .validate(
                &snapshot,
                &tools,
                &Limits {
                    max_materialized_bytes: snapshot.encoded_bytes() - 1,
                    ..Limits::default()
                }
            )
            .err(),
        Some(PrepareError::Graph(GraphError::MaterializedByteLimit))
    );
    assert_eq!(
        packet
            .validate(
                &snapshot,
                &tools,
                &Limits {
                    history_episodes: 0,
                    ..Limits::default()
                }
            )
            .err(),
        Some(PrepareError::Graph(
            HistoryConsistencyError::CallerWindowLimit(root).into()
        ))
    );
    for limits in [
        Limits {
            max_block_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_depth: 1,
            ..Limits::default()
        },
        Limits {
            max_collection_entries: 17,
            ..Limits::default()
        },
        Limits {
            max_memory_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_command_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_observation_bytes: 0,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            packet.validate(&snapshot, &tools, &limits),
            Err(PrepareError::Graph(_))
        ));
    }
    assert!(matches!(
        packet.validate(
            &snapshot,
            &tools,
            &Limits {
                max_receipt_bytes: 0,
                ..Limits::default()
            }
        ),
        Err(PrepareError::Compatibility(InputCompatibilityError::Input(
            _
        )))
    ));
    assert_eq!(
        snapshot.definition().limits().history_episodes,
        Limits::default().history_episodes
    );
    assert_eq!(snapshot.state().history_count(), 1);
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn full_u64_revision_and_captured_execution_limits_do_not_narrow_or_claim_budget_permission() {
    let (mut blocks, root) = super::super::super::history::completed_pair();
    let root = patch(&mut blocks, root, |r| {
        map_mut(r).insert("revision".into(), Cbor::Integer(u64::MAX.into()));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let packet = packet(&snapshot, &source);
    let registered = registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let caller = Limits {
        max_model_calls_per_activation: 0,
        max_actions_per_episode: 0,
        max_local_tools_per_activation: 0,
        ..Limits::default()
    };
    let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
    assert_eq!(checked.binding().expected_revision, u64::MAX);
    assert_eq!(checked.caller_limits(), &caller);
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0); // Checked data does not grant an inference/model/tool call.
}

#[test]
fn uncommitted_packet_recovery_is_deterministic_without_inventing_a_global_freshness_ledger() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let packet = packet(&snapshot, &source);
    let registered = registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            packet
                .validate(
                    &snapshot,
                    &tools,
                    &Limits {
                        history_episodes: 0,
                        ..Limits::default()
                    }
                )
                .unwrap()
                .binding(),
            packet.binding()
        );
    }
    assert_eq!(snapshot.state().history_count(), 0);
    assert_eq!(snapshot.definition().limits().history_episodes, 2);
}
