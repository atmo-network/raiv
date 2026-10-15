use super::*;
use crate::{
    activation::{
        CallBudget, CallBudgetError, InferenceError, InferencePhase, InferenceSession,
        RequestSequence, UnitBudget, UnitError,
    },
    definition::ToolControl,
    tools::ToolSet,
};

fn funded(
    snapshot: &RestoredAgent,
    source: &Cbor,
    caller: &Limits,
) -> (InferenceSession, UnitBudget) {
    let packet = preparation::packet(snapshot, source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, caller).unwrap();
    let checked = packet.validate(snapshot, &tools, caller).unwrap();
    (
        InferenceSession::new(
            RequestSequence::new([93; 32]),
            CallBudget::from_input(&checked).unwrap(),
        ),
        UnitBudget::from_input(&checked),
    )
}

fn set_units(source: &mut Cbor, activation: u64, episode: u64) {
    let grant = map_mut(source).get_mut("budget_admission").unwrap();
    map_mut(grant).insert("activation_units".into(), Cbor::Integer(activation.into()));
    map_mut(grant).insert(
        "remaining_episode_units".into(),
        Cbor::Integer(episode.into()),
    );
}

fn number(value: &Cbor) -> u64 {
    let Cbor::Integer(value) = value else {
        panic!("number")
    };
    (*value).try_into().unwrap()
}

#[test]
fn checked_packet_grants_carry_full_scope_admission_and_independent_unit_capacities_without_guessing_usage(
) {
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let source = full_context::incoming(&snapshot);
        let (run, units) = funded(&snapshot, &source, &Limits::default());
        let grant = field(&source, "budget_admission");
        assert_eq!(units.binding(), run.budget().binding());
        let Cbor::Bytes(id) = field(grant, "admission_id") else {
            panic!()
        };
        assert_eq!(&units.admission_id(), id.as_slice());
        assert_eq!(units.reserved_units(), 0);
        assert_eq!(
            units.remaining_activation_units(),
            number(field(grant, "activation_units"))
        );
        assert_eq!(
            units.remaining_episode_units(),
            number(field(grant, "remaining_episode_units"))
        );
    }
}

#[test]
fn model_and_explicit_controls_reserve_counts_and_units_once_before_work_with_no_view_reply_or_terminal_double_debit(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    set_units(&mut source, 8, 10);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let first = run.issue_with_units(&mut units, 2).unwrap();
    assert_eq!(units.reserved_units(), 2);
    for _ in 0..3 {
        assert_eq!(run.pending_request(), Some(first));
        assert_eq!(run.check_response(first), Ok(()));
    }
    assert_eq!(units.reserved_units(), 2);
    run.accept_reply(first).unwrap();
    run.control_with_units(ToolControl::Continue, &mut units, 3)
        .unwrap();
    assert_eq!(units.reserved_units(), 5);
    assert_eq!(run.budget().used().local_tool_calls, 1);
    let second = run.issue_with_units(&mut units, 2).unwrap();
    run.accept_reply(second).unwrap();
    run.control_with_units(ToolControl::Finish, &mut units, 1)
        .unwrap();
    assert_eq!(units.reserved_units(), 8);
    assert_eq!(units.remaining_activation_units(), 0);
    assert_eq!(units.remaining_episode_units(), 2);
    assert_eq!(run.phase(), InferencePhase::Sealed);
    assert_eq!(run.budget().used().model_calls, 2);
    assert_eq!(run.budget().used().actions, 0);
    assert_eq!(
        run.control_with_units(ToolControl::Finish, &mut units, 0),
        Err(InferenceError::WrongPhase)
    );
    assert_eq!(units.reserved_units(), 8);
}

#[test]
fn explicit_yield_reserves_units_and_one_action_then_seals_without_claiming_a_constructed_candidate(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    set_units(&mut source, 9, 12);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let key = run.issue_with_units(&mut units, 2).unwrap();
    run.accept_reply(key).unwrap();
    run.control_with_units(ToolControl::Yield, &mut units, 3)
        .unwrap();
    assert_eq!(run.phase(), InferencePhase::Sealed);
    assert_eq!(run.budget().used().actions, 1);
    assert_eq!(units.reserved_units(), 5);
    assert_eq!(units.remaining_activation_units(), 4);
    assert_eq!(units.remaining_episode_units(), 7);
}

#[test]
fn exhausted_activation_or_episode_and_over_quote_fail_without_a_request_control_or_implicit_no_op()
{
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    for (activation, episode, quote) in [(0, 9, 0), (9, 0, 0), (2, 9, 3), (9, 2, 3)] {
        let mut source = full_context::incoming(&snapshot);
        set_units(&mut source, activation, episode);
        let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
        assert_eq!(
            run.issue_with_units(&mut units, quote),
            Err(InferenceError::Units(UnitError::Exhausted))
        );
        assert_eq!(run.phase(), InferencePhase::Failed);
        assert!(run.pending_request().is_none());
        assert_eq!(run.budget().used().model_calls, 0);
        assert_eq!(units.reserved_units(), 0);
        assert_eq!(run.release_sequence().unwrap().last_id(), 0);
    }
    let mut source = full_context::incoming(&snapshot);
    set_units(&mut source, 1, 9);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let key = run.issue_with_units(&mut units, 1).unwrap();
    run.accept_reply(key).unwrap();
    assert_eq!(
        run.control_with_units(ToolControl::Finish, &mut units, 0),
        Err(InferenceError::Units(UnitError::Exhausted))
    );
    assert_eq!(run.phase(), InferencePhase::Failed);
    assert_eq!(run.budget().used().actions, 0);
    assert_eq!(units.reserved_units(), 1);
}

