use super::*;
use crate::{format::oracle::json_bytes, memory::tests::memory};
use alloc::vec::Vec;
use serde_json::{json, Value};

fn apply(before: Value, patch: Value, expected: Value) {
    let mut working = memory(before);
    let bytes = json_bytes(&patch);
    assert_eq!(
        working.apply_patch(&bytes, &Limits::default()).unwrap(),
        json_bytes(&expected)
    );
    let once = working.as_bytes().to_vec();
    let cid = working.cid();
    let pointer = working.as_bytes().as_ptr();
    working.apply_patch(&bytes, &Limits::default()).unwrap();
    assert_eq!(working.as_bytes(), once);
    assert_eq!(working.cid(), cid);
    assert_eq!(working.as_bytes().as_ptr(), pointer);
}

#[test]
fn recursive_merge_preserves_neighbors_and_replaces_nonobjects_and_arrays() {
    apply(
        json!({"strategy":{"x":{"a":1,"b":2},"neighbor":3}}),
        json!({"strategy":{"x":{"a":4,"c":5}}}),
        json!({"strategy":{"x":{"a":4,"b":2,"c":5},"neighbor":3}}),
    );
    for old in [json!(1), json!(true), json!("old"), json!([1, 2])] {
        apply(
            json!({"tactics":{"x":old}}),
            json!({"tactics":{"x":{"new":3}}}),
            json!({"tactics":{"x":{"new":3}}}),
        );
    }
    for next in [json!(0), json!(false), json!(""), json!([]), json!([1, 2])] {
        apply(
            json!({"tactics":{"x":{"old":1}}}),
            json!({"tactics":{"x":next}}),
            json!({"tactics":{"x":next}}),
        );
    }
    apply(
        json!({"tactics":{"x":[{"old":1},2,3]}}),
        json!({"tactics":{"x":[{"new":2}]}}),
        json!({"tactics":{"x":[{"new":2}]}}),
    );
    apply(
        json!({"tactics":{"x":[1,2]}}),
        json!({"tactics":{"x":{"[0]":9}}}),
        json!({"tactics":{"x":{"[0]":9}}}),
    );
}

#[test]
fn null_removes_leaves_subtrees_and_planes_without_ancestors_or_cross_plane_cascades() {
    apply(
        json!({"strategy":{"x":{"a":1,"b":2},"keep":false}}),
        json!({"strategy":{"x":{"a":null}}}),
        json!({"strategy":{"x":{"b":2},"keep":false}}),
    );
    apply(
        json!({"strategy":{"x":{"a":1}}}),
        json!({"strategy":{"x":null}}),
        json!({}),
    );
    apply(
        json!({}),
        json!({"tactics":{"absent":{"x":null}}}),
        json!({}),
    );
    apply(
        json!({"intents":{"key1":1},"strategy":{"key1":2},"tactics":{"key1":3}}),
        json!({"intents":{"key1":null}}),
        json!({"strategy":{"key1":2},"tactics":{"key1":3}}),
    );
    apply(
        json!({"tactics":{"x":1},"strategy":{"x":2}}),
        json!({"tactics":null}),
        json!({"strategy":{"x":2}}),
    );
}

#[test]
fn empty_merge_is_not_clear_but_new_empty_objects_cascade_away() {
    apply(
        json!({"tactics":{"x":{"keep":1}}}),
        json!({"tactics":{}}),
        json!({"tactics":{"x":{"keep":1}}}),
    );
    apply(
        json!({"tactics":{"x":{"keep":1}}}),
        json!({"tactics":{"x":{}}}),
        json!({"tactics":{"x":{"keep":1}}}),
    );
    for old in [json!(1), json!([1]), json!(false)] {
        apply(
            json!({"tactics":{"x":old}}),
            json!({"tactics":{"x":{}}}),
            json!({}),
        );
    }
    apply(json!({}), json!({"tactics":{"x":{"y":{}}}}), json!({}));
    apply(
        json!({"strategy":{"x":{"y":1}}}),
        json!({"strategy":{"x":{"y":null}}}),
        json!({}),
    );
}

