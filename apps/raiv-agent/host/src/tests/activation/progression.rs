//! Retained RAM and fixed checkpoint DATA in the guest, never canonical host acceptance.
use super::*;
fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
pub(super) fn call(tool: &str, args: Value) -> Value {
    map(vec![
        ("call_id", Value::Text("GUEST_RAM_ONLY_NATIVE_ID".into())),
        ("tool_id", Value::Text(tool.into())),
        ("arguments", args),
    ])
}
pub(super) fn reply(outer: u64, body: u64, calls: Vec<Value>) -> Vec<u8> {
    envelope(
        "inference_reply",
        map(vec![
            ("request_id", Value::Integer(outer.into())),
            (
                "reply",
                map(vec![
                    ("request_id", Value::Integer(body.into())),
                    ("text", Value::Text("GUEST_RAM_ONLY_REPLY_TEXT".into())),
                    ("calls", Value::Array(calls)),
                    (
                        "usage",
                        map(vec![
                            ("model_calls", Value::Integer(0.into())),
                            ("local_tool_calls", Value::Integer(0.into())),
                            ("input_tokens", Value::Null),
                            ("output_tokens", Value::Null),
                            ("billed_units", Value::Null),
                        ]),
                    ),
                ]),
            ),
        ]),
    )
}
fn sections(response: &[u8]) -> Vec<Value> {
    let decoded = decode(response);
    let Value::Bytes(bytes) = field(field(field(&decoded, "payload"), "request"), "context") else {
        panic!()
    };
    let Value::Array(sections) = decode(bytes) else {
        panic!()
    };
    sections
}
fn infer_id(response: &[u8]) -> Value {
    field(field(&decode(response), "payload"), "request_id").clone()
}
fn candidate(response: &[u8]) -> Value {
    let decoded = decode(response);
    assert_eq!(field(&decoded, "type"), &Value::Text("boundary".into()));
    assert_eq!(encode(&decoded), response);
    field(field(&decoded, "payload"), "candidate").clone()
}
fn blocks(candidate: &Value) -> Vec<Value> {
    let Value::Array(blocks) = field(candidate, "new_blocks") else {
        panic!()
    };
    blocks.clone()
}
fn tag_cid(value: &Value) -> &[u8] {
    let Value::Tag(42, payload) = value else {
        panic!()
    };
    let Value::Bytes(bytes) = payload.as_ref() else {
        panic!()
    };
    assert_eq!(bytes[0], 0);
    &bytes[1..]
}
fn body(block: &Value) -> Value {
    let Value::Bytes(bytes) = field(block, "body") else {
        panic!()
    };
    decode(bytes)
}
fn usage(candidate: &Value) -> Value {
    let boundary = blocks(candidate)
        .into_iter()
        .find(|b| field(&body(b), "kind") == &Value::Text("boundary".into()))
        .unwrap();
    field(&body(&boundary), "usage").clone()
}
fn lifecycle(mut step: impl FnMut(&[u8]) -> Vec<u8>) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../fixtures/format/lifecycle.json"
    ))
    .unwrap();
    let mut data = genesis(1);
    for (index, witness) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let input = decode(&hex(witness["input"]["hex"].as_str().unwrap()));
        let initial = step(&activate(&data, &input));
        assert_eq!(infer_id(&initial), Value::Integer(1.into()));
        let before = sections(&initial);
        let patch = call("patch_memory", fixture_value(&witness["patch"]));
        assert_eq!(
            code(&step(&reply(2, 1, vec![patch.clone()]))),
            "wrong_inference_request"
        );
        assert_eq!(
            code(&step(&reply(1, 2, vec![patch.clone()]))),
            "wrong_inference_request"
        );
        let next = step(&reply(1, 1, vec![patch.clone()]));
        assert_eq!(infer_id(&next), Value::Integer(2.into()));
        let after = sections(&next);
        assert_eq!(&after[..6], &before[..6]);
        let Value::Array(turns) = field(&after[6], "entries") else {
            panic!()
        };
        assert_eq!(turns.len(), 1);
        let Value::Array(results) = field(&turns[0], "results") else {
            panic!()
        };
        assert_eq!(field(&results[0], "ok"), &Value::Bool(true));
        let report = field(&results[0], "result");
        assert_eq!(
            encode(report),
            encode(&fixture_value(&witness["patch_result"]["value"]))
        );
        assert_eq!(
            encode(report),
            hex(witness["patch_result"]["hex"].as_str().unwrap())
        );
        assert_eq!(
            code(&step(&reply(1, 1, vec![patch]))),
            "wrong_inference_request"
        );
        let final_reply = reply(
            2,
            2,
            witness["tool_id"]
                .as_str()
                .map(|tool| vec![call(tool, fixture_value(&witness["arguments"]))])
                .unwrap_or_default(),
        );
        let output = step(&final_reply);
        let sealed = candidate(&output);
        // Independent Python golden root, never derived from guest output.
        let root = witness["state"]["cid"]
            .as_str()
            .unwrap()
            .parse::<raiv::format::DaslCid>()
            .unwrap();
        assert_eq!(
            field(&sealed, "new_root"),
            &Value::Bytes(root.as_bytes().to_vec())
        );
        assert_eq!(
            field(&sealed, "expected_root"),
            field(field(&data, "payload"), "root")
        );
        assert_eq!(
            field(&sealed, "expected_revision"),
            &Value::Integer((index as u64).into())
        );
        assert_eq!(field(&sealed, "agent_id"), &Value::Bytes(vec![0xb0; 32]));
        assert_eq!(
            field(&sealed, "admitted_event_id"),
            field(&input, "event_id")
        );
        let golden_boundary = decode(&hex(witness["boundary"]["hex"].as_str().unwrap()));
        assert_eq!(
            field(&sealed, "admitted_input_digest"),
            &Value::Bytes(tag_cid(field(&golden_boundary, "input"))[4..].to_vec())
        );
        let delta = blocks(&sealed);
        let actual_cids = delta
            .iter()
            .map(|b| field(b, "cid").clone())
            .collect::<Vec<_>>();
        let mut sorted = actual_cids.clone();
        sorted.sort_by_key(encode);
        assert_eq!(actual_cids, sorted);
        assert_eq!(code(&step(&final_reply)), "wrong_inference_request");
        assert_eq!(code(&step(&activate(&data, &input))), "guest_instance_busy");
        let mut stored = field(field(&data, "payload"), "blocks").clone();
        let Value::Array(all) = &mut stored else {
            panic!()
        };
        for emitted in delta {
            let Value::Bytes(bytes) = field(&emitted, "body") else {
                panic!()
            };
            assert!(["input", "memory", "boundary", "episode", "state"]
                .iter()
                .any(|key| hex(witness[key]["hex"].as_str().unwrap()) == *bytes));
            for sentinel in [
                b"GUEST_RAM_ONLY_NATIVE_ID".as_slice(),
                b"GUEST_RAM_ONLY_REPLY_TEXT",
                b"ram_trajectory",
                b"previous_root",
            ] {
                assert!(!bytes.windows(sentinel.len()).any(|part| part == sentinel));
            }
            if let Some(existing) = all
                .iter()
                .find(|b| field(b, "cid") == field(&emitted, "cid"))
            {
                assert_eq!(existing, &emitted);
            } else {
                all.push(emitted);
            }
        }
        for key in ["input", "memory", "boundary", "episode", "state"] {
            let expected = Value::Bytes(hex(witness[key]["hex"].as_str().unwrap()));
            assert!(all.iter().any(|b| field(b, "body") == &expected));
        }
        data = decode(&envelope(
            "initialized",
            map(vec![
                ("root", field(&sealed, "new_root").clone()),
                ("blocks", stored),
            ]),
        ));
        // Mark the next test step as a fresh VM; no host acceptance/effects are performed.
        step(&[]);
    }
}
fn fixture_value(value: &serde_json::Value) -> Value {
    fixture(value)
}
#[test]
fn native_retained_patch_yield_finish_and_fresh_instance_recovery_match_six_independent_checkpoint_witnesses(
) {
    let mut guest = guest::Guest::new();
    lifecycle(|input| {
        if input.is_empty() {
            guest = guest::Guest::new();
            Vec::new()
        } else {
            guest.step(input).unwrap()
        }
    });
}
fn correction_and_finish(mut step: impl FnMut(&[u8]) -> Vec<u8>) {
    let initial = step(&activate(&genesis(1), &packet()));
    let prefix = sections(&initial);
    let multi = reply(
        1,
        1,
        vec![
            call(
                "patch_memory",
                map(vec![(
                    "intents",
                    map(vec![("must_not_apply", Value::Bool(true))]),
                )]),
            ),
            call("trade", map(vec![])),
        ],
    );
    let next = step(&multi);
    assert_eq!(infer_id(&next), Value::Integer(2.into()));
    assert_eq!(&sections(&next)[..6], &prefix[..6]);
    let sealed = candidate(&step(&reply(2, 2, vec![])));
    assert_eq!(
        field(field(&sealed, "disposition"), "tag"),
        &Value::Text("await_actor".into())
    );
    assert_eq!(
        field(&usage(&sealed), "model_calls"),
        &Value::Integer(2.into())
    );
    assert_eq!(
        field(&usage(&sealed), "local_tool_calls"),
        &Value::Integer(0.into())
    );
    assert!(blocks(&sealed)
        .iter()
        .all(|b| field(&body(b), "kind") != &Value::Text("memory".into())));
}
#[test]
fn all_multiple_calls_correct_before_memory_or_finish_and_explicit_finish_reuses_surviving_memory()
{
    let mut guest = guest::Guest::new();
    correction_and_finish(|input| guest.step(input).unwrap());
}
#[test]
fn no_effect_patch_spends_work_and_finish_preserves_memory_while_disabled_profile_has_no_memory() {
    for profile in [0, 1] {
        let mut guest = guest::Guest::new();
        let initial = guest.step(&activate(&genesis(profile), &packet())).unwrap();
        let mut id = 1;
        if profile == 1 {
            let patch = call(
                "patch_memory",
                map(vec![("intents", map(vec![("retain", Value::Bool(false))]))]),
            );
            let next = guest.step(&reply(id, id, vec![patch])).unwrap();
            assert_eq!(infer_id(&next), Value::Integer(2.into()));
            id = 2;
            let next = guest
                .step(&reply(id, id, vec![call("patch_memory", map(vec![]))]))
                .unwrap();
            assert_eq!(infer_id(&next), Value::Integer(3.into()));
            id = 3;
        } else {
            assert_eq!(infer_id(&initial), Value::Integer(1.into()));
        }
        let sealed = candidate(&guest.step(&reply(id, id, vec![])).unwrap());
        assert_eq!(
            field(&usage(&sealed), "model_calls"),
            &Value::Integer(id.into())
        );
        assert_eq!(
            field(&usage(&sealed), "local_tool_calls"),
            &Value::Integer((id - 1).into())
        );
        let memory = blocks(&sealed)
            .into_iter()
            .find(|b| field(&body(b), "kind") == &Value::Text("memory".into()));
        if profile == 1 {
            assert_eq!(
                encode(field(&body(&memory.unwrap()), "value")),
                encode(&map(vec![(
                    "intents",
                    map(vec![("retain", Value::Bool(false))])
                )]))
            );
        } else {
            assert!(memory.is_none());
        }
    }
}
#[test]
fn matching_malformed_normalized_reply_is_fatal_but_outer_malformed_frames_preserve_pending() {
    let mut guest = guest::Guest::new();
    guest.step(&activate(&genesis(0), &packet())).unwrap();
    assert_eq!(code(&guest.step(&[0xff]).unwrap()), "invalid_abi_frame");
    let invalid = envelope(
        "inference_reply",
        map(vec![
            ("request_id", Value::Integer(1.into())),
            ("reply", map(vec![])),
        ]),
    );
    assert_eq!(code(&guest.step(&invalid).unwrap()), "activation_failed");
    assert_eq!(
        code(&guest.step(&reply(1, 1, vec![])).unwrap()),
        "wrong_inference_request"
    );
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_wasm_retained_progression_and_data_recovery_equal_native_and_independent_golden_candidates()
{
    let module = wasm();
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    let mut native = guest::Guest::new();
    lifecycle(|input| {
        if input.is_empty() {
            vm = GuestVm::new(&module, VmLimits::default()).unwrap();
            native = guest::Guest::new();
            Vec::new()
        } else {
            let expected = native.step(input).unwrap();
            let actual = vm.round_trip(input).unwrap();
            assert_eq!(actual, expected);
            actual
        }
    });
    let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
    let mut native = guest::Guest::new();
    correction_and_finish(|input| {
        let expected = native.step(input).unwrap();
        let actual = vm.round_trip(input).unwrap();
        assert_eq!(actual, expected);
        actual
    });
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn mid_live_vm_fuel_or_allocation_failure_discards_instance_without_boundary() {
    for fuel in [true, false] {
        let mut vm = GuestVm::new(&wasm(), VmLimits::default()).unwrap();
        vm.round_trip(&activate(&genesis(1), &packet())).unwrap();
        let input = if fuel {
            vm.limits.fuel_per_step = 1;
            reply(1, 1, vec![])
        } else {
            let size = vm.memory.data_size(&vm.store);
            *vm.store.data_mut() = StoreLimitsBuilder::new().memory_size(size).build();
            vec![0; MAX_FRAME_BYTES]
        };
        assert!(vm.round_trip(&input).is_err());
        assert!(matches!(vm.round_trip(&error(1)), Err(BridgeError::Dead)));
    }
}
