use super::*;
use crate::{
    activation::{
        CallBudget, CallBudgetError, InferenceError, InferencePhase, InferenceSession,
        RequestBinding, RequestSequence,
    },
    definition::ToolControl,
    tools::ToolSet,
};

pub(super) fn session(
    snapshot: &RestoredAgent,
    source: &Cbor,
    caller: &Limits,
) -> InferenceSession {
    let packet = preparation::packet(snapshot, source);
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, caller).unwrap();
    let checked = packet.validate(snapshot, &tools, caller).unwrap();
    InferenceSession::new(
        RequestSequence::new([93; 32]),
        CallBudget::from_input(&checked).unwrap(),
    )
}
#[test]
fn one_outstanding_request_and_stable_pending_views_debit_only_issue_not_reply_or_inspection() {
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let source = full_context::incoming(&snapshot);
        let mut run = session(&snapshot, &source, &Limits::default());
        let before = run.budget().used();
        assert_eq!(run.phase(), InferencePhase::Ready);
        assert!(run.pending_request().is_none());
        assert_eq!(
            run.control(ToolControl::Finish),
            Err(InferenceError::WrongPhase)
        );
        assert_eq!(run.corrective_turn(), Err(InferenceError::WrongPhase));
        assert_eq!(run.release_sequence(), Err(InferenceError::WrongPhase));
        assert_eq!(run.budget().used(), before);
        let key = run.issue().unwrap();
        assert_eq!(
            key,
            RequestBinding {
                instance_id: [93; 32],
                request_id: 1
            }
        );
        assert_eq!(run.budget().used().model_calls, before.model_calls + 1);
        let used = run.budget().used();
        for _ in 0..3 {
            assert_eq!(run.pending_request(), Some(key));
            assert_eq!(run.check_response(key), Ok(()));
        }
        assert_eq!(run.issue(), Err(InferenceError::WrongPhase));
        assert_eq!(
            run.control(ToolControl::Continue),
            Err(InferenceError::WrongPhase)
        );
        assert_eq!(run.release_sequence(), Err(InferenceError::WrongPhase));
        assert_eq!(run.budget().used(), used);
        run.accept_reply(key).unwrap();
        assert_eq!(run.phase(), InferencePhase::ReplyReady);
        assert!(run.pending_request().is_none());
        assert_eq!(run.budget().used(), used);
        assert_eq!(run.issue(), Err(InferenceError::WrongPhase));
        assert_eq!(run.accept_reply(key), Err(InferenceError::WrongPhase));
        assert_eq!(run.inference_error(key), Err(InferenceError::WrongPhase));
        assert_eq!(run.budget().used(), used);
    }
}
#[test]
fn wrong_instance_stale_future_duplicate_and_delayed_error_keys_leave_pending_state_exact() {
    let fixture = &identity_graphs()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    let mut run = session(&snapshot, &source, &Limits::default());
    let key = run.issue().unwrap();
    let before = run.budget().used();
    for (wrong, error) in [
        (
            RequestBinding {
                instance_id: [94; 32],
                ..key
            },
            InferenceError::WrongInstance,
        ),
        (
            RequestBinding {
                request_id: 0,
                ..key
            },
            InferenceError::WrongRequest,
        ),
        (
            RequestBinding {
                request_id: u64::MAX,
                ..key
            },
            InferenceError::WrongRequest,
        ),
    ] {
        assert_eq!(run.accept_reply(wrong), Err(error));
        assert_eq!(run.inference_error(wrong), Err(error));
        assert_eq!(run.check_response(wrong), Err(error));
        assert_eq!(run.pending_request(), Some(key));
        assert_eq!(run.budget().used(), before);
    }
    run.accept_reply(key).unwrap();
    run.corrective_turn().unwrap();
    let next = run.issue().unwrap();
    assert_eq!(next.request_id, 2);
    assert_eq!(run.accept_reply(key), Err(InferenceError::WrongRequest));
    assert_eq!(run.inference_error(key), Err(InferenceError::WrongRequest));
    assert_eq!(run.pending_request(), Some(next));
    assert_eq!(run.budget().used().model_calls, 2);
    run.inference_error(next).unwrap();
    assert_eq!(run.phase(), InferencePhase::Failed);
    assert!(run.pending_request().is_none());
    assert_eq!(run.issue(), Err(InferenceError::WrongPhase));
    assert_eq!(
        run.control(ToolControl::Finish),
        Err(InferenceError::WrongPhase)
    );
    assert_eq!(run.budget().used().model_calls, 2);
}
#[test]
fn continue_and_corrective_cycles_preserve_episode_counts_and_monotonic_ids_without_mutating_checkpoint_buffers(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    let admission = map_mut(&mut source).get_mut("budget_admission").unwrap();
    map_mut(admission).insert("remaining_model_calls".into(), Cbor::Integer(3.into()));
    let mut run = session(&snapshot, &source, &Limits::default());
    let base = run.budget().accepted_episode_counts();
    let before = snapshot
        .blocks()
        .map(|(cid, b)| (cid, b.as_ptr()))
        .collect::<BTreeMap<_, _>>();
    let first = run.issue().unwrap();
    run.accept_reply(first).unwrap();
    run.control(ToolControl::Continue).unwrap();
    assert_eq!(run.phase(), InferencePhase::Ready);
    assert_eq!(run.budget().used().local_tool_calls, 1);
    let second = run.issue().unwrap();
    assert_eq!(second.request_id, 2);
    run.accept_reply(second).unwrap();
    run.corrective_turn().unwrap();
    assert_eq!(run.budget().used().corrective_turns, 1);
    let third = run.issue().unwrap();
    assert_eq!(third.request_id, 3);
    run.accept_reply(third).unwrap();
    assert_eq!(run.budget().remaining().model_calls, 0);
    run.control(ToolControl::Finish).unwrap();
    assert_eq!(run.phase(), InferencePhase::Sealed);
    assert_eq!(
        run.budget().episode_counts().model_calls,
        base.model_calls + 3
    );
    assert_eq!(run.budget().episode_counts().actions, base.actions);
    assert_eq!(
        snapshot
            .blocks()
            .map(|(cid, b)| (cid, b.as_ptr()))
            .collect::<BTreeMap<_, _>>(),
        before
    );
}
#[test]
fn terminal_control_seals_and_instance_sequence_survives_the_next_activation_without_id_reuse() {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let source = full_context::incoming(&snapshot);
    for control in [ToolControl::Yield, ToolControl::Finish] {
        let mut run = session(&snapshot, &source, &Limits::default());
        let first = run.issue().unwrap();
        run.accept_reply(first).unwrap();
        run.control(control).unwrap();
        assert_eq!(run.phase(), InferencePhase::Sealed);
        assert_eq!(
            run.budget().used().actions,
            u64::from(control == ToolControl::Yield)
        );
        let used = run.budget().used();
        assert_eq!(run.issue(), Err(InferenceError::WrongPhase));
        assert_eq!(run.accept_reply(first), Err(InferenceError::WrongPhase));
        assert_eq!(run.control(control), Err(InferenceError::WrongPhase));
        assert_eq!(run.budget().used(), used);
        let sequence = run.release_sequence().unwrap();
        assert_eq!(sequence.last_id(), 1);
        assert_eq!(sequence.instance_id(), [93; 32]);
        assert_eq!(run.phase(), InferencePhase::Released);
        assert_eq!(run.release_sequence(), Err(InferenceError::WrongPhase));
        // Test only nonce continuity; acceptance/fresh admission are external prerequisites.
        let packet = preparation::packet(&snapshot, &source);
        let registered = preparation::registrations(snapshot.definition());
        let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
        let checked = packet
            .validate(&snapshot, &tools, &Limits::default())
            .unwrap();
        let mut next = InferenceSession::new(sequence, CallBudget::from_input(&checked).unwrap());
        let key = next.issue().unwrap();
        assert_eq!(key.request_id, 2);
        assert_eq!(next.accept_reply(first), Err(InferenceError::WrongRequest));
        assert_eq!(next.pending_request(), Some(key));
    }
}
#[test]
fn refused_control_can_correct_but_corrective_exhaustion_and_failed_model_admission_never_seal_no_op(
) {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    let admission = map_mut(&mut source).get_mut("budget_admission").unwrap();
    map_mut(admission).insert("remaining_model_calls".into(), Cbor::Integer(1.into()));
    map_mut(admission).insert("remaining_actions".into(), Cbor::Integer(0.into()));
    let mut run = session(
        &snapshot,
        &source,
        &Limits {
            max_corrective_turns: 1,
            ..Limits::default()
        },
    );
    let key = run.issue().unwrap();
    run.accept_reply(key).unwrap();
    let before = run.budget().used();
    assert_eq!(
        run.control(ToolControl::Yield),
        Err(InferenceError::Counts(CallBudgetError::ActionsExhausted))
    );
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    assert_eq!(run.budget().used(), before);
    run.corrective_turn().unwrap();
    assert_eq!(
        run.issue(),
        Err(InferenceError::Counts(CallBudgetError::ModelCallsExhausted))
    );
    assert_eq!(run.phase(), InferencePhase::Failed);
    assert_eq!(
        run.control(ToolControl::Finish),
        Err(InferenceError::WrongPhase)
    );
    assert_eq!(run.budget().used().model_calls, 1);
    assert_eq!(run.release_sequence().unwrap().last_id(), 1);
    let mut run = session(
        &snapshot,
        &source,
        &Limits {
            max_corrective_turns: 0,
            ..Limits::default()
        },
    );
    let key = run.issue().unwrap();
    run.accept_reply(key).unwrap();
    assert_eq!(
        run.corrective_turn(),
        Err(InferenceError::Counts(
            CallBudgetError::CorrectiveTurnsExhausted
        ))
    );
    assert_eq!(run.phase(), InferencePhase::Failed);
    assert_eq!(run.budget().used().corrective_turns, 0);
    assert_eq!(run.budget().used().actions, 0);
}
#[test]
fn repeated_plain_text_corrections_are_activation_bounded_and_failure_preserves_spent_attempts() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut source = full_context::incoming(&snapshot);
    map_mut(map_mut(&mut source).get_mut("budget_admission").unwrap())
        .insert("remaining_model_calls".into(), Cbor::Integer(3.into()));
    let mut run = session(
        &snapshot,
        &source,
        &Limits {
            max_corrective_turns: 2,
            ..Limits::default()
        },
    );
    for expected in 1..=2 {
        let key = run.issue().unwrap();
        assert_eq!(key.request_id, expected);
        run.accept_reply(key).unwrap();
        run.corrective_turn().unwrap();
    }
    let key = run.issue().unwrap();
    assert_eq!(key.request_id, 3);
    run.accept_reply(key).unwrap();
    assert_eq!(
        run.corrective_turn(),
        Err(InferenceError::Counts(
            CallBudgetError::CorrectiveTurnsExhausted
        ))
    );
    assert_eq!(run.phase(), InferencePhase::Failed);
    assert_eq!(run.budget().used().model_calls, 3);
    assert_eq!(run.budget().used().corrective_turns, 2);
    assert_eq!(run.budget().used().local_tool_calls, 0);
    assert_eq!(run.budget().used().actions, 0);
}
