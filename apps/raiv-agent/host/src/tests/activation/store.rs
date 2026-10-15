//! Volatile exact placement/readback only, never acceptance or a storage lease.

use super::*;
use crate::{
    output::Output,
    store::{MemoryBlockStore, StoreError, StoreLimits},
};
use raiv::{
    format::{DaslCid, DaslCodec, DecodeLimits},
    graph::{MaterializedBlocks, RestoredAgent},
    limits::Limits,
};

fn capacity(source: &RestoredAgent) -> StoreLimits {
    StoreLimits {
        max_blocks: source.block_count(),
        max_bytes: source.encoded_bytes(),
        wire: DecodeLimits::default(),
    }
}

#[test]
fn exact_closure_placement_is_idempotent_and_late_capacity_failure_preserves_all_owned_buffers() {
    let source = routing::source();
    let mut store = MemoryBlockStore::new(capacity(&source));
    store.place(&source).unwrap();
    store.readback(&source, &Limits::default()).unwrap();
    let pointers: Vec<_> = source
        .blocks()
        .map(|(cid, _)| (cid, store.get(cid).unwrap().as_ptr()))
        .collect();
    store.place(&source).unwrap();
    assert_eq!(store.block_count(), source.block_count());
    assert_eq!(store.encoded_bytes(), source.encoded_bytes());
    for (cid, pointer) in pointers {
        assert_eq!(store.get(cid).unwrap().as_ptr(), pointer);
    }
    for limits in [
        StoreLimits {
            max_blocks: source.block_count() - 1,
            ..capacity(&source)
        },
        StoreLimits {
            max_bytes: source.encoded_bytes() - 1,
            ..capacity(&source)
        },
    ] {
        let mut partial = MemoryBlockStore::new(limits);
        let (cid, body) = source.blocks().nth(1).unwrap();
        partial.put(cid, body).unwrap();
        let count = partial.block_count();
        let bytes = partial.encoded_bytes();
        let pointer = partial.get(cid).unwrap().as_ptr();
        assert_eq!(partial.place(&source), Err(StoreError::Capacity));
        assert_eq!(partial.block_count(), count);
        assert_eq!(partial.encoded_bytes(), bytes);
        assert_eq!(partial.get(cid).unwrap().as_ptr(), pointer);
    }
}

#[test]
fn wrong_hash_codec_noncanonical_and_narrow_wire_limits_never_become_stored_data() {
    let mut store = MemoryBlockStore::new(StoreLimits {
        max_blocks: 10,
        max_bytes: 1000,
        wire: DecodeLimits::default(),
    });
    for (codec, body) in [
        (DaslCodec::Raw, vec![0xa0]),
        (DaslCodec::Drisl, vec![0x18, 0]),
        (DaslCodec::Drisl, vec![0xa0, 0]),
    ] {
        let cid = DaslCid::for_block(codec, &body);
        assert!(store.put(cid, &body).is_err());
        assert_eq!(store.block_count(), 0);
        assert_eq!(store.encoded_bytes(), 0);
    }
    let cid = DaslCid::for_block(DaslCodec::Drisl, &[0xa0]);
    assert_eq!(store.put(cid, &[0x80]), Err(StoreError::Integrity));
    let source = routing::source();
    let mut tiny = MemoryBlockStore::new(StoreLimits {
        wire: DecodeLimits {
            max_block_bytes: 1,
            ..DecodeLimits::default()
        },
        ..capacity(&source)
    });
    assert!(matches!(tiny.place(&source), Err(StoreError::Codec(_))));
    assert_eq!(tiny.block_count(), 0);
}

#[test]
fn readback_requires_every_dependency_and_stricter_caller_capacity_without_genesis_fallback() {
    let source = routing::source();
    let all: Vec<_> = source.blocks().collect();
    for missing in 0..all.len() {
        let mut store = MemoryBlockStore::new(capacity(&source));
        for (index, (cid, bytes)) in all.iter().enumerate() {
            if index != missing {
                store.put(*cid, bytes).unwrap();
            }
        }
        assert!(matches!(
            store.readback(&source, &Limits::default()),
            Err(StoreError::Graph(_))
        ));
    }
    let mut store = MemoryBlockStore::new(capacity(&source));
    store.place(&source).unwrap();
    assert!(matches!(
        store.readback(
            &source,
            &Limits {
                max_blocks: 0,
                ..Limits::default()
            }
        ),
        Err(StoreError::Graph(_))
    ));
    assert_eq!(store.block_count(), source.block_count());
    store.readback(&source, &Limits::default()).unwrap();
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_sealed_candidate_places_exact_new_and_retained_bytes_without_updating_registered_anchor() {
    let wasm = wasm();
    let source = routing::source();
    let packet = routing::prepared(&source);
    let pin = routing::pin(&wasm, &source);
    let mut registry = crate::registry::Registry::new(1).unwrap();
    let id = source.state().agent_id();
    registry
        .register_genesis(&wasm, pin, source, VmLimits::default())
        .unwrap();
    let registered = registry.agent(id).unwrap();
    let anchor = registered.anchor();
    let mut run = registered.start_run(&wasm, &packet).unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    let next = run.candidate_snapshot().unwrap();
    let old = registered.snapshot();
    let delta = run.candidate().unwrap().new_blocks();
    let mut store = MemoryBlockStore::new(StoreLimits {
        max_blocks: old.block_count() + next.block_count(),
        max_bytes: old.encoded_bytes() + next.encoded_bytes(),
        wire: DecodeLimits::default(),
    });
    store.place(old).unwrap();
    let pointers: Vec<_> = old
        .blocks()
        .map(|(cid, _)| (cid, store.get(cid).unwrap().as_ptr()))
        .collect();
    store.place(next).unwrap();
    store.readback(next, &Limits::default()).unwrap();
    assert_eq!(store.block_count(), old.block_count() + delta.len());
    for block in delta.iter() {
        assert_eq!(store.get(block.cid), Some(block.body));
    }
    for (cid, pointer) in pointers {
        assert_eq!(store.get(cid).unwrap().as_ptr(), pointer);
    }
    assert_eq!(registered.anchor(), anchor);
    // Same sealed data can be placed/read back again, with no additional inference.
    let count = store.block_count();
    store.place(next).unwrap();
    store.readback(next, &Limits::default()).unwrap();
    assert_eq!(store.block_count(), count);
    assert!(run.pending().is_none());
    let Output::Boundary(candidate) = run.output().unwrap() else {
        panic!()
    };
    assert_eq!(candidate.new_root(), next.root());
}
