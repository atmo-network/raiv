use super::*;
use crate::{
    activation::{CallBudget, CallBudgetError, CallCounts, EpisodeCounts},
    definition::{Definition, ToolControl},
    tools::{ToolSet, ToolSetError},
};

fn number(value: &Cbor) -> u64 {
    let Cbor::Integer(value) = value else {
        panic!("unsigned")
    };
    (*value).try_into().unwrap()
}
fn accepted(snapshot: &RestoredAgent) -> EpisodeCounts {
    let mut result = EpisodeCounts {
        model_calls: 0,
        actions: 0,
    };
    if let Some(episode) = snapshot.current_episode() {
        for cid in episode.boundaries() {
            let boundary = cbor(snapshot.boundary(cid).unwrap().as_bytes());
            result.model_calls += number(field(field(&boundary, "usage"), "model_calls"));
            if field(field(&boundary, "exit"), "tag") == &Cbor::Text("await_action".into()) {
                result.actions += 1;
            }
        }
    }
    result
}
fn grant(source: &mut Cbor, models: u64, actions: u64) {
    let admission = map_mut(source).get_mut("budget_admission").unwrap();
    map_mut(admission).insert("remaining_model_calls".into(), Cbor::Integer(models.into()));
    map_mut(admission).insert("remaining_actions".into(), Cbor::Integer(actions.into()));
}

#[test]
fn current_episode_counts_survive_action_results_but_completed_history_never_reduces_new_wake_capacity(
) {
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let source = full_context::incoming(&snapshot);
        let packet = preparation::packet(&snapshot, &source);
        let registered = preparation::registrations(snapshot.definition());
        let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
        let checked = packet
            .validate(&snapshot, &tools, &Limits::default())
            .unwrap();
        let budget = CallBudget::from_input(&checked).unwrap();
        let expected = accepted(&snapshot);
        assert_eq!(budget.accepted_episode_counts(), expected);
        assert_eq!(budget.episode_counts(), expected);
        assert_eq!(budget.binding(), packet.binding());
        let policy = snapshot.definition().limits();
        let admission = field(&source, "budget_admission");
        assert_eq!(
            budget.capacity(),
            CallCounts {
                model_calls: policy
                    .max_model_calls_per_activation
                    .min(policy.max_model_calls_per_episode - expected.model_calls)
                    .min(number(field(admission, "remaining_model_calls"))),
                actions: (policy.max_actions_per_episode - expected.actions)
                    .min(number(field(admission, "remaining_actions"))),
                local_tool_calls: policy.max_local_tools_per_activation,
                corrective_turns: policy.max_corrective_turns
            }
        );
        assert_eq!(
            budget.used(),
            CallCounts {
                model_calls: 0,
                actions: 0,
                local_tool_calls: 0,
                corrective_turns: 0
            }
        );
        if snapshot.current_episode().is_none() {
            assert_eq!(
                expected,
                EpisodeCounts {
                    model_calls: 0,
                    actions: 0
                }
            );
        }
    }
}

#[test]
fn caller_and_input_grants_only_tighten_remaining_capacity_and_cannot_reset_accepted_counters() {
    let fixture = &identity_graphs()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let expected = accepted(&snapshot);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    for (models, actions, caller, remaining_models, remaining_actions) in [
        (
            10,
            10,
            Limits {
                max_model_calls_per_activation: 3,
                max_model_calls_per_episode: expected.model_calls + 2,
                max_actions_per_episode: expected.actions + 1,
                max_local_tools_per_activation: 2,
                max_corrective_turns: 1,
                ..Limits::default()
            },
            2,
            1,
        ),
        (1, 0, Limits::default(), 1, 0),
        (
            u64::MAX,
            u64::MAX,
            Limits {
                max_model_calls_per_episode: expected.model_calls.saturating_sub(1),
                max_actions_per_episode: expected.actions.saturating_sub(1),
                ..Limits::default()
            },
            0,
            0,
        ),
    ] {
        let mut source = full_context::incoming(&snapshot);
        grant(&mut source, models, actions);
        let packet = preparation::packet(&snapshot, &source);
        let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
        let budget = CallBudget::from_input(&checked).unwrap();
        assert_eq!(budget.remaining().model_calls, remaining_models);
        assert_eq!(budget.remaining().actions, remaining_actions);
        assert_eq!(budget.accepted_episode_counts(), expected);
        assert_eq!(
            budget.capacity().local_tool_calls,
            caller.max_local_tools_per_activation.min(
                snapshot
                    .definition()
                    .limits()
                    .max_local_tools_per_activation
            )
        );
        assert_eq!(
            budget.capacity().corrective_turns,
            caller
                .max_corrective_turns
                .min(snapshot.definition().limits().max_corrective_turns)
        );
    }
}

