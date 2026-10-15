use super::*;
use crate::format::oracle::{encode, map_mut};
use alloc::{vec, vec::Vec};
use ciborium::value::Value as Cbor;

fn map(entries: Vec<(&str, Cbor)>) -> Cbor {
    Cbor::Map(
        entries
            .into_iter()
            .map(|(key, value)| (Cbor::Text(key.into()), value))
            .collect(),
    )
}

fn call(id: &str, arguments: Cbor) -> Cbor {
    map(vec![
        ("call_id", Cbor::Text(id.into())),
        ("tool_id", Cbor::Text("patch_memory".into())),
        ("arguments", arguments),
    ])
}

fn frame(request: u64, calls: Vec<Cbor>) -> Cbor {
    map(vec![
        ("request_id", Cbor::Integer(request.into())),
        ("text", Cbor::Text("machine reply é".into())),
        ("calls", Cbor::Array(calls)),
        (
            "usage",
            map(vec![
                ("model_calls", Cbor::Integer(u64::MAX.into())),
                ("local_tool_calls", Cbor::Integer(u64::MAX.into())),
                ("input_tokens", Cbor::Null),
                ("output_tokens", Cbor::Integer(u64::MAX.into())),
                ("billed_units", Cbor::Null),
            ]),
        ),
    ])
}

fn arguments() -> Cbor {
    map(vec![
        ("literal", Cbor::Text("e\u{301}".into())),
        ("max", Cbor::Integer(9_007_199_254_740_991_u64.into())),
        ("min", Cbor::Integer((-9_007_199_254_740_991_i64).into())),
    ])
}

#[test]
fn full_width_runtime_metadata_and_borrowed_exact_argument_spans_do_not_become_model_numbers_or_owned_payloads(
) {
    let args = arguments();
    let expected = encode(&args);
    let bytes = encode(&frame(u64::MAX, vec![call("native_é", args)]));
    let reply = ModelReply::decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(reply.as_bytes().as_ptr(), bytes.as_ptr());
    assert_eq!(reply.request_id(), u64::MAX);
    assert_eq!(reply.text(), "machine reply é");
    assert_eq!(
        reply.usage(),
        Usage {
            model_calls: u64::MAX,
            local_tool_calls: u64::MAX,
            input_tokens: None,
            output_tokens: Some(u64::MAX),
            billed_units: None
        }
    );
    assert_eq!(reply.call_count(), 1);
    let selected = reply.control_call().unwrap().unwrap();
    assert_eq!(
        selected.meta(),
        CallMeta {
            call_id: "native_é",
            tool_id: "patch_memory"
        }
    );
    assert_eq!(selected.arguments_bytes(), expected);
    let offset = bytes
        .windows(expected.len())
        .position(|b| b == expected)
        .unwrap();
    assert_eq!(
        selected.arguments_bytes().as_ptr(),
        bytes[offset..].as_ptr()
    );
    assert_eq!(reply.calls().collect::<Vec<_>>(), vec![selected.meta()]);
    let id_offset = bytes
        .windows("native_é".len())
        .position(|b| b == "native_é".as_bytes())
        .unwrap();
    assert_eq!(
        selected.meta().call_id.as_ptr(),
        bytes[id_offset..].as_ptr()
    );
}

#[test]
fn empty_control_is_none_and_multiple_calls_reject_all_before_any_argument_selection() {
    let bytes = encode(&frame(0, vec![]));
    let reply = ModelReply::decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(reply.control_call(), Ok(None));
    assert_eq!(reply.calls().count(), 0);
    for bad in [
        Cbor::Bytes(vec![1, 2]),
        Cbor::Integer(u64::MAX.into()),
        Cbor::Text("not a map".into()),
    ] {
        let bytes = encode(&frame(
            1,
            vec![call("valid_first", arguments()), call("bad_second", bad)],
        ));
        let reply = ModelReply::decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(reply.control_call(), Err(ReplyError::MultipleToolCalls));
        assert_eq!(
            reply.calls().map(|meta| meta.call_id).collect::<Vec<_>>(),
            vec!["valid_first", "bad_second"]
        );
        assert_eq!(reply.call_count(), 2);
    }
}

#[test]
fn single_arguments_require_a_model_map_without_runtime_values_lossy_integers_or_scalar_coercion() {
    for (arg, error) in [
        (
            Cbor::Bytes(vec![1]),
            ReplyError::Codec(CodecError::UnsupportedValue),
        ),
        (
            map(vec![("x", Cbor::Integer(u64::MAX.into()))]),
            ReplyError::Codec(CodecError::ModelIntegerRange),
        ),
        (Cbor::Text("{}".into()), ReplyError::InvalidArguments),
        (Cbor::Null, ReplyError::InvalidArguments),
        (Cbor::Array(vec![]), ReplyError::InvalidArguments),
    ] {
        let bytes = encode(&frame(1, vec![call("x", arg)]));
        let reply = ModelReply::decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(reply.control_call(), Err(error));
        assert_eq!(reply.calls().next().unwrap().call_id, "x");
    }
}

