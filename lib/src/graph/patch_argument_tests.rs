use super::*;
use crate::{
    activation::{ModelReply, ReplyError},
    format::oracle::json_bytes,
    memory::{PatchArguments, PatchError},
    projection::InitialProjection,
};
use serde_json::json;
fn patch_call(id: &str, args: Cbor) -> Cbor {
    let mut call = reply::call(id, args);
    map_mut(&mut call).insert("tool_id".into(), Cbor::Text("patch_memory".into()));
    call
}
#[test]
fn selected_single_reply_arguments_transform_only_working_memory_and_preserve_initial_context_and_checkpoint_bytes(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let input = full_context::incoming(&snapshot);
    let input = full_context::input(&input);
    let prefix = InitialProjection::build(&snapshot, &input, &Limits::default()).unwrap();
    let before = prefix.as_bytes().to_vec();
    let checkpoint = snapshot.memory().unwrap();
    let pointer = checkpoint.as_bytes().as_ptr();
    let cid = checkpoint.cid();
    let mut working = Memory::decode(checkpoint.as_bytes(), &DecodeLimits::default()).unwrap();
    let args = cbor(&json_bytes(&json!({"intents":{"normalized":"safe"}})));
    let frame = encode(&reply::frame(
        1,
        vec![patch_call("native patch call", args)],
    ));
    let parsed = ModelReply::decode(&frame, &Limits::default()).unwrap();
    let selected = parsed.control_call().unwrap().unwrap();
    let patch = PatchArguments::decode(selected.arguments_bytes(), &Limits::default()).unwrap();
    assert_eq!(
        patch.as_bytes().as_ptr(),
        selected.arguments_bytes().as_ptr()
    );
    let mut expected = cbor(checkpoint.value_bytes());
    map_mut(&mut expected).insert(
        "intents".into(),
        Cbor::Map(vec![(
            Cbor::Text("normalized".into()),
            Cbor::Text("safe".into()),
        )]),
    );
    assert_eq!(patch.apply(&mut working).unwrap(), encode(&expected));
    assert_eq!(snapshot.memory().unwrap().cid(), cid);
    assert_eq!(snapshot.memory().unwrap().as_bytes().as_ptr(), pointer);
    assert_eq!(prefix.as_bytes(), before);
    assert_ne!(working.cid(), cid); // Pure normalization/transform, not registered runtime dispatch or units approval.
}
#[test]
fn multiple_reply_calls_or_invalid_patch_shapes_never_choose_a_first_patch_or_mutate_checkpoint_memory(
) {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let working = Memory::decode(
        snapshot.memory().unwrap().as_bytes(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let before = working.as_bytes().to_vec();
    let pointer = working.as_bytes().as_ptr();
    let args = cbor(&json_bytes(&json!({"intents":{"x":1}})));
    let frame = encode(&reply::frame(
        1,
        vec![
            patch_call("first", args.clone()),
            patch_call("second", args),
        ],
    ));
    let parsed = ModelReply::decode(&frame, &Limits::default()).unwrap();
    assert_eq!(parsed.control_call(), Err(ReplyError::MultipleToolCalls));
    assert_eq!(working.as_bytes(), before);
    assert_eq!(working.as_bytes().as_ptr(), pointer);
    let args = cbor(&json_bytes(&json!({"tactics":[]})));
    let frame = encode(&reply::frame(1, vec![patch_call("invalid", args)]));
    let parsed = ModelReply::decode(&frame, &Limits::default()).unwrap();
    let selected = parsed.control_call().unwrap().unwrap();
    assert!(matches!(
        PatchArguments::decode(selected.arguments_bytes(), &Limits::default()),
        Err(PatchError::PlaneNotMap)
    ));
    assert_eq!(working.as_bytes(), before);
    assert_eq!(working.as_bytes().as_ptr(), pointer);
    let args = cbor(&json_bytes(&json!({"tactics":{"x":[{"bad":null}]}})));
    let frame = encode(&reply::frame(1, vec![patch_call("overlap", args)]));
    let parsed = ModelReply::decode(&frame, &Limits::default()).unwrap();
    let selected = parsed.control_call().unwrap().unwrap();
    assert!(matches!(
        PatchArguments::decode(selected.arguments_bytes(), &Limits::default()),
        Err(PatchError::NullInArray)
    ));
    assert_eq!(working.as_bytes(), before);
    assert_eq!(working.as_bytes().as_ptr(), pointer);
}
