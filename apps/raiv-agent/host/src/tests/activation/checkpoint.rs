use super::*;
use crate::{
    acceptance::accept_ram,
    admission::AdmissionBook,
    checkpoint::{CheckpointError, CheckpointManifest, ManifestLimits},
    disk::{tests::Directory, DiskBlockStore},
    store::{MemoryBlockStore, StoreLimits},
};
use raiv::{format::DecodeLimits, limits::Limits};
fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}
fn encoded(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(v, &mut out).unwrap();
    out
}
fn list(v: &mut Value) -> &mut Vec<Value> {
    let Value::Array(a) = v else { panic!() };
    a
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn typed_metadata_postgenesis_history_unresolved_launch_and_await_action_restore_as_data_only() {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let mut book = AdmissionBook::new(4).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    for (event, grant, tool) in [(0x60, 0x70, ""), (0x61, 0x71, "trade")] {
        let registered = registry.agent([0xb0; 32]).unwrap();
        let input = admission::input(event, grant);
        let grant = input.budget_admission();
        let token = book.admit_actor_wake(registered, input, grant).unwrap();
        let reserved = CheckpointManifest::capture(registered, &book, &caps()).unwrap();
        let mut run = book.start_run(&wasm, registered, &token).unwrap();
        let launched = CheckpointManifest::capture(registered, &book, &caps()).unwrap();
        assert_eq!(launched.admissions().last().unwrap().launch_attempts, 1);
        assert!(reserved.encode(&caps()).unwrap() != launched.encode(&caps()).unwrap());
        let (ticket, _) = run.pending().unwrap();
        run.reply(ticket, &routing::normalized(1, tool, map(vec![])))
            .unwrap();
        let mut run = run.into_sealed().unwrap();
        let mut ram = MemoryBlockStore::new(StoreLimits {
            max_blocks: 100,
            max_bytes: 1_000_000,
            wire: DecodeLimits::default(),
        });
        ram.place(run.snapshot().unwrap()).unwrap();
        disk.place(run.snapshot().unwrap()).unwrap();
        accept_ram(&mut registry, &mut book, &token, &mut run, &ram).unwrap();
        let registered = registry.agent([0xb0; 32]).unwrap();
        let data = CheckpointManifest::capture(registered, &book, &caps()).unwrap();
        let bytes = data.encode(&caps()).unwrap();
        let previous = disk.manifest(caps().wire).read().unwrap();
        disk.manifest(caps().wire)
            .replace(previous.as_deref(), &bytes)
            .unwrap();
        let reloaded = disk.manifest(caps().wire).read().unwrap().unwrap();
        let decoded = CheckpointManifest::decode(&reloaded, &caps()).unwrap();
        let restored = decoded.restore(&disk, &Limits::default()).unwrap();
        assert_eq!(&restored, registered.snapshot());
        assert_eq!(
            registry.agent([0xb0; 32]).unwrap().anchor(),
            decoded.anchor()
        );
        let mut wrong: Value = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        let rows = list(&mut list(&mut wrong)[9]);
        let last = rows.last_mut().unwrap();
        list(&mut list(last)[0])[5] = Value::Bytes(vec![0; 32]);
        let wrong = CheckpointManifest::decode(&encoded(&wrong), &caps()).unwrap();
        assert!(matches!(
            wrong.restore(&disk, &Limits::default()),
            Err(CheckpointError::Binding)
        ));
        let mut missing: Value = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        list(&mut list(&mut missing)[9]).pop();
        assert!(CheckpointManifest::decode(&encoded(&missing), &caps()).is_err());
        assert_eq!(
            registry.agent([0xb0; 32]).unwrap().anchor(),
            decoded.anchor()
        );
    }
    drop(disk);
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    let data = CheckpointManifest::decode(&bytes, &caps()).unwrap();
    let mut omitted: Value = ciborium::de::from_reader(bytes.as_slice()).unwrap();
    list(&mut list(&mut omitted)[9]).remove(0);
    let omitted = CheckpointManifest::decode(&encoded(&omitted), &caps()).unwrap();
    assert!(matches!(
        omitted.restore(&disk, &Limits::default()),
        Err(CheckpointError::Binding)
    ));
    let restored = data.restore(&disk, &Limits::default()).unwrap();
    assert_eq!(restored.state().revision(), 2);
    assert!(matches!(
        restored.state().continuation(),
        raiv::state::Continuation::AwaitAction { .. }
    ));
    // Data recovery doesn't install a head or reconstitute the previous native origin/token.
    let fresh = admission::registry(&wasm);
    assert_eq!(fresh.agent([0xb0; 32]).unwrap().anchor().revision, 0);
}