#[test]
fn array_objects_normalize_without_losing_positions_or_falsy_data() {
    apply(
        json!({}),
        json!({"tactics":{"steps":[{"x":{}}, {}, [{"a":{"b":{}}}]],"false":false,"zero":0,"text":"","array":[]}}),
        json!({"tactics":{"steps":[{}, {}, [{}]],"false":false,"zero":0,"text":"","array":[]}}),
    );
}

#[test]
fn literal_keys_never_become_paths_references_or_indices() {
    apply(
        json!({}),
        json!({"strategy":{"a.b":1,"a/b":2,"[0]":3,"$ref":"intents.key","":false,"ключ":0}}),
        json!({"strategy":{"a.b":1,"a/b":2,"[0]":3,"$ref":"intents.key","":false,"ключ":0}}),
    );
}

#[test]
fn map_union_preserves_canonical_order_across_utf8_and_text_header_boundaries() {
    let mut before = serde_json::Map::new();
    let mut patch = serde_json::Map::new();
    let mut expected = serde_json::Map::new();
    for (index, key) in [
        "".into(),
        "é".into(),
        "z".repeat(23),
        "a".repeat(24),
        "z".repeat(255),
        "a".repeat(256),
    ]
    .into_iter()
    .enumerate()
    {
        before.insert(key.clone(), json!(index));
        patch.insert(key.clone(), json!({"updated":index}));
        expected.insert(key, json!({"updated":index}));
    }
    apply(
        json!({"strategy":before}),
        json!({"strategy":patch}),
        json!({"strategy":expected}),
    );
}

#[test]
fn full_draft_cleanup_includes_unmentioned_branches() {
    // Exercise the private normalizer independently; persisted Memory cannot contain
    // these unnormalized fields, so do not bypass its public decoder to manufacture it.
    let record = json_bytes(
        &json!({"kind":"memory","version":1,"value":{"strategy":{"untouched":{"empty":{}},"keep":1},"tactics":{"arr":[{"x":{}}]}}}),
    );
    let bounds = DecodeLimits::default();
    crate::format::validate_model_value(&record, &bounds).unwrap();
    let output = merge_map_field(
        ValueView::preflighted(&record),
        "value",
        ValueView::preflighted(&[0xa0]),
        &bounds,
        super::super::plane,
    )
    .unwrap();
    let expected = memory(json!({"strategy":{"keep":1},"tactics":{"arr":[{}]}}));
    assert_eq!(output, expected.as_bytes());
    let twice = merge_map_field(
        ValueView::preflighted(&output),
        "value",
        ValueView::preflighted(&[0xa0]),
        &bounds,
        super::super::plane,
    )
    .unwrap();
    assert_eq!(output, twice);
}

#[test]
fn failures_preserve_original_payload_cid_and_pointer_at_every_stage() {
    let source = json!({"intents":{"keep":1}});
    for patch in [
        json!(null),
        json!([]),
        json!("{}"),
        json!({"tactics":[]}),
        json!({"strategy":1}),
        json!({"tactics":{"x":[null]}}),
        json!({"tactics":{"x":[{"x":null}]}}),
        json!({"tactics":{"x":[[{"x":[null]}]]}}),
    ] {
        let mut working = memory(source.clone());
        let bytes = working.as_bytes().to_vec();
        let cid = working.cid();
        let pointer = working.as_bytes().as_ptr();
        assert!(
            working
                .apply_patch(&json_bytes(&patch), &Limits::default())
                .is_err(),
            "{patch}"
        );
        assert_eq!(working.as_bytes(), bytes);
        assert_eq!(working.cid(), cid);
        assert_eq!(working.as_bytes().as_ptr(), pointer);
    }
    let mut working = memory(source);
    let bytes = working.as_bytes().to_vec();
    let pointer = working.as_bytes().as_ptr();
    let patch = json_bytes(&json!({"tactics":{"big":"x".repeat(500)}}));
    assert_eq!(
        working.apply_patch(
            &patch,
            &Limits {
                max_memory_bytes: bytes.len() as u64,
                ..Limits::default()
            }
        ),
        Err(PatchError::Codec(CodecError::ByteLimit))
    );
    assert_eq!(working.as_bytes(), bytes);
    assert_eq!(working.as_bytes().as_ptr(), pointer);
}

