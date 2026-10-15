use super::*;

pub(crate) mod activation;

use ciborium::value::{CanonicalValue, Value};
use std::{collections::BTreeMap, fs};

fn encode(value: &Value) -> Vec<u8> {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Map(fields) => {
                let mut fields = fields
                    .iter()
                    .map(|(k, v)| (canonical(k), canonical(v)))
                    .collect::<Vec<_>>();
                fields.sort_by(|(a, _), (b, _)| {
                    CanonicalValue::from(a.clone()).cmp(&CanonicalValue::from(b.clone()))
                });
                Value::Map(fields)
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&canonical(value), &mut bytes).unwrap();
    bytes
}

fn decode(bytes: &[u8]) -> Value {
    ciborium::de::from_reader(bytes).unwrap()
}

fn map(fields: Vec<(&str, Value)>) -> Value {
    Value::Map(
        fields
            .into_iter()
            .map(|(k, v)| (Value::Text(k.into()), v))
            .collect(),
    )
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    let Value::Map(fields) = value else {
        panic!("map")
    };
    &fields
        .iter()
        .find(|(k, _)| k == &Value::Text(key.into()))
        .unwrap()
        .1
}

fn frame(profile: usize) -> Vec<u8> {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../../fixtures/format/genesis.json")).unwrap();
    let blocks = fixtures[profile]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let h = b["hex"].as_str().unwrap();
            let bytes = (0..h.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            (b["kind"].as_str().unwrap(), bytes)
        })
        .collect::<BTreeMap<_, _>>();
    encode(&map(vec![
        ("abi_version", Value::Integer(1.into())),
        ("type", Value::Text("initialize".into())),
        (
            "payload",
            map(vec![
                ("agent_id", Value::Bytes(vec![0xb0; 32])),
                ("definition", Value::Bytes(blocks["definition"].clone())),
                (
                    "memory",
                    blocks
                        .get("memory")
                        .map(|b| Value::Bytes(b.clone()))
                        .unwrap_or(Value::Null),
                ),
            ]),
        ),
    ]))
}

fn wasm() -> Vec<u8> {
    fs::read(
        std::env::var("RAIV_GUEST_WASM")
            .expect("set RAIV_GUEST_WASM to the compiled Rust wasm32v1-none module"),
    )
    .unwrap()
}

#[test]
fn native_bootstrap_errors_are_atomic_and_initialize_requires_fresh_guest() {
    for profile in [0, 1] {
        let mut guest = guest::Guest::new();
        let valid = frame(profile);
        for invalid in [&[0xf6][..], &valid[..valid.len() - 1]] {
            let error = guest.step(invalid).unwrap();
            assert_eq!(field(&decode(&error), "type"), &Value::Text("error".into()));
        }
        let output = guest.step(&valid).unwrap();
        assert_eq!(
            field(&decode(&output), "type"),
            &Value::Text("initialized".into())
        );
        let error = guest.step(&valid).unwrap();
        assert_eq!(
            field(field(&decode(&error), "payload"), "code"),
            &Value::Text("already_initialized".into())
        );
    }
}

#[test]
fn native_transport_rejects_versions_duplicates_trailing_unknown_shapes_and_oversized_data() {
    let original = decode(&frame(0));
    let Value::Map(fields) = original.clone() else {
        panic!()
    };
    let mut cases = vec![vec![0xff], encode(&Value::Array(vec![]))];
    let mut trailing = frame(0);
    trailing.push(0);
    cases.push(trailing);
    let mut duplicate = fields.clone();
    duplicate.push((Value::Text("type".into()), Value::Text("initialize".into())));
    cases.push(encode(&Value::Map(duplicate)));
    for (key, value) in [
        ("abi_version", Value::Integer(2.into())),
        ("type", Value::Text("activate".into())),
        ("payload", Value::Map(vec![])),
    ] {
        let changed = Value::Map(
            fields
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if k == &Value::Text(key.into()) {
                            value.clone()
                        } else {
                            v.clone()
                        },
                    )
                })
                .collect(),
        );
        cases.push(encode(&changed));
    }
    for invalid in cases {
        let mut guest = guest::Guest::new();
        let output = guest.step(&invalid).unwrap();
        assert_eq!(
            field(&decode(&output), "type"),
            &Value::Text("error".into())
        );
        assert_eq!(
            field(&decode(&guest.step(&frame(0)).unwrap()), "type"),
            &Value::Text("initialized".into())
        );
    }
    let mut guest = guest::Guest::new();
    let invalid = vec![0; MAX_FRAME_BYTES + 1];
    assert_eq!(
        field(&decode(&guest.step(&invalid).unwrap()), "type"),
        &Value::Text("error".into())
    );
}

