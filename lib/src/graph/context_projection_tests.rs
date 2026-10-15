use super::*;
use crate::{
    format::{oracle::fixture_value, ModelValue},
    input::ActivationInput,
    projection::{InitialProjection, ProjectionError},
};

pub(super) fn incoming(snapshot: &RestoredAgent) -> Cbor {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../fixtures/format/input.json")).unwrap();
    let mut input = fixture_value(&fixtures[0]["record"]);
    map_mut(&mut input).insert("event_id".into(), Cbor::Bytes(vec![71; 32]));
    if let Some(episode) = snapshot.current_episode() {
        let boundary = snapshot
            .boundary(episode.boundaries().last().unwrap())
            .unwrap();
        let Exit::AwaitAction(bytes) = boundary.exit() else {
            panic!("pending")
        };
        let command = cbor(bytes);
        let fixtures: Value =
            serde_json::from_str(include_str!("../../../fixtures/format/terminal.json")).unwrap();
        let mut receipt = fixture_value(&fixtures["receipts"][0]["record"]);
        map_mut(&mut receipt).insert("action_id".into(), field(&command, "action_id").clone());
        let digest = Sha256::digest(encode(&Cbor::Array(vec![
            Cbor::Text("raiv/command/1".into()),
            Cbor::Bytes(snapshot.definition().execution_domain().to_vec()),
            command,
        ])))
        .to_vec();
        map_mut(&mut receipt).insert("command_digest".into(), Cbor::Bytes(digest));
        map_mut(&mut input).insert("receipt".into(), receipt);
        map_mut(&mut input).insert("cause".into(), Cbor::Text("action_result".into()));
    }
    input
}

pub(super) fn input(value: &Cbor) -> ActivationInput {
    ActivationInput::decode(&encode(value), &Limits::default()).unwrap()
}

fn trajectory(value: Cbor) -> ModelValue {
    ModelValue::decode(
        &encode(&value),
        &DecodeLimits {
            max_block_bytes: 131072,
            ..DecodeLimits::default()
        },
    )
    .unwrap()
}

fn empty() -> ModelValue {
    trajectory(Cbor::Array(vec![]))
}

fn position(value: &Cbor) -> Cbor {
    let mut result = value.clone();
    map_mut(&mut result).insert("domain".into(), encoding::text_id(field(value, "domain")));
    map_mut(&mut result).insert("number".into(), encoding::decimal(field(value, "number")));
    if !matches!(field(value, "reference"), Cbor::Null) {
        map_mut(&mut result).insert(
            "reference".into(),
            encoding::text_id(field(value, "reference")),
        );
    }
    result
}

