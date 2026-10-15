//! Independent CBOR schema/bounds mutations and actual bridge output rejection.

use super::*;
use crate::output::{DecodeError, Output};
use raiv::limits::Limits;

fn outputs() -> Vec<Vec<u8>> {
    let mut guest = guest::Guest::new();
    let data = guest.step(&frame(1)).unwrap();
    let infer = guest.step(&activate(&decode(&data), &packet())).unwrap();
    let boundary = guest.step(&progression::reply(1, 1, vec![])).unwrap();
    let error = guest.step(&error(1)).unwrap();
    vec![data, infer, boundary, error]
}

#[test]
fn closed_outputs_borrow_exact_source_spans_full_width_runtime_fields_and_cid_sorted_delta() {
    let frames = outputs();
    for bytes in &frames {
        Output::decode(bytes, &Limits::default()).unwrap();
    }
    let Output::Initialized { root, blocks } =
        Output::decode(&frames[0], &Limits::default()).unwrap()
    else {
        panic!()
    };
    assert!(!blocks.is_empty());
    assert!(blocks.iter().any(|b| b.cid == root));
    for block in blocks.iter() {
        let pointer = block.body.as_ptr() as usize;
        assert!(
            pointer >= frames[0].as_ptr() as usize
                && pointer + block.body.len() <= frames[0].as_ptr() as usize + frames[0].len()
        );
        block.cid.verify(block.body).unwrap();
    }
    let Output::Infer {
        request_id,
        request,
    } = Output::decode(&frames[1], &Limits::default()).unwrap()
    else {
        panic!()
    };
    assert_eq!(request_id, 1);
    assert_eq!(request.tools().count(), 3);
    assert_eq!(request.limits(), Limits::default());
    assert!(request.context_bytes().as_ptr() as usize >= frames[1].as_ptr() as usize);
    assert!(request.model_parameters_bytes().len() > 1);
    let Output::Boundary(candidate) = Output::decode(&frames[2], &Limits::default()).unwrap()
    else {
        panic!()
    };
    assert_eq!(candidate.expected_root(), root);
    assert_eq!(candidate.agent_id(), [0xb0; 32]);
    assert_eq!(candidate.expected_revision(), 0);
    assert_eq!(candidate.admitted_event_id(), [0x60; 32]);
    assert_eq!(candidate.admitted_input_digest().len(), 32);
    assert_eq!(
        candidate.disposition(),
        raiv::state::Continuation::AwaitActor
    );
    assert!(candidate
        .new_blocks()
        .iter()
        .any(|b| b.cid == candidate.new_root()));
    let mut wide = decode(&frames[2]);
    let mut payload = field(&wide, "payload").clone();
    let mut value = field(&payload, "candidate").clone();
    set(
        &mut value,
        "expected_revision",
        Value::Integer(u64::MAX.into()),
    );
    set(&mut payload, "candidate", value);
    set(&mut wide, "payload", payload);
    let wide = encode(&wide);
    let Output::Boundary(value) = Output::decode(&wide, &Limits::default()).unwrap() else {
        panic!()
    };
    assert_eq!(value.expected_revision(), u64::MAX);
    let Output::Error { code } = Output::decode(&frames[3], &Limits::default()).unwrap() else {
        panic!()
    };
    assert_eq!(code, "wrong_inference_request");
}

#[test]
fn transport_rejects_truncation_versions_duplicates_unknown_fields_types_and_trailing_data() {
    for bytes in outputs() {
        for len in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(Output::decode(&bytes[..len], &Limits::default()).is_err());
        }
        let original = decode(&bytes);
        let Value::Map(fields) = original.clone() else {
            panic!()
        };
        let mut duplicate = fields.clone();
        duplicate.push(fields[0].clone());
        assert!(Output::decode(&encode(&Value::Map(duplicate)), &Limits::default()).is_err());
        for (key, value) in [
            ("abi_version", Value::Integer(2.into())),
            ("type", Value::Text("unrecognized".into())),
            ("payload", Value::Null),
        ] {
            let mut changed = original.clone();
            set(&mut changed, key, value);
            assert!(Output::decode(&encode(&changed), &Limits::default()).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Output::decode(&trailing, &Limits::default()).is_err());
    }
    let code = "x".repeat(257);
    let bytes = envelope("error", map(vec![("code", Value::Text(code))]));
    assert!(matches!(
        Output::decode(&bytes, &Limits::default()),
        Err(DecodeError::Limit)
    ));
}