#[test]
fn transport_depth_and_aggregate_entries_are_bounded_before_payload_ownership() {
    let mut deep = vec![0x81; 65];
    deep.push(0);
    let mut guest = guest::Guest::new();
    assert_eq!(
        field(&decode(&guest.step(&deep).unwrap()), "type"),
        &Value::Text("error".into())
    );
    // Every individual array fits, but their aggregate plus envelope fields does not.
    let array = Value::Array(vec![Value::Null; 32_768]);
    let frame = encode(&map(vec![
        ("abi_version", Value::Integer(1.into())),
        ("type", Value::Text("initialize".into())),
        ("payload", map(vec![("a", array.clone()), ("b", array)])),
    ]));
    let error = decode(&guest.step(&frame).unwrap());
    assert_eq!(
        field(field(&error, "payload"), "code"),
        &Value::Text("abi_frame_limit".into())
    );
}

#[test]
fn host_range_checks_refuse_null_overflow_out_of_bounds_and_invalid_limits() {
    for (ptr, len, size) in [(0, 1, 100), (u32::MAX, 2, usize::MAX), (99, 2, 100)] {
        assert!(checked_range(ptr, len, size).is_err());
    }
    assert_eq!(checked_range(1, 99, 100).unwrap(), 1..100);
    assert!(matches!(
        GuestVm::new(
            &[],
            VmLimits {
                memory_bytes: 0,
                ..VmLimits::default()
            }
        ),
        Err(BridgeError::InvalidLimits)
    ));
}