#[test]
fn wrong_packet_phase_or_count_refusal_preserves_unit_ledger_and_never_changes_another_pending_request(
) {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    set_units(&mut source, 10, 10);
    let (mut run, mut units) = funded(
        &snapshot,
        &source,
        &Limits {
            max_model_calls_per_activation: 0,
            ..Limits::default()
        },
    );
    assert!(matches!(
        run.issue_with_units(&mut units, 2),
        Err(InferenceError::Counts(CallBudgetError::ModelCallsExhausted))
    ));
    assert_eq!(units.reserved_units(), 0);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let mut other = source.clone();
    map_mut(&mut other).insert("event_id".into(), Cbor::Bytes(vec![94; 32]));
    let (_, mut wrong) = funded(&snapshot, &other, &Limits::default());
    assert_eq!(
        run.issue_with_units(&mut wrong, 2),
        Err(InferenceError::Units(UnitError::WrongInput))
    );
    assert_eq!(run.phase(), InferencePhase::Ready);
    assert_eq!(run.budget().used().model_calls, 0);
    assert_eq!(wrong.reserved_units(), 0);
    let key = run.issue_with_units(&mut units, 2).unwrap();
    assert_eq!(
        run.issue_with_units(&mut wrong, 2),
        Err(InferenceError::WrongPhase)
    );
    assert_eq!(run.pending_request(), Some(key));
    assert_eq!(units.reserved_units(), 2);
    run.accept_reply(key).unwrap();
    assert_eq!(
        run.control_with_units(ToolControl::Finish, &mut wrong, 1),
        Err(InferenceError::Units(UnitError::WrongInput))
    );
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    assert_eq!(wrong.reserved_units(), 0);
    let mut changed = source.clone();
    map_mut(map_mut(&mut changed).get_mut("budget_admission").unwrap())
        .insert("admission_id".into(), Cbor::Bytes(vec![77; 32]));
    let (_, mut changed_units) = funded(&snapshot, &changed, &Limits::default());
    assert_eq!(
        run.control_with_units(ToolControl::Finish, &mut changed_units, 1),
        Err(InferenceError::Units(UnitError::WrongInput))
    );
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    let (mut run, mut units) = funded(
        &snapshot,
        &source,
        &Limits {
            max_actions_per_episode: 0,
            ..Limits::default()
        },
    );
    let key = run.issue_with_units(&mut units, 2).unwrap();
    run.accept_reply(key).unwrap();
    assert_eq!(
        run.control_with_units(ToolControl::Yield, &mut units, 3),
        Err(InferenceError::Counts(CallBudgetError::ActionsExhausted))
    );
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    assert_eq!(units.reserved_units(), 2);
    let (mut run, mut units) = funded(
        &snapshot,
        &source,
        &Limits {
            max_local_tools_per_activation: 0,
            ..Limits::default()
        },
    );
    let key = run.issue_with_units(&mut units, 2).unwrap();
    run.accept_reply(key).unwrap();
    assert_eq!(
        run.control_with_units(ToolControl::Continue, &mut units, 3),
        Err(InferenceError::Counts(CallBudgetError::LocalToolsExhausted))
    );
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    assert_eq!(units.reserved_units(), 2);
}

#[test]
fn unknown_reported_billing_and_provider_failures_cannot_refund_reservations_or_override_private_counts(
) {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    set_units(&mut source, 10, 20);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let key = run.issue_with_units(&mut units, 3).unwrap();
    let bytes = encode(&reply::frame(key.request_id, vec![]));
    let parsed = run
        .accept_normalized_reply(key, &bytes, snapshot.definition(), &Limits::default())
        .unwrap();
    assert_eq!(parsed.usage().billed_units, None);
    assert_eq!(units.reserved_units(), 3);
    assert_eq!(run.budget().used().model_calls, 1);
    run.corrective_turn().unwrap();
    let next = run.issue_with_units(&mut units, 4).unwrap();
    run.inference_error(next).unwrap();
    assert_eq!(run.phase(), InferencePhase::Failed);
    assert_eq!(units.reserved_units(), 7);
    assert_eq!(units.remaining_episode_units(), 13);
    assert_eq!(run.budget().used().model_calls, 2);
}

#[test]
fn full_u64_exact_reservation_and_free_positive_capacity_are_checked_without_arithmetic_wrap() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    set_units(&mut source, u64::MAX, u64::MAX);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let key = run.issue_with_units(&mut units, u64::MAX).unwrap();
    assert_eq!(units.reserved_units(), u64::MAX);
    assert_eq!(units.remaining_activation_units(), 0);
    run.inference_error(key).unwrap();
    assert_eq!(units.reserved_units(), u64::MAX);
    set_units(&mut source, 2, 3);
    let (mut run, mut units) = funded(&snapshot, &source, &Limits::default());
    let key = run.issue_with_units(&mut units, 0).unwrap();
    run.accept_reply(key).unwrap();
    run.control_with_units(ToolControl::Finish, &mut units, 0)
        .unwrap();
    assert_eq!(run.phase(), InferencePhase::Sealed);
    assert_eq!(units.reserved_units(), 0);
    assert_eq!(units.remaining_activation_units(), 2);
    assert_eq!(run.budget().used().model_calls, 1);
}