// Independent recursive JSON oracle is test-only; production uses borrowed flat spans.
fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for value in map.values_mut() {
                normalize(value);
            }
            map.retain(|_, v| !v.as_object().is_some_and(|m| m.is_empty()));
        }
        Value::Array(values) => {
            for value in values {
                normalize(value);
            }
        }
        _ => {}
    }
}

fn merge(base: &mut Value, patch: &Value) {
    if let Value::Object(fields) = patch {
        if !base.is_object() {
            *base = json!({});
        }
        let base = base.as_object_mut().unwrap();
        for (key, value) in fields {
            if value.is_null() {
                base.remove(key);
            } else if value.is_object() {
                merge(base.entry(key).or_insert(json!({})), value);
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = patch.clone();
    }
}

#[test]
fn deterministic_property_grid_matches_independent_merge_and_canonical_order() {
    let bases = [
        json!({}),
        json!({"x":1}),
        json!({"x":[1,2]}),
        json!({"x":{"a":1,"keep":2}}),
        json!({"a.b":false,"keep":[]}),
    ];
    let patches = [
        json!({}),
        json!({"x":null}),
        json!({"x":{}}),
        json!({"x":{"a":null,"b":3}}),
        json!({"x":false}),
        json!({"x":[{}, {"empty":{}}]}),
        json!({"a.b":0,"keep":[]}),
    ];
    for base in &bases {
        for patch in &patches {
            let before = if base.as_object().unwrap().is_empty() {
                json!({})
            } else {
                json!({"strategy":base,"intents":{"stable":1}})
            };
            let patch = json!({"strategy":patch});
            let mut expected = before.clone();
            merge(&mut expected, &patch);
            normalize(&mut expected);
            let mut twice = expected.clone();
            normalize(&mut twice);
            assert_eq!(twice, expected);
            apply(before, patch.clone(), expected.clone());
            let mut fields = Vec::new();
            for (key, value) in patch.as_object().unwrap().iter().rev() {
                fields.push((
                    ciborium::value::Value::Text(key.clone()),
                    ciborium::de::from_reader::<ciborium::value::Value, _>(
                        json_bytes(value).as_slice(),
                    )
                    .unwrap(),
                ));
            }
            let wire = crate::format::oracle::encode(&ciborium::value::Value::Map(fields));
            let mut working = memory(expected.clone());
            working.apply_patch(&wire, &Limits::default()).unwrap();
            assert_eq!(working.value_bytes(), json_bytes(&expected));
        }
    }
}

#[test]
fn non_object_planes_get_actionable_feedback_naming_the_expected_shape() {
    // Live-observed model mistakes: a plane given as text, then as an array.
    for bytes in [
        &b"\xa1\x67intents\x61x"[..],
        &b"\xa1\x67intents\x81\x61x"[..],
    ] {
        let Err(error) = PatchArguments::decode(bytes, &Limits::default()) else {
            panic!("non-object plane accepted");
        };
        assert_eq!(error, PatchError::PlaneNotMap);
        assert!(error.feedback().starts_with("invalid_patch: "));
        assert!(error.feedback().contains("object of named entries"));
    }
    for error in [
        PatchError::NullInArray,
        PatchError::InvalidArguments,
        PatchError::EntryLimit,
        PatchError::Codec(CodecError::ByteLimit),
    ] {
        assert!(error.feedback().starts_with("invalid_patch: ") && error.feedback().len() <= 256);
    }
}