#[test]
fn block_integrity_unique_cids_delta_order_and_independent_caller_bounds_precede_data_use() {
    let frames = outputs();
    let original = decode(&frames[0]);
    let payload = field(&original, "payload");
    let Value::Array(items) = field(payload, "blocks") else {
        panic!()
    };
    for corrupt in [true, false] {
        let mut changed = original.clone();
        let mut payload = field(&changed, "payload").clone();
        let mut items = items.clone();
        if corrupt {
            set(&mut items[0], "body", Value::Bytes(vec![0xa0]));
        } else {
            items.push(items[0].clone());
        }
        set(&mut payload, "blocks", Value::Array(items));
        set(&mut changed, "payload", payload);
        assert!(Output::decode(&encode(&changed), &Limits::default()).is_err());
    }
    for limits in [
        Limits {
            max_blocks: 0,
            ..Limits::default()
        },
        Limits {
            max_materialized_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_block_bytes: 1,
            ..Limits::default()
        },
    ] {
        assert!(Output::decode(&frames[0], &limits).is_err());
    }
    let mut changed = decode(&frames[2]);
    let mut payload = field(&changed, "payload").clone();
    let mut candidate = field(&payload, "candidate").clone();
    let Value::Array(items) = field(&candidate, "new_blocks") else {
        panic!()
    };
    let mut reversed = items.clone();
    reversed.reverse();
    set(&mut candidate, "new_blocks", Value::Array(reversed));
    set(&mut payload, "candidate", candidate);
    set(&mut changed, "payload", payload);
    assert!(Output::decode(&encode(&changed), &Limits::default()).is_err());
}

#[test]
fn request_context_parameters_descriptors_and_limits_remain_bounded_and_closed() {
    let frames = outputs();
    let original = decode(&frames[1]);
    for (key, value) in [
        ("projection_version", Value::Integer(2.into())),
        ("context", Value::Bytes(vec![0xf6])),
        ("model_parameters", Value::Null),
        ("limits", map(vec![])),
        ("tools", Value::Array(vec![map(vec![])])),
    ] {
        let mut changed = original.clone();
        let mut payload = field(&changed, "payload").clone();
        let mut request = field(&payload, "request").clone();
        set(&mut request, key, value);
        set(&mut payload, "request", request);
        set(&mut changed, "payload", payload);
        assert!(Output::decode(&encode(&changed), &Limits::default()).is_err());
    }
    assert!(Output::decode(
        &frames[1],
        &Limits {
            max_context_bytes: 1,
            ..Limits::default()
        }
    )
    .is_err());
    let mut deep = vec![0x81; 65];
    deep.push(0);
    assert!(Output::decode(&deep, &Limits::default()).is_err());
    let huge = envelope(
        "error",
        map(vec![("code", Value::Text("x".repeat(MAX_FRAME_BYTES)))]),
    );
    assert!(Output::decode(&huge, &Limits::default()).is_err());
}

pub(crate) fn invalid_output_module() -> Vec<u8> {
    // Minimal import-free test module: required typed exports, one memory, alloc=32,
    // step=(64<<32)|2, static invalid CBOR at offset 64. No WAT dependency or I/O.
    let mut wasm=b"\0asm\x01\0\0\0\x01\x11\x03\x60\x01\x7f\x01\x7f\x60\x02\x7f\x7f\0\x60\x02\x7f\x7f\x01\x7e\x03\x04\x03\0\x01\x02\x05\x03\x01\0\x01".to_vec();
    wasm.extend_from_slice(
        b"\x07\x32\x04\x06memory\x02\0\x0araiv_alloc\0\0\x0craiv_dealloc\0\x01\x09raiv_step\0\x02",
    );
    wasm.extend_from_slice(b"\x0a\x13\x03\x04\0\x41\x20\x0b\x02\0\x0b\x09\0\x42\x82\x80\x80\x80\x80\x08\x0b\x0b\x09\x01\0\x41\xc0\0\x0b\x02\xff\xff");
    wasm
}

#[test]
fn vm_invalid_typed_output_marks_instance_dead_even_with_valid_abi_ranges() {
    let mut vm = GuestVm::new(&invalid_output_module(), VmLimits::default()).unwrap();
    assert!(matches!(
        vm.round_trip(&[0xf6]),
        Err(BridgeError::Output(_))
    ));
    assert!(matches!(vm.round_trip(&[0xf6]), Err(BridgeError::Dead)));
}
