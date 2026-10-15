//! Actual guest yield/error frames; no host decisions or acceptance simulation.

use super::*;

mod acceptance;
mod action_result;
pub(crate) mod admission;
mod candidate;
#[cfg(target_os = "linux")]
mod checkpoint;
#[cfg(target_os = "linux")]
mod disk;
#[cfg(target_os = "linux")]
mod launch;
#[cfg(target_os = "linux")]
mod local_acceptance;
mod organic_patch;
pub(crate) mod output;
#[cfg(target_os = "linux")]
mod policy;
mod progression;
#[cfg(target_os = "linux")]
mod recovery;
mod registry;
#[cfg(target_os = "linux")]
mod reservation;
mod routing;
#[cfg(target_os = "linux")]
mod simulator;
mod store;
#[cfg(target_os = "linux")]
mod world;

fn set(value: &mut Value, key: &str, replacement: Value) {
    let Value::Map(fields) = value else { panic!() };
    fields
        .iter_mut()
        .find(|(k, _)| k == &Value::Text(key.into()))
        .unwrap()
        .1 = replacement;
}

fn fixture(value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(v) => Value::Bool(*v),
        serde_json::Value::Number(v) => Value::Integer(v.as_u64().unwrap().into()),
        serde_json::Value::String(v) => Value::Text(v.clone()),
        serde_json::Value::Array(v) => Value::Array(v.iter().map(fixture).collect()),
        serde_json::Value::Object(v) if v.len() == 1 && v.contains_key("$bytes") => {
            let h = v["$bytes"].as_str().unwrap();
            Value::Bytes(
                (0..h.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
                    .collect(),
            )
        }
        serde_json::Value::Object(v) => Value::Map(
            v.iter()
                .map(|(k, v)| (Value::Text(k.clone()), fixture(v)))
                .collect(),
        ),
    }
}

fn packet() -> Value {
    let inputs: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../fixtures/format/input.json")).unwrap();
    let mut input = inputs[0]["record"].clone();
    input["budget_admission"]["activation_units"] = 100.into();
    input["budget_admission"]["remaining_episode_units"] = 100.into();
    fixture(&input)
}

fn envelope(kind: &str, payload: Value) -> Vec<u8> {
    encode(&map(vec![
        ("abi_version", Value::Integer(1.into())),
        ("type", Value::Text(kind.into())),
        ("payload", payload),
    ]))
}

fn genesis(profile: usize) -> Value {
    let mut g = guest::Guest::new();
    decode(&g.step(&frame(profile)).unwrap())
}

fn activate(data: &Value, input: &Value) -> Vec<u8> {
    let data = field(data, "payload");
    envelope(
        "activate",
        map(vec![
            ("root", field(data, "root").clone()),
            ("blocks", field(data, "blocks").clone()),
            ("input", Value::Bytes(encode(input))),
        ]),
    )
}

fn error(id: u64) -> Vec<u8> {
    envelope(
        "inference_error",
        map(vec![
            ("request_id", Value::Integer(id.into())),
            ("code", Value::Text("SYNTHETIC_PROVIDER_FAILURE".into())),
        ]),
    )
}

fn code(output: &[u8]) -> String {
    let output = decode(output);
    let Value::Text(code) = field(field(&output, "payload"), "code") else {
        panic!()
    };
    code.clone()
}

fn verify_infer(output: &[u8], profile: usize) {
    let response = decode(output);
    assert_eq!(field(&response, "type"), &Value::Text("infer".into()));
    let payload = field(&response, "payload");
    assert_eq!(field(payload, "request_id"), &Value::Integer(1.into()));
    let request = field(payload, "request");
    assert_eq!(
        field(request, "projection_version"),
        &Value::Integer(1.into())
    );
    let Value::Bytes(context) = field(request, "context") else {
        panic!()
    };
    let Value::Array(sections) = decode(context) else {
        panic!()
    };
    assert_eq!(sections.len(), 7);
    let definitions: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/format/definition.json"
    ))
    .unwrap();
    let expected = &definitions[profile]["record"];
    assert_eq!(
        field(request, "model_parameters"),
        &fixture(&expected["inference_profile"])
    );
    assert_eq!(
        encode(field(request, "limits")),
        encode(&fixture(&expected["limits"]))
    );
    assert_eq!(
        encode(field(request, "tools")),
        encode(&fixture(&expected["tools"]))
    );
    assert_eq!(
        field(&sections[0], "mandate"),
        &Value::Text(expected["mandate"].as_str().unwrap().into())
    );
    assert_eq!(field(&sections[6], "entries"), &Value::Array(vec![]));
    // Ciborium independently reconstructs every complete canonical response byte.
    assert_eq!(encode(&response), output);
}

