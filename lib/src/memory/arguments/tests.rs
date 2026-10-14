use super::*;
use crate::{
    format::{oracle::json_bytes, CodecError},
    memory::tests::memory,
};
use alloc::{vec, vec::Vec};
use serde_json::json;

#[test]
fn borrowed_object_arguments_capture_limits_and_return_complete_memory() {
    let bytes = json_bytes(&json!({"strategy":{"rule":"retry"},"intents":{"done":null}}));
    let limits = Limits::default();
    let patch = PatchArguments::decode(&bytes, &limits).unwrap();
    assert_eq!(patch.as_bytes().as_ptr(), bytes.as_ptr());
    assert_eq!(patch.limits(), &limits);
    let mut working = memory(json!({"intents":{"done":1,"keep":2}}));
    let draft = patch.patched(&working).unwrap();
    assert_eq!(
        draft.value_bytes(),
        json_bytes(&json!({"intents":{"keep":2},"strategy":{"rule":"retry"}}))
    );
    assert_eq!(
        working.value_bytes(),
        json_bytes(&json!({"intents":{"done":1,"keep":2}}))
    );
    assert_eq!(patch.apply(&mut working).unwrap(), draft.value_bytes());
}
#[test]
fn duplicate_canonical_keys_are_not_sequential_writes_and_old_envelopes_are_unknown_planes() {
    // {tactics:{x:1,x:2}} keeps duplicate evidence in raw wire, never a lossy Map.
    let duplicate = b"\xa1\x67tactics\xa2\x61x\x01\x61x\x02";
    assert!(matches!(
        PatchArguments::decode(duplicate, &Limits::default()),
        Err(PatchError::Codec(CodecError::DuplicateKey))
    ));
    let bytes = json_bytes(&json!({"operations":[{"op":"set","path":["intents","x"],"value":1}]}));
    let patch = PatchArguments::decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(patch.processed_planes().count(), 0);
    assert_eq!(patch.skipped_planes().next().unwrap().name, "operations");
    let mut working = memory(json!({}));
    patch.apply(&mut working).unwrap();
    assert_eq!(working.value_bytes(), json_bytes(&json!({})));
    for bytes in [vec![0xa0, 0], vec![0xbf, 0xff], vec![0xa1, 0x01, 0]] {
        assert!(PatchArguments::decode(&bytes, &Limits::default()).is_err());
    }
}
#[test]
fn replacement_array_null_is_rejected_before_cleanup_could_hide_it() {
    for value in [
        json!([null]),
        json!([{"x":null}]),
        json!([{"x":[{"y":null}]}]),
        json!([[null]]),
    ] {
        let patch = json_bytes(&json!({"tactics":{"x":value}}));
        assert!(matches!(
            PatchArguments::decode(&patch, &Limits::default()),
            Err(PatchError::NullInArray)
        ));
    }
    assert!(PatchArguments::decode(
        &json_bytes(&json!({"tactics":{"x":{"y":null}}})),
        &Limits::default()
    )
    .is_ok());
}
#[test]
fn full_input_count_depth_and_byte_bounds_apply_before_small_output() {
    let patch = json_bytes(&json!({"tactics":{"x":{"y":null}}}));
    assert!(matches!(
        PatchArguments::decode(
            &patch,
            &Limits {
                max_patch_entries: 2,
                ..Limits::default()
            }
        ),
        Err(PatchError::EntryLimit)
    ));
    assert!(PatchArguments::decode(
        &patch,
        &Limits {
            max_patch_entries: 3,
            ..Limits::default()
        }
    )
    .is_ok());
    assert!(matches!(
        PatchArguments::decode(
            &patch,
            &Limits {
                max_model_reply_bytes: patch.len() as u64 - 1,
                ..Limits::default()
            }
        ),
        Err(PatchError::Codec(CodecError::ByteLimit))
    ));
    assert!(matches!(
        PatchArguments::decode(
            &patch,
            &Limits {
                max_depth: 2,
                ..Limits::default()
            }
        ),
        Err(PatchError::Codec(CodecError::DepthLimit))
    ));
    // Array members and map fields inside replacement data also count.
    let patch = json_bytes(&json!({"tactics":{"x":[{"empty":{}}]}}));
    assert!(matches!(
        PatchArguments::decode(
            &patch,
            &Limits {
                max_patch_entries: 3,
                ..Limits::default()
            }
        ),
        Err(PatchError::EntryLimit)
    ));
    let mut working = memory(json!({}));
    working
        .apply_patch(
            &patch,
            &Limits {
                max_patch_entries: 4,
                ..Limits::default()
            },
        )
        .unwrap();
    assert_eq!(
        working.value_bytes(),
        json_bytes(&json!({"tactics":{"x":[{}]}}))
    );
    assert!(PatchArguments::decode(
        &[0xa0],
        &Limits {
            max_patch_entries: 0,
            ..Limits::default()
        }
    )
    .is_ok());
}
#[test]
fn model_numeric_and_runtime_value_restrictions_apply_even_to_replacement_arrays() {
    for value in [
        json!(9007199254740992u64),
        json!(-9007199254740992i64),
        json!(1.5),
    ] {
        assert!(PatchArguments::decode(
            &json_bytes(&json!({"strategy":{"x":[value]}})),
            &Limits::default()
        )
        .is_err());
    }
    let patch = crate::format::oracle::fixture_bytes(&json!({"strategy":{"x":{"$bytes":"01"}}}));
    assert!(matches!(
        PatchArguments::decode(&patch, &Limits::default()),
        Err(PatchError::Codec(CodecError::UnsupportedValue))
    ));
}
#[test]
fn deep_array_merge_and_destruction_are_iterative_and_output_depth_is_checked_atomically() {
    let mut bytes: Vec<u8> = b"\xa1\x67tactics\xa1\x61x".to_vec();
    bytes.extend(core::iter::repeat_n(0x81, 9999));
    bytes.push(0);
    let limits = Limits {
        max_depth: 10003,
        max_patch_entries: 10001,
        max_memory_bytes: 65536,
        ..Limits::default()
    };
    let patch = PatchArguments::decode(&bytes, &limits).unwrap();
    let mut working = memory(json!({}));
    patch.apply(&mut working).unwrap();
    assert!(Memory::decode(working.as_bytes(), &limits.decode_limits()).is_ok());
    let mut working = memory(json!({}));
    let pointer = working.as_bytes().as_ptr();
    let patch = PatchArguments::decode(
        &bytes,
        &Limits {
            max_depth: 10002,
            ..limits
        },
    )
    .unwrap();
    assert_eq!(
        patch.apply(&mut working),
        Err(PatchError::Codec(CodecError::DepthLimit))
    );
    assert_eq!(working.value_bytes(), &[0xa0]);
    assert_eq!(working.as_bytes().as_ptr(), pointer);
}