fn prefix(snapshot: &RestoredAgent, input: &Cbor) -> Vec<Cbor> {
    let definition = cbor(snapshot.definition().as_bytes());
    let Cbor::Array(tools) = field(&definition, "tools") else {
        panic!("tools")
    };
    let tools = tools
        .iter()
        .map(|value| {
            let mut result = value.clone();
            map_mut(&mut result)
                .insert("version".into(), encoding::decimal(field(value, "version")));
            if !matches!(field(value, "authority_binding"), Cbor::Null) {
                map_mut(&mut result).insert(
                    "authority_binding".into(),
                    encoding::text_id(field(value, "authority_binding")),
                );
            }
            result
        })
        .collect();
    let mut grant = field(input, "budget_admission").clone();
    map_mut(&mut grant).insert(
        "admission_id".into(),
        encoding::text_id(field(field(input, "budget_admission"), "admission_id")),
    );
    for name in [
        "activation_units",
        "remaining_episode_units",
        "remaining_model_calls",
        "remaining_actions",
    ] {
        map_mut(&mut grant).insert(
            name.into(),
            encoding::decimal(field(field(input, "budget_admission"), name)),
        );
    }
    let receipt = if matches!(field(input, "receipt"), Cbor::Null) {
        Cbor::Null
    } else {
        encoding::normalized_receipt(field(input, "receipt"))
    };
    let mut sections = vec![
        Cbor::Map(vec![
            (Cbor::Text("kind".into()), Cbor::Text("policy".into())),
            (
                Cbor::Text("policy".into()),
                field(&definition, "policy").clone(),
            ),
            (
                Cbor::Text("mandate".into()),
                field(&definition, "mandate").clone(),
            ),
        ]),
        Cbor::Map(vec![
            (Cbor::Text("kind".into()), Cbor::Text("tools".into())),
            (Cbor::Text("tools".into()), Cbor::Array(tools)),
        ]),
        Cbor::Map(vec![
            (
                Cbor::Text("kind".into()),
                Cbor::Text("machine_input".into()),
            ),
            (Cbor::Text("cause".into()), field(input, "cause").clone()),
            (
                Cbor::Text("event_id".into()),
                encoding::text_id(field(input, "event_id")),
            ),
            (Cbor::Text("receipt".into()), receipt),
            (
                Cbor::Text("observation".into()),
                field(input, "observation").clone(),
            ),
            (
                Cbor::Text("observed_at".into()),
                position(field(input, "observed_at")),
            ),
            (Cbor::Text("budget_admission".into()), grant),
        ]),
    ];
    let Cbor::Array(history) = encoding::oracle(snapshot) else {
        panic!("history")
    };
    sections.extend(history);
    let memory = snapshot
        .memory()
        .map(|value| cbor(value.value_bytes()))
        .unwrap_or(Cbor::Null);
    sections.push(Cbor::Map(vec![
        (Cbor::Text("kind".into()), Cbor::Text("memory".into())),
        (Cbor::Text("memory".into()), memory),
    ]));
    sections
}

pub(super) fn oracle(snapshot: &RestoredAgent, input: &Cbor, trajectory: &Cbor) -> Cbor {
    let mut sections = prefix(snapshot, input);
    sections.push(Cbor::Map(vec![
        (
            Cbor::Text("kind".into()),
            Cbor::Text("ram_trajectory".into()),
        ),
        (Cbor::Text("entries".into()), trajectory.clone()),
    ]));
    Cbor::Array(sections)
}

#[test]
fn complete_context_and_immutable_prefix_match_independent_ciborium_for_all_phases_profiles_and_sections(
) {
    let mut snapshots = restored::valid_snapshots();
    snapshots.push(super::super::super::history::completed_pair());
    for (blocks, root) in snapshots {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let source = incoming(&snapshot);
        let input = input(&source);
        let captured = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
        assert_eq!(
            captured.as_bytes(),
            encode(&Cbor::Array(prefix(&snapshot, &source)))
        );
        let ram = Cbor::Array(vec![Cbor::Map(vec![(
            Cbor::Text("text".into()),
            Cbor::Text("ephemeral".into()),
        )])]);
        assert_eq!(
            captured
                .context(&trajectory(ram.clone()))
                .unwrap()
                .as_bytes(),
            encode(&oracle(&snapshot, &source, &ram))
        );
    }
}