#[test]
fn total_reply_bounds_use_reply_not_block_ceiling_and_cover_real_embedding_depth_and_counts() {
    let bytes = encode(&frame(1, vec![call("x", arguments())]));
    let exact = Limits {
        max_model_reply_bytes: bytes.len() as u64,
        max_block_bytes: 0,
        ..Limits::default()
    };
    let reply = ModelReply::decode(&bytes, &exact).unwrap();
    assert!(reply.control_call().unwrap().is_some());
    assert_eq!(
        ModelReply::decode(
            &bytes,
            &Limits {
                max_model_reply_bytes: bytes.len() as u64 - 1,
                ..exact
            }
        )
        .err(),
        Some(ReplyError::Codec(CodecError::ByteLimit))
    );
    assert_eq!(
        ModelReply::decode(
            &bytes,
            &Limits {
                max_depth: 4,
                ..exact
            }
        )
        .err(),
        Some(ReplyError::Codec(CodecError::DepthLimit))
    );
    assert!(ModelReply::decode(
        &bytes,
        &Limits {
            max_depth: 5,
            ..exact
        }
    )
    .is_ok());
    assert_eq!(
        ModelReply::decode(
            &bytes,
            &Limits {
                max_collection_entries: 4,
                ..exact
            }
        )
        .err(),
        Some(ReplyError::Codec(CodecError::CollectionLimit))
    );
    assert!(ModelReply::decode(
        &bytes,
        &Limits {
            max_collection_entries: 5,
            ..exact
        }
    )
    .is_ok());
    let mut huge = frame(1, vec![]);
    map_mut(&mut huge).insert("text".into(), Cbor::Text("x".repeat(70_000)));
    let huge = encode(&huge);
    assert!(ModelReply::decode(
        &huge,
        &Limits {
            max_model_reply_bytes: huge.len() as u64,
            ..Limits::default()
        }
    )
    .is_ok());
}

#[test]
fn all_truncated_prefixes_trailing_noncanonical_duplicate_and_unknown_fields_reject_as_frames() {
    let source = frame(1, vec![call("x", arguments())]);
    let bytes = encode(&source);
    for end in 0..bytes.len() {
        assert!(
            ModelReply::decode(&bytes[..end], &Limits::default()).is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        ModelReply::decode(&trailing, &Limits::default()).err(),
        Some(ReplyError::Codec(CodecError::TrailingBytes))
    );
    let mut extra = source.clone();
    map_mut(&mut extra).insert("authority".into(), Cbor::Null);
    assert_eq!(
        ModelReply::decode(&encode(&extra), &Limits::default()).err(),
        Some(ReplyError::InvalidSchema)
    );
    let mut missing = source.clone();
    let Cbor::Map(entries) = &mut missing else {
        panic!()
    };
    entries.retain(|(key, _)| key != &Cbor::Text("text".into()));
    assert_eq!(
        ModelReply::decode(&encode(&missing), &Limits::default()).err(),
        Some(ReplyError::InvalidSchema)
    );
    let mut duplicate = vec![0xa5];
    duplicate.extend_from_slice(&bytes[1..]);
    duplicate.extend_from_slice(&[0x64, b't', b'e', b'x', b't', 0x60]);
    assert!(matches!(
        ModelReply::decode(&duplicate, &Limits::default()),
        Err(ReplyError::Codec(
            CodecError::DuplicateKey | CodecError::NonCanonical
        ))
    ));
    let mut nonminimal = bytes.clone();
    let request = nonminimal.len() - 1;
    assert_eq!(nonminimal[request], 1);
    nonminimal.splice(request.., [0x18, 1]);
    assert_eq!(
        ModelReply::decode(&nonminimal, &Limits::default()).err(),
        Some(ReplyError::Codec(CodecError::NonCanonical))
    );
}

#[test]
fn exact_call_and_usage_membership_is_not_a_version_authority_or_cost_default() {
    for member in ["tool_version", "authority_binding", "extra"] {
        let mut bad = call("x", arguments());
        map_mut(&mut bad).insert(member.into(), Cbor::Null);
        assert_eq!(
            ModelReply::decode(&encode(&frame(1, vec![bad])), &Limits::default()).err(),
            Some(ReplyError::InvalidSchema)
        );
    }
    let mut source = frame(1, vec![call("x", arguments())]);
    map_mut(map_mut(&mut source).get_mut("usage").unwrap())
        .insert("billed_units".into(), Cbor::Text("0".into()));
    assert_eq!(
        ModelReply::decode(&encode(&source), &Limits::default()).err(),
        Some(ReplyError::Usage(UsageError::InvalidSchema))
    );
    let mut wrong = frame(1, vec![]);
    map_mut(&mut wrong).insert("request_id".into(), Cbor::Text("1".into()));
    assert_eq!(
        ModelReply::decode(&encode(&wrong), &Limits::default()).err(),
        Some(ReplyError::InvalidSchema)
    );
    let mut invalid_utf = encode(&frame(1, vec![call("é", arguments())]));
    let offset = invalid_utf
        .windows(2)
        .position(|b| b == [0xc3, 0xa9])
        .unwrap();
    invalid_utf[offset + 1] = 0x28;
    assert!(ModelReply::decode(&invalid_utf, &Limits::default()).is_err());
}