#[test]
fn vm_refuses_imported_capabilities_without_linking_wasi_or_host_functions() {
    let imported = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x02\x0a\x01\x04host\x01f\0\0";
    assert!(matches!(
        GuestVm::new(imported, VmLimits::default()),
        Err(BridgeError::ForbiddenImports)
    ));
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn rust_wasm_bootstrap_equals_native_guest_and_exact_genesis_for_both_memory_profiles() {
    let module = wasm();
    for profile in [0, 1] {
        let input = frame(profile);
        let mut native = guest::Guest::new();
        let expected = native.step(&input).unwrap();
        let mut vm = GuestVm::new(&module, VmLimits::default()).unwrap();
        let output = vm.round_trip(&input).unwrap();
        assert_eq!(output, expected);
        let decoded = decode(&output);
        let payload = field(&decoded, "payload");
        let root = field(payload, "root").clone();
        let Value::Bytes(root) = root else { panic!() };
        assert_eq!(root.len(), 36);
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../../fixtures/format/genesis.json")).unwrap();
        let Value::Array(blocks) = field(payload, "blocks") else {
            panic!()
        };
        for expected in fixtures[profile]["blocks"].as_array().unwrap() {
            let h = expected["hex"].as_str().unwrap();
            let body = (0..h.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            let block = blocks
                .iter()
                .find(|b| field(b, "body") == &Value::Bytes(body.clone()))
                .expect("exact golden genesis block");
            if expected["kind"] == "agent_state" {
                assert_eq!(field(block, "cid"), &Value::Bytes(root.clone()));
            }
        }
        let output = vm.round_trip(&input).unwrap();
        assert_eq!(
            field(field(&decode(&output), "payload"), "code"),
            &Value::Text("already_initialized".into())
        );
    }
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_guest_ownership_rejects_foreign_and_freed_input_and_preserves_output_across_input_free() {
    let mut vm = GuestVm::new(&wasm(), VmLimits::default()).unwrap();
    vm.store.set_fuel(vm.limits.fuel_per_step).unwrap();
    let ptr = vm.alloc.call(&mut vm.store, 1).unwrap();
    assert_ne!(ptr, 0);
    assert_eq!(vm.alloc.call(&mut vm.store, 1).unwrap(), 0);
    vm.dealloc.call(&mut vm.store, (ptr + 1, 1)).unwrap();
    assert_eq!(vm.alloc.call(&mut vm.store, 1).unwrap(), 0);
    vm.dealloc.call(&mut vm.store, (ptr, 2)).unwrap();
    assert_eq!(vm.alloc.call(&mut vm.store, 1).unwrap(), 0);
    let packed = vm.step.call(&mut vm.store, (ptr + 1, 1)).unwrap();
    assert_ne!(packed, 0);
    let (out, len) = ((packed >> 32) as u32, packed as u32);
    let mut bytes = vec![0; len as usize];
    vm.memory.read(&vm.store, out as usize, &mut bytes).unwrap();
    assert_eq!(field(&decode(&bytes), "type"), &Value::Text("error".into()));
    vm.dealloc.call(&mut vm.store, (out, len)).unwrap(); // Output is never an input allocation.
    assert_eq!(vm.alloc.call(&mut vm.store, 1).unwrap(), 0);
    vm.dealloc.call(&mut vm.store, (ptr, 1)).unwrap();
    let mut same = vec![0; len as usize];
    vm.memory.read(&vm.store, out as usize, &mut same).unwrap();
    assert_eq!(bytes, same);
    assert_ne!(vm.step.call(&mut vm.store, (ptr, 1)).unwrap(), 0); // Safe error, never a freed-pointer dereference.
    assert_eq!(
        field(&decode(&vm.round_trip(&frame(0)).unwrap()), "type"),
        &Value::Text("initialized".into())
    );
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_wasm_abi_errors_do_not_advance_bootstrap() {
    let mut vm = GuestVm::new(&wasm(), VmLimits::default()).unwrap();
    for invalid in [vec![0xff], frame(0)[..10].to_vec()] {
        assert_eq!(
            field(&decode(&vm.round_trip(&invalid).unwrap()), "type"),
            &Value::Text("error".into())
        );
    }
    assert_eq!(
        field(&decode(&vm.round_trip(&frame(0)).unwrap()), "type"),
        &Value::Text("initialized".into())
    );
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_vm_fuel_memory_and_allocation_exhaustion_never_produce_success_or_reusable_failed_vm() {
    let module = wasm();
    let mut vm = GuestVm::new(
        &module,
        VmLimits {
            fuel_per_step: 1,
            ..VmLimits::default()
        },
    )
    .unwrap();
    assert!(vm.round_trip(&frame(0)).is_err());
    assert!(matches!(vm.round_trip(&frame(0)), Err(BridgeError::Dead)));
    assert!(GuestVm::new(
        &module,
        VmLimits {
            memory_bytes: 1,
            ..VmLimits::default()
        }
    )
    .is_err());
    let probe = GuestVm::new(&module, VmLimits::default()).unwrap();
    let minimum = probe.memory.data_size(&probe.store);
    let mut vm = GuestVm::new(
        &module,
        VmLimits {
            memory_bytes: minimum,
            ..VmLimits::default()
        },
    )
    .unwrap();
    assert!(vm.round_trip(&vec![0; MAX_FRAME_BYTES]).is_err());
    assert!(matches!(vm.round_trip(&frame(0)), Err(BridgeError::Dead)));
}