#[test]
fn root_selection_never_remaps_synonyms_and_reports_complete_canonical_data() {
    let before = memory(json!({"strategy":{"keep":false}}));
    let bytes = json_bytes(
        &json!({"intent":{"wrong":1},"goals":[null,{"ignored":[null]}],"Tactics":42,"intents":{"next":"inspect"},"tactics":{"goals":{"literal":true}}}),
    );
    let patch = PatchArguments::decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(
        patch.processed_planes().collect::<Vec<_>>(),
        vec!["intents", "tactics"]
    );
    assert_eq!(
        patch
            .skipped_planes()
            .map(|s| (s.name, s.reason()))
            .collect::<Vec<_>>(),
        vec![
            ("goals", "unsupported_root"),
            ("intent", "unsupported_root"),
            ("Tactics", "unsupported_root")
        ]
    );
    let accepted = patch.patched(&before).unwrap();
    assert_eq!(
        accepted.value_bytes(),
        json_bytes(
            &json!({"strategy":{"keep":false},"intents":{"next":"inspect"},"tactics":{"goals":{"literal":true}}})
        )
    );
    let report = patch.report(&before, &accepted).unwrap();
    assert_eq!(
        report.as_bytes(),
        json_bytes(
            &json!({"memory":{"strategy":{"keep":false},"intents":{"next":"inspect"},"tactics":{"goals":{"literal":true}}},"changed":true,"processed_planes":["intents","tactics"],"skipped_planes":[{"plane":"goals","reason":"unsupported_root"},{"plane":"intent","reason":"unsupported_root"},{"plane":"Tactics","reason":"unsupported_root"}]})
        )
    );
    let no_change = patch.patched(&accepted).unwrap();
    let report = patch.report(&accepted, &no_change).unwrap();
    let oracle: ciborium::value::Value = ciborium::de::from_reader(report.as_bytes()).unwrap();
    let ciborium::value::Value::Map(entries) = oracle else {
        panic!()
    };
    assert!(entries.contains(&(
        ciborium::value::Value::Text("changed".into()),
        ciborium::value::Value::Bool(false)
    )));
}
#[test]
fn all_skipped_reuses_memory_but_ignored_payloads_cannot_escape_any_input_bound() {
    let bytes =
        json_bytes(&json!({"plans":{"nested":[null,{"x":null}]},"intent":{},"Strategy":false}));
    let patch = PatchArguments::decode(&bytes, &Limits::default()).unwrap();
    let mut live = memory(json!({"intents":{"keep":1}}));
    let cid = live.cid();
    let ptr = live.as_bytes().as_ptr();
    patch.apply(&mut live).unwrap();
    assert_eq!(live.cid(), cid);
    assert_eq!(live.as_bytes().as_ptr(), ptr);
    assert_eq!(patch.processed_planes().count(), 0);
    for limit in [
        Limits {
            max_patch_entries: 3,
            ..Limits::default()
        },
        Limits {
            max_model_reply_bytes: bytes.len() as u64 - 1,
            ..Limits::default()
        },
        Limits {
            max_depth: 2,
            ..Limits::default()
        },
    ] {
        assert!(PatchArguments::decode(&bytes, &limit).is_err());
    }
    assert!(matches!(
        PatchArguments::decode(b"\xa2\x65goals\xa0\x65goals\xa0", &Limits::default()),
        Err(PatchError::Codec(CodecError::DuplicateKey))
    ));
    for v in [
        json!({"goals":{},"tactics":[]}),
        json!({"goals":{},"intents":{"x":1},"tactics":{"bad":[null]}}),
    ] {
        assert!(live
            .apply_patch(&json_bytes(&v), &Limits::default())
            .is_err());
        assert_eq!(live.cid(), cid);
        assert_eq!(live.as_bytes().as_ptr(), ptr);
    }
}
#[test]
fn report_output_failure_is_bounded_staging_not_a_memory_assignment() {
    let bytes = json_bytes(&json!({"intents":{"x":2}}));
    let limits = Limits {
        max_context_bytes: bytes.len() as u64,
        ..Limits::default()
    };
    let patch = PatchArguments::decode(&bytes, &limits).unwrap();
    let before = memory(json!({"intents":{"x":1}}));
    let cid = before.cid();
    let ptr = before.as_bytes().as_ptr();
    let draft = patch.patched(&before).unwrap();
    let inbound = Limits {
        max_model_reply_bytes: bytes.len() as u64,
        ..Limits::default()
    };
    let permitted = PatchArguments::decode(&bytes, &inbound)
        .unwrap()
        .report(&before, &draft)
        .unwrap();
    assert!(permitted.as_bytes().len() as u64 > inbound.max_model_reply_bytes);
    assert!(matches!(
        patch.report(&before, &draft),
        Err(PatchError::Codec(CodecError::ByteLimit))
    ));
    for bounds in [
        Limits {
            max_block_bytes: draft.as_bytes().len() as u64,
            ..Limits::default()
        },
        Limits {
            max_collection_entries: 3,
            ..Limits::default()
        },
    ] {
        let staged = PatchArguments::decode(&bytes, &bounds).unwrap();
        assert!(staged.patched(&before).is_ok());
        assert!(staged.report(&before, &draft).is_err());
    }
    assert_eq!(before.cid(), cid);
    assert_eq!(before.as_bytes().as_ptr(), ptr);
}