#[test]
fn native_activate_yields_exact_enabled_request_then_stale_errors_preserve_pending_and_matching_error_is_terminal(
) {
    for profile in [0, 1] {
        let input = activate(&genesis(profile), &packet());
        let mut guest = guest::Guest::new();
        verify_infer(&guest.step(&input).unwrap(), profile);
        assert_eq!(code(&guest.step(&input).unwrap()), "guest_instance_busy");
        assert_eq!(
            code(&guest.step(&frame(profile)).unwrap()),
            "already_initialized"
        );
        for invalid in [
            error(0),
            error(u64::MAX),
            vec![0xff],
            error(1)[..9].to_vec(),
            envelope("inference_reply", map(vec![])),
        ] {
            let response = guest.step(&invalid).unwrap();
            assert_eq!(
                field(&decode(&response), "type"),
                &Value::Text("error".into())
            );
        }
        assert_eq!(code(&guest.step(&error(1)).unwrap()), "inference_failed");
        assert_eq!(
            code(&guest.step(&error(1)).unwrap()),
            "wrong_inference_request"
        );
        assert_eq!(code(&guest.step(&input).unwrap()), "guest_instance_busy");
    }
}

#[test]
fn malformed_missing_corrupt_duplicate_blocks_and_unfunded_packets_never_spend_first_request_or_fallback_to_genesis(
) {
    for profile in [0, 1] {
        let original = decode(&activate(&genesis(profile), &packet()));
        let Value::Array(blocks) = field(field(&original, "payload"), "blocks") else {
            panic!()
        };
        let mut cases = Vec::new();
        for index in 0..blocks.len() {
            let mut bad = original.clone();
            let mut payload = field(&bad, "payload").clone();
            let mut subset = blocks.clone();
            subset.remove(index);
            set(&mut payload, "blocks", Value::Array(subset));
            set(&mut bad, "payload", payload);
            cases.push(encode(&bad));
        }
        for duplicate in [true, false] {
            let mut bad = original.clone();
            let mut payload = field(&bad, "payload").clone();
            let mut subset = blocks.clone();
            if duplicate {
                subset.push(subset[0].clone());
            } else {
                set(&mut subset[0], "body", Value::Bytes(vec![0xa0]));
            }
            set(&mut payload, "blocks", Value::Array(subset));
            set(&mut bad, "payload", payload);
            cases.push(encode(&bad));
        }
        let mut input = packet();
        let mut budget = field(&input, "budget_admission").clone();
        set(&mut budget, "activation_units", Value::Integer(1.into()));
        set(&mut input, "budget_admission", budget);
        cases.push(activate(&genesis(profile), &input));
        for invalid in cases {
            let mut guest = guest::Guest::new();
            assert_eq!(
                field(&decode(&guest.step(&invalid).unwrap()), "type"),
                &Value::Text("error".into())
            );
            verify_infer(&guest.step(&encode(&original)).unwrap(), profile);
        }
    }
}

#[test]
fn compiled_registration_and_scripted_model_profile_mismatches_fail_before_inference() {
    for change_model in [true, false] {
        let mut initialize = decode(&frame(0));
        let mut payload = field(&initialize, "payload").clone();
        let Value::Bytes(bytes) = field(&payload, "definition") else {
            panic!()
        };
        let mut definition = decode(bytes);
        if change_model {
            set(
                &mut definition,
                "inference_profile",
                map(vec![("model", Value::Text("unapproved-live-model".into()))]),
            );
        } else {
            let mut tools = field(&definition, "tools").clone();
            let Value::Array(entries) = &mut tools else {
                panic!()
            };
            set(&mut entries[0], "version", Value::Integer(2.into()));
            set(&mut definition, "tools", tools);
        }
        set(
            &mut payload,
            "definition",
            Value::Bytes(encode(&definition)),
        );
        set(&mut initialize, "payload", payload);
        let mut bootstrap = guest::Guest::new();
        let data = decode(&bootstrap.step(&encode(&initialize)).unwrap());
        assert_eq!(field(&data, "type"), &Value::Text("initialized".into()));
        let mut guest = guest::Guest::new();
        assert_eq!(
            code(&guest.step(&activate(&data, &packet())).unwrap()),
            if change_model {
                "unsupported_scripted_profile"
            } else {
                "compiled_registry_mismatch"
            }
        );
        verify_infer(&guest.step(&activate(&genesis(0), &packet())).unwrap(), 0);
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_wasm_activate_retains_live_request_across_wrong_errors_and_matches_native_until_terminal_failure(
) {
    for profile in [0, 1] {
        let input = activate(&genesis(profile), &packet());
        let mut native = guest::Guest::new();
        let mut vm = GuestVm::new(&wasm(), VmLimits::default()).unwrap();
        for input in [
            input.clone(),
            input.clone(),
            frame(profile),
            error(0),
            error(u64::MAX),
            vec![0xff],
            error(1),
            error(1),
            input,
        ] {
            let expected = native.step(&input).unwrap();
            let actual = vm.round_trip(&input).unwrap();
            assert_eq!(actual, expected);
            if field(&decode(&actual), "type") == &Value::Text("infer".into()) {
                verify_infer(&actual, profile);
            }
        }
    }
}