#[test]
fn all_messages_declared_descriptors_and_trajectory_count_in_one_context_limit_without_truncation()
{
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let input = input(&source);
    let captured = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
    let complete = captured.context(&empty()).unwrap();
    let size = complete.as_bytes().len() as u64;
    let exact = InitialProjection::build(
        &snapshot,
        &input,
        &Limits {
            max_context_bytes: size,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(exact.context(&empty()).unwrap(), complete);
    let small = InitialProjection::build(
        &snapshot,
        &input,
        &Limits {
            max_context_bytes: size - 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(small.context(&empty()), Err(ProjectionError::ContextLimit));
    let history = HistoryProjection::new(&snapshot)
        .encode(&Limits::default())
        .unwrap();
    assert!(history.as_bytes().len() < captured.as_bytes().len());
    assert_eq!(
        InitialProjection::build(
            &snapshot,
            &input,
            &Limits {
                max_context_bytes: history.as_bytes().len() as u64,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::ContextLimit)
    );
    let large = trajectory(Cbor::Array(vec![Cbor::Text("x".repeat(2048))]));
    assert_eq!(exact.context(&large), Err(ProjectionError::ContextLimit));
    let before = captured.as_bytes().to_vec();
    assert_eq!(captured.context(&empty()).unwrap(), complete);
    assert_eq!(captured.as_bytes(), before);
}

#[test]
fn learned_definition_context_and_input_wire_caps_cannot_be_widened_by_caller_data() {
    let fixture = &roles()[0];
    let mut blocks = store(fixture);
    let root = root_of(fixture);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let input = input(&source);
    let size = InitialProjection::build(&snapshot, &input, &Limits::default())
        .unwrap()
        .context(&empty())
        .unwrap()
        .as_bytes()
        .len() as u64;
    let root = definition_change(&mut blocks, root, |r| {
        map_mut(map_mut(r).get_mut("limits").unwrap())
            .insert("max_context_bytes".into(), Cbor::Integer((size - 1).into()));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let captured = InitialProjection::build(
        &snapshot,
        &input,
        &Limits {
            max_context_bytes: u64::MAX,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(
        captured.context(&empty()),
        Err(ProjectionError::ContextLimit)
    );
    assert!(matches!(
        InitialProjection::build(
            &snapshot,
            &input,
            &Limits {
                max_observation_bytes: 0,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::Input(_))
    ));
    assert!(matches!(
        InitialProjection::build(
            &snapshot,
            &input,
            &Limits {
                max_block_bytes: 0,
                ..Limits::default()
            }
        ),
        Err(ProjectionError::Input(_))
    ));
}

#[test]
fn wrong_phase_wrong_action_or_digest_reject_before_capturing_an_initial_context() {
    let mut actor = None;
    let mut pending = None;
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        if snapshot.current_episode().is_some() {
            pending = Some(snapshot);
        } else {
            actor = Some(snapshot);
        }
    }
    let actor = actor.unwrap();
    let pending = pending.unwrap();
    let wake = input(&incoming(&actor));
    let result = input(&incoming(&pending));
    assert_eq!(
        InitialProjection::build(&pending, &wake, &Limits::default()),
        Err(ProjectionError::WrongPhase)
    );
    assert_eq!(
        InitialProjection::build(&actor, &result, &Limits::default()),
        Err(ProjectionError::WrongPhase)
    );
    for name in ["action_id", "command_digest"] {
        let mut bad = incoming(&pending);
        map_mut(map_mut(&mut bad).get_mut("receipt").unwrap())
            .insert(name.into(), Cbor::Bytes(vec![99; 32]));
        assert_eq!(
            InitialProjection::build(&pending, &input(&bad), &Limits::default()),
            Err(ProjectionError::ReceiptMismatch)
        );
    }
}

#[test]
fn runtime_widths_nulls_and_machine_source_labels_normalize_without_loss_or_fake_memory() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = incoming(&snapshot);
    map_mut(map_mut(&mut source).get_mut("observed_at").unwrap())
        .insert("number".into(), Cbor::Integer(u64::MAX.into()));
    map_mut(map_mut(&mut source).get_mut("observed_at").unwrap())
        .insert("reference".into(), Cbor::Bytes(vec![44; 32]));
    for name in [
        "activation_units",
        "remaining_episode_units",
        "remaining_model_calls",
        "remaining_actions",
    ] {
        map_mut(map_mut(&mut source).get_mut("budget_admission").unwrap())
            .insert(name.into(), Cbor::Integer(u64::MAX.into()));
    }
    let input = input(&source);
    let projection = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
    let result = projection.context(&empty()).unwrap();
    assert_eq!(
        result.as_bytes(),
        encode(&oracle(&snapshot, &source, &Cbor::Array(vec![])))
    );
    let Cbor::Array(sections) = cbor(result.as_bytes()) else {
        panic!("sections")
    };
    assert_eq!(
        field(&sections[2], "kind"),
        &Cbor::Text("machine_input".into())
    );
    assert_eq!(field(&sections[5], "memory"), &Cbor::Null);
}

#[test]
fn trajectory_shape_count_and_contextual_depth_fail_without_rewriting_prefix_or_memory() {
    let fixture = &roles()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let input = input(&source);
    let projection = InitialProjection::build(
        &snapshot,
        &input,
        &Limits {
            max_collection_entries: 8,
            ..Limits::default()
        },
    )
    .unwrap();
    let before = projection.as_bytes().to_vec();
    assert_eq!(
        projection.context(&trajectory(Cbor::Map(vec![]))),
        Err(ProjectionError::TrajectoryNotList)
    );
    assert_eq!(
        projection.context(&trajectory(Cbor::Array(vec![Cbor::Null; 9]))),
        Err(ProjectionError::Codec(CodecError::CollectionLimit))
    );
    let shallow = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
    let mut nested = Cbor::Null;
    for _ in 0..30 {
        nested = Cbor::Array(vec![nested]);
    }
    assert_eq!(
        shallow.context(&trajectory(Cbor::Array(vec![nested]))),
        Err(ProjectionError::Codec(CodecError::DepthLimit))
    );
    assert_eq!(projection.as_bytes(), before);
    assert!(projection.context(&empty()).is_ok());
}

#[test]
fn sparse_restore_and_projection_preserve_absent_planes_and_distinguish_disabled_memory() {
    for value in [
        serde_json::json!({}),
        serde_json::json!({"intents":{"x":1}}),
        serde_json::json!({"strategy":{"x":1}}),
        serde_json::json!({"tactics":{"x":1}}),
    ] {
        let fixture = &fixtures()[1];
        let mut blocks = store(fixture);
        let memory = Memory::decode(
            &crate::format::oracle::json_bytes(
                &serde_json::json!({"kind":"memory","version":1,"value":value}),
            ),
            &DecodeLimits::default(),
        )
        .unwrap();
        let cid = memory.cid();
        blocks.0.insert(cid, memory.as_bytes().to_vec());
        let root = patch(&mut blocks, root_of(fixture), |r| {
            map_mut(r).insert("memory".into(), link(cid));
        });
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        assert_eq!(
            snapshot.memory().unwrap().value_bytes(),
            crate::format::oracle::json_bytes(&value)
        );
        let source = incoming(&snapshot);
        let input = input(&source);
        let projection = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
        let Cbor::Array(sections) = cbor(projection.as_bytes()) else {
            panic!()
        };
        assert_eq!(
            field(sections.last().unwrap(), "memory"),
            &cbor(memory.value_bytes())
        );
    }
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    assert!(snapshot.memory().is_none());
    let source = incoming(&snapshot);
    let input = input(&source);
    let projection = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
    let Cbor::Array(sections) = cbor(projection.as_bytes()) else {
        panic!()
    };
    assert_eq!(field(sections.last().unwrap(), "memory"), &Cbor::Null);
}

#[test]
fn independent_working_patches_only_append_data_and_never_rewrite_the_captured_initial_memory() {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = incoming(&snapshot);
    let input = input(&source);
    let projection = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
    let initial = projection.as_bytes().to_vec();
    let mut working = Memory::decode(
        snapshot.memory().unwrap().as_bytes(),
        &DecodeLimits::default(),
    )
    .unwrap();
    working
        .apply_patch(
            &crate::format::oracle::json_bytes(
                &serde_json::json!({"intents":{"new_note":"accepted"}}),
            ),
            snapshot.definition().limits(),
        )
        .unwrap();
    let ram = Cbor::Array(vec![Cbor::Map(vec![(
        Cbor::Text("memory".into()),
        cbor(working.value_bytes()),
    )])]);
    assert_eq!(
        projection
            .context(&trajectory(ram.clone()))
            .unwrap()
            .as_bytes(),
        encode(&oracle(&snapshot, &source, &ram))
    );
    assert_eq!(projection.as_bytes(), initial);
    assert_ne!(working.cid(), snapshot.memory().unwrap().cid());
}