#[test]
fn debits_are_explicit_independent_atomic_on_refusal_and_never_synthesize_finish() {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    grant(&mut source, 1, 1);
    let packet = preparation::packet(&snapshot, &source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let caller = Limits {
        max_local_tools_per_activation: 1,
        max_corrective_turns: 1,
        ..Limits::default()
    };
    let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
    let mut budget = CallBudget::from_input(&checked).unwrap();
    let before = snapshot
        .blocks()
        .map(|(cid, b)| (cid, b.as_ptr()))
        .collect::<BTreeMap<_, _>>();
    budget.debit_model_call().unwrap();
    budget.debit_control(ToolControl::Continue).unwrap();
    budget.debit_control(ToolControl::Yield).unwrap();
    budget.debit_corrective_turn().unwrap();
    let used = budget.used();
    assert_eq!(
        used,
        CallCounts {
            model_calls: 1,
            local_tool_calls: 1,
            actions: 1,
            corrective_turns: 1
        }
    );
    assert_eq!(
        budget.debit_model_call(),
        Err(CallBudgetError::ModelCallsExhausted)
    );
    assert_eq!(
        budget.debit_control(ToolControl::Continue),
        Err(CallBudgetError::LocalToolsExhausted)
    );
    assert_eq!(
        budget.debit_control(ToolControl::Yield),
        Err(CallBudgetError::ActionsExhausted)
    );
    assert_eq!(
        budget.debit_corrective_turn(),
        Err(CallBudgetError::CorrectiveTurnsExhausted)
    );
    assert_eq!(budget.used(), used);
    budget.debit_control(ToolControl::Finish).unwrap();
    assert_eq!(budget.used(), used);
    assert_eq!(
        budget.episode_counts(),
        EpisodeCounts {
            model_calls: accepted(&snapshot).model_calls + 1,
            actions: accepted(&snapshot).actions + 1
        }
    );
    assert_eq!(
        snapshot
            .blocks()
            .map(|(cid, b)| (cid, b.as_ptr()))
            .collect::<BTreeMap<_, _>>(),
        before
    );
}

#[test]
fn count_enabled_registered_views_keep_order_and_finish_when_yield_or_continue_counts_run_out() {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    grant(&mut source, 1, 0);
    let packet = preparation::packet(&snapshot, &source);
    let mut registered = preparation::registrations(snapshot.definition());
    registered.reverse();
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let caller = Limits {
        max_local_tools_per_activation: 1,
        ..Limits::default()
    };
    let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
    let mut budget = CallBudget::from_input(&checked).unwrap();
    let actual = budget
        .count_enabled_tools(&tools, snapshot.definition())
        .unwrap()
        .map(|t| t.descriptor())
        .collect::<Vec<_>>();
    let expected = snapshot
        .definition()
        .tools()
        .filter(|t| t.control != ToolControl::Yield)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    for (a, e) in actual.iter().zip(&expected) {
        assert_eq!(a.arguments_schema.as_ptr(), e.arguments_schema.as_ptr());
    }
    budget.debit_model_call().unwrap();
    budget.debit_control(ToolControl::Continue).unwrap();
    let actual = budget
        .count_enabled_tools(&tools, snapshot.definition())
        .unwrap()
        .map(|t| t.descriptor())
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        snapshot
            .definition()
            .tools()
            .filter(|t| t.control == ToolControl::Finish)
            .collect::<Vec<_>>()
    );
    assert!(actual.is_empty()); // Finish is intrinsic, not an enabled descriptor.
    assert_eq!(budget.remaining().model_calls, 0); // Last admitted reply can still explicitly Finish; units remain a separate guard.
}

#[test]
fn count_eligibility_cannot_rebind_to_another_definition_or_toolset() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let checked = packet
        .validate(&snapshot, &tools, &Limits::default())
        .unwrap();
    let budget = CallBudget::from_input(&checked).unwrap();
    let mut other = cbor(snapshot.definition().as_bytes());
    map_mut(&mut other).insert("mandate".into(), Cbor::Text("other mandate".into()));
    let other = Definition::decode(&encode(&other), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&other);
    let other_tools = ToolSet::bind(&other, &registered, &Limits::default()).unwrap();
    assert_eq!(
        budget.count_enabled_tools(&other_tools, &other).err(),
        Some(CallBudgetError::DefinitionMismatch)
    );
    assert_eq!(
        budget
            .count_enabled_tools(&other_tools, snapshot.definition())
            .err(),
        Some(CallBudgetError::Tools(ToolSetError::DefinitionMismatch))
    );
}

