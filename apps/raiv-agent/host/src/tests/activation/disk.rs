use super::*;
use crate::{
    acceptance::accept_ram,
    admission::AdmissionBook,
    disk::{tests::Directory, DiskBlockStore},
};
use raiv::{format::DecodeLimits, limits::Limits};
use std::fs;

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn full_old_new_closure_reopen_readback_every_missing_corrupt_dependency_and_no_manifest_or_head_change(
) {
    let wasm = wasm();
    let directory = Directory::new();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(2).unwrap();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    let mut run = book.start_run(&wasm, registered, &token).unwrap();
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    let mut sealed = run.into_sealed().unwrap();
    let old = registry.agent([0xb0; 32]).unwrap().anchor();
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.place(registry.agent([0xb0; 32]).unwrap().snapshot())
        .unwrap();
    disk.place(sealed.snapshot().unwrap()).unwrap();
    disk.place(sealed.snapshot().unwrap()).unwrap();
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), old);
    drop(disk);
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let snapshot = sealed.snapshot().unwrap();
    for (cid, body) in snapshot.blocks() {
        let path = directory.0.join(cid.to_string());
        fs::remove_file(&path).unwrap();
        assert!(disk.readback(snapshot, &Limits::default()).is_err());
        disk.put(cid, body).unwrap();
        fs::write(&path, [0x80]).unwrap();
        assert!(disk.readback(snapshot, &Limits::default()).is_err());
        assert!(disk.put(cid, body).is_err());
        assert_eq!(fs::read(&path).unwrap(), [0x80]);
        // Explicit test recovery of deliberately corrupted fixture data, not production fallback.
        fs::remove_file(&path).unwrap();
        disk.put(cid, body).unwrap();
    }
    let caller = Limits {
        max_blocks: 0,
        ..Limits::default()
    };
    assert!(disk.readback(snapshot, &caller).is_err());
    let caller = Limits {
        max_block_bytes: 0,
        ..Limits::default()
    };
    assert!(disk.readback(snapshot, &caller).is_err());
    let materialized = disk.readback(snapshot, &Limits::default()).unwrap();
    assert_eq!(registry.agent([0xb0; 32]).unwrap().anchor(), old);
    // This remains accept_ram: persisted block bytes do not supply a durable decision.
    let decision =
        accept_ram(&mut registry, &mut book, &token, &mut sealed, &materialized).unwrap();
    assert_eq!(decision.revision, 1);
    assert!(!directory.0.join("manifest").exists());
}
