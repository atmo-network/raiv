use super::*;
use crate::{
    activation::{CallMeta, InferenceError, InferencePhase, ReplyError},
    definition::ToolControl,
    tools::ToolSet,
};
fn map(entries: Vec<(&str, Cbor)>) -> Cbor {
    Cbor::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Cbor::Text(k.into()), v))
            .collect(),
    )
}
pub(super) fn call(id: &str, args: Cbor) -> Cbor {
    map(vec![
        ("call_id", Cbor::Text(id.into())),
        ("tool_id", Cbor::Text("no_op".into())),
        ("arguments", args),
    ])
}
pub(super) fn frame(request: u64, calls: Vec<Cbor>) -> Cbor {
    map(vec![
        ("request_id", Cbor::Integer(request.into())),
        ("text", Cbor::Text("".into())),
        ("calls", Cbor::Array(calls)),
        (
            "usage",
            map(vec![
                ("model_calls", Cbor::Integer(u64::MAX.into())),
                ("local_tool_calls", Cbor::Integer(u64::MAX.into())),
                ("input_tokens", Cbor::Null),
                ("output_tokens", Cbor::Null),
                ("billed_units", Cbor::Null),
            ]),
        ),
    ])
}
#[test]
fn correlation_precedes_parsing_and_body_ids_closed_schemas_and_truncation_do_not_consume_pending_state(
) {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let input = full_context::incoming(&snapshot);
    let mut run = inference::session(&snapshot, &input, &Limits::default());
    let key = run.issue().unwrap();
    let before = run.budget().used();
    assert_eq!(
        run.accept_normalized_reply(
            crate::activation::RequestBinding {
                instance_id: [94; 32],
                ..key
            },
            &[0xff],
            snapshot.definition(),
            &Limits {
                max_model_reply_bytes: 0,
                ..Limits::default()
            }
        )
        .err(),
        Some(ReplyError::Inference(InferenceError::WrongInstance))
    );
    assert_eq!(run.pending_request(), Some(key));
    let wrong = encode(&frame(key.request_id + 1, vec![]));
    assert_eq!(
        run.accept_normalized_reply(key, &wrong, snapshot.definition(), &Limits::default())
            .err(),
        Some(ReplyError::Inference(InferenceError::WrongRequest))
    );
    let good = encode(&frame(
        key.request_id,
        vec![call("finish", Cbor::Map(vec![]))],
    ));
    for end in 0..good.len() {
        assert!(run
            .accept_normalized_reply(key, &good[..end], snapshot.definition(), &Limits::default())
            .is_err());
        assert_eq!(run.pending_request(), Some(key));
        assert_eq!(run.budget().used(), before);
    }
    let mut bad = frame(key.request_id, vec![]);
    map_mut(&mut bad).insert("unexpected".into(), Cbor::Null);
    assert_eq!(
        run.accept_normalized_reply(
            key,
            &encode(&bad),
            snapshot.definition(),
            &Limits::default()
        )
        .err(),
        Some(ReplyError::InvalidSchema)
    );
    assert_eq!(run.pending_request(), Some(key));
    let reply = run
        .accept_normalized_reply(key, &good, snapshot.definition(), &Limits::default())
        .unwrap();
    assert_eq!(
        reply.control_call().unwrap().unwrap().meta().call_id,
        "finish"
    );
    assert_eq!(reply.usage().model_calls, u64::MAX);
    assert_eq!(run.budget().used().model_calls, 1);
    assert_eq!(run.budget().used().local_tool_calls, 0);
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    assert_eq!(
        run.accept_normalized_reply(key, &good, snapshot.definition(), &Limits::default())
            .err(),
        Some(ReplyError::Inference(InferenceError::WrongPhase))
    );
    run.control(ToolControl::Finish).unwrap();
}
#[test]
fn complete_multi_call_frames_consume_correlation_once_but_select_no_first_call_and_recover_with_all_native_ids(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let input = full_context::incoming(&snapshot);
    let mut run = inference::session(&snapshot, &input, &Limits::default());
    let key = run.issue().unwrap();
    let bytes = encode(&frame(
        key.request_id,
        vec![
            call("first", Cbor::Map(vec![])),
            call("second", Cbor::Bytes(vec![1])),
        ],
    ));
    let reply = run
        .accept_normalized_reply(key, &bytes, snapshot.definition(), &Limits::default())
        .unwrap();
    assert_eq!(reply.control_call(), Err(ReplyError::MultipleToolCalls));
    assert_eq!(
        reply.calls().collect::<Vec<_>>(),
        vec![
            CallMeta {
                call_id: "first",
                tool_id: "no_op"
            },
            CallMeta {
                call_id: "second",
                tool_id: "no_op"
            }
        ]
    );
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    assert_eq!(run.budget().used().local_tool_calls, 0);
    assert_eq!(run.budget().used().actions, 0);
    run.corrective_turn().unwrap();
    let next = run.issue().unwrap();
    assert_eq!(next.request_id, key.request_id + 1);
    let bytes = encode(&frame(
        next.request_id,
        vec![call("invalid", Cbor::Text("not normalized map".into()))],
    ));
    let reply = run
        .accept_normalized_reply(next, &bytes, snapshot.definition(), &Limits::default())
        .unwrap();
    assert_eq!(reply.control_call(), Err(ReplyError::InvalidArguments));
    assert_eq!(run.budget().used().local_tool_calls, 0);
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
    run.corrective_turn().unwrap();
    assert_eq!(run.budget().used().corrective_turns, 2);
}
#[test]
fn reply_limits_intersect_definition_and_caller_without_using_structural_block_limits_or_truncating_frames(
) {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let input = full_context::incoming(&snapshot);
    let mut run = inference::session(&snapshot, &input, &Limits::default());
    let key = run.issue().unwrap();
    let bytes = encode(&frame(key.request_id, vec![]));
    assert_eq!(
        run.accept_normalized_reply(
            key,
            &bytes,
            snapshot.definition(),
            &Limits {
                max_model_reply_bytes: bytes.len() as u64 - 1,
                ..Limits::default()
            }
        )
        .err(),
        Some(ReplyError::Codec(CodecError::ByteLimit))
    );
    assert_eq!(run.pending_request(), Some(key));
    let mut large = frame(key.request_id, vec![]);
    map_mut(&mut large).insert(
        "text".into(),
        Cbor::Text("x".repeat(snapshot.definition().limits().max_model_reply_bytes as usize)),
    );
    let large = encode(&large);
    assert_eq!(
        run.accept_normalized_reply(
            key,
            &large,
            snapshot.definition(),
            &Limits {
                max_model_reply_bytes: u64::MAX,
                ..Limits::default()
            }
        )
        .err(),
        Some(ReplyError::Codec(CodecError::ByteLimit))
    );
    assert_eq!(run.pending_request(), Some(key));
    for caller in [
        Limits {
            max_depth: 1,
            ..Limits::default()
        },
        Limits {
            max_collection_entries: 4,
            ..Limits::default()
        },
    ] {
        assert!(run
            .accept_normalized_reply(key, &bytes, snapshot.definition(), &caller)
            .is_err());
        assert_eq!(run.pending_request(), Some(key));
    }
    assert!(run
        .accept_normalized_reply(
            key,
            &bytes,
            snapshot.definition(),
            &Limits {
                max_model_reply_bytes: bytes.len() as u64,
                max_block_bytes: 0,
                ..Limits::default()
            }
        )
        .is_ok());
    assert_eq!(run.phase(), InferencePhase::ReplyReady);
}
#[test]
fn another_definition_cannot_supply_wider_reply_limits_and_no_byte_view_authorizes_tools() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let input = full_context::incoming(&snapshot);
    let mut run = inference::session(&snapshot, &input, &Limits::default());
    let key = run.issue().unwrap();
    let mut definition = cbor(snapshot.definition().as_bytes());
    map_mut(&mut definition).insert("policy".into(), Cbor::Text("other policy".into()));
    let other = Definition::decode(&encode(&definition), &DecodeLimits::default()).unwrap();
    let bytes = encode(&frame(
        key.request_id,
        vec![call("arbitrary native ID", Cbor::Map(vec![]))],
    ));
    assert_eq!(
        run.accept_normalized_reply(key, &bytes, &other, &Limits::default())
            .err(),
        Some(ReplyError::DefinitionMismatch)
    );
    assert_eq!(run.pending_request(), Some(key));
    let registered = preparation::registrations(snapshot.definition());
    let tools = ToolSet::bind(snapshot.definition(), &registered, &Limits::default()).unwrap();
    let reply = run
        .accept_normalized_reply(key, &bytes, snapshot.definition(), &Limits::default())
        .unwrap();
    let selected = reply.control_call().unwrap().unwrap();
    assert_eq!(selected.meta().call_id, "arbitrary native ID");
    assert!(tools.lookup("outside_definition").is_none());
    assert_eq!(run.budget().used().local_tool_calls, 0); // No validator/planner has been selected or invoked by admission.
}