#[test]
fn definition_ceilings_and_full_u64_grants_do_not_narrow_or_get_widened_by_caller() {
    let fixture = &fixtures()[0];
    let mut blocks = store(fixture);
    let root = root_of(fixture);
    let old = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let def = patch(&mut blocks, old.state().definition_link(), |value| {
        let limits = map_mut(value).get_mut("limits").unwrap();
        for name in [
            "max_model_calls_per_activation",
            "max_model_calls_per_episode",
            "max_actions_per_episode",
            "max_local_tools_per_activation",
            "max_corrective_turns",
        ] {
            map_mut(limits).insert(name.into(), Cbor::Integer(u64::MAX.into()));
        }
    });
    let root = patch(&mut blocks, root, |value| {
        map_mut(value).insert("definition".into(), link(def));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    grant(&mut source, u64::MAX, u64::MAX);
    let packet = preparation::packet(&snapshot, &source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let caller = Limits {
        max_model_calls_per_activation: u64::MAX,
        max_model_calls_per_episode: u64::MAX,
        max_actions_per_episode: u64::MAX,
        max_local_tools_per_activation: u64::MAX,
        max_corrective_turns: u64::MAX,
        ..Limits::default()
    };
    let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
    let mut budget = CallBudget::from_input(&checked).unwrap();
    let base = accepted(&snapshot);
    assert_eq!(budget.capacity().model_calls, u64::MAX - base.model_calls);
    assert_eq!(budget.capacity().actions, u64::MAX - base.actions);
    assert_eq!(budget.capacity().local_tool_calls, u64::MAX);
    assert_eq!(budget.capacity().corrective_turns, u64::MAX);
    budget.debit_model_call().unwrap();
    budget.debit_control(ToolControl::Yield).unwrap();
    assert_eq!(
        budget.remaining().model_calls,
        u64::MAX - base.model_calls - 1
    );
    assert_eq!(budget.remaining().actions, u64::MAX - base.actions - 1);
    // Independently lower the Definition, preserving prior accepted counts and wire data.
    let def = patch(&mut blocks, def, |value| {
        let limits = map_mut(value).get_mut("limits").unwrap();
        map_mut(limits).insert(
            "max_model_calls_per_activation".into(),
            Cbor::Integer(2.into()),
        );
        map_mut(limits).insert(
            "max_model_calls_per_episode".into(),
            Cbor::Integer((base.model_calls + 1).into()),
        );
        map_mut(limits).insert(
            "max_actions_per_episode".into(),
            Cbor::Integer((base.actions + 1).into()),
        );
        map_mut(limits).insert(
            "max_local_tools_per_activation".into(),
            Cbor::Integer(3.into()),
        );
        map_mut(limits).insert("max_corrective_turns".into(), Cbor::Integer(1.into()));
    });
    let root = patch(&mut blocks, root, |value| {
        map_mut(value).insert("definition".into(), link(def));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &caller).unwrap();
    let source = full_context::incoming(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &caller).unwrap();
    let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
    let budget = CallBudget::from_input(&checked).unwrap();
    assert_eq!(
        budget.capacity(),
        CallCounts {
            model_calls: 1,
            actions: 1,
            local_tool_calls: 3,
            corrective_turns: 1
        }
    );
}

#[test]
fn zero_counts_refuse_new_work_without_turning_validated_data_into_an_execution_permit() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    grant(&mut source, 0, 0);
    let packet = preparation::packet(&snapshot, &source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let caller = Limits {
        max_local_tools_per_activation: 0,
        max_corrective_turns: 0,
        ..Limits::default()
    };
    let checked = packet.validate(&snapshot, &tools, &caller).unwrap();
    let mut budget = CallBudget::from_input(&checked).unwrap();
    assert_eq!(
        budget.debit_model_call(),
        Err(CallBudgetError::ModelCallsExhausted)
    );
    assert_eq!(
        budget.debit_control(ToolControl::Yield),
        Err(CallBudgetError::ActionsExhausted)
    );
    assert_eq!(
        budget.debit_control(ToolControl::Continue),
        Err(CallBudgetError::LocalToolsExhausted)
    );
    assert_eq!(
        budget.debit_corrective_turn(),
        Err(CallBudgetError::CorrectiveTurnsExhausted)
    );
    assert_eq!(
        budget.episode_counts(),
        EpisodeCounts {
            model_calls: 0,
            actions: 0
        }
    );
}
