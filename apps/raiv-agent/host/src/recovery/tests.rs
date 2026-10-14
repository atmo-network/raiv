use super::*;
use crate::{
    admission::AdmissionStatus,
    disk::tests::Directory,
    tests::activation::{admission, output},
};
use raiv::format::DecodeLimits;
fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}
fn pin(data: &CheckpointManifest, bytes: &[u8]) -> LocalRecoveryPin {
    LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, bytes).digest(),
        anchor: data.anchor(),
        program: data.program_binding(),
        resources: data.resources(),
    }
}
#[test]
fn explicit_pins_missing_bytes_and_late_book_capacity_never_publish_half_recovery() {
    let wasm = output::invalid_output_module();
    let original = admission::registry(&wasm);
    let registered = original.agent([0xb0; 32]).unwrap();
    let mut old_book = AdmissionBook::new(3).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    old_book.admit_actor_wake(registered, input, grant).unwrap();
    let data = CheckpointManifest::capture(registered, &old_book, &caps()).unwrap();
    let bytes = data.encode(&caps()).unwrap();
    let pin = pin(&data, &bytes);
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.place(registered.snapshot()).unwrap();
    disk.manifest(caps().wire).replace(None, &bytes).unwrap();
    for mutation in 0..5 {
        let mut pin = pin;
        match mutation {
            0 => pin.manifest_digest[0] ^= 1,
            1 => pin.anchor.revision += 1,
            2 => pin.program.module_digest[0] ^= 1,
            3 => pin.resources.fuel_per_step += 1,
            4 => pin.resources.memory_bytes += 1,
            _ => unreachable!(),
        };
        let mut registry = Registry::new(2).unwrap();
        let mut book = AdmissionBook::new(3).unwrap();
        assert!(matches!(
            adopt_local(
                &wasm,
                &mut disk,
                &mut registry,
                &mut book,
                pin,
                &caps(),
                &Limits::default()
            ),
            Err(RecoveryError::PinMismatch)
        ));
        assert!(registry.is_empty());
        assert!(book.is_empty());
    }
    let mut no_entries = Registry::new(2).unwrap();
    let mut no_records = AdmissionBook::new(3).unwrap();
    disk.manifest(caps().wire)
        .replace(Some(&bytes), &[0xa0])
        .unwrap();
    assert!(matches!(
        adopt_local(
            &wasm,
            &mut disk,
            &mut no_entries,
            &mut no_records,
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::PinMismatch)
    ));
    disk.manifest(caps().wire)
        .replace(Some(&[0xa0]), &bytes)
        .unwrap();
    let absent = Directory::new();
    let mut absent = DiskBlockStore::open_existing(&absent.0, DecodeLimits::default()).unwrap();
    assert!(matches!(
        adopt_local(
            &wasm,
            &mut absent,
            &mut no_entries,
            &mut no_records,
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::MissingManifest)
    ));
    assert!(no_entries.is_empty());
    assert!(no_records.is_empty());
    let other = original.agent([0xb1; 32]).unwrap();
    let mut book = AdmissionBook::new(1).unwrap();
    let input = admission::input(0x61, 0x71);
    let grant = input.budget_admission();
    book.admit_actor_wake(other, input, grant).unwrap();
    let previous = book.record([0xb1; 32], [0x61; 32]);
    let mut registry = Registry::new(2).unwrap();
    assert!(matches!(
        adopt_local(
            &wasm,
            &mut disk,
            &mut registry,
            &mut book,
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::Admission(AdmissionError::Capacity))
    ));
    assert!(registry.is_empty());
    assert_eq!(book.len(), 1);
    assert_eq!(book.record([0xb1; 32], [0x61; 32]), previous);
    let mut empty_book = AdmissionBook::new(3).unwrap();
    assert!(adopt_local(
        &[0],
        &mut disk,
        &mut registry,
        &mut empty_book,
        pin,
        &caps(),
        &Limits::default()
    )
    .is_err());
    assert!(registry.is_empty());
    assert!(empty_book.is_empty());
    let missing = data.required_cids()[0];
    std::fs::remove_file(directory.0.join(missing.to_string())).unwrap();
    assert!(adopt_local(
        &wasm,
        &mut disk,
        &mut registry,
        &mut empty_book,
        pin,
        &caps(),
        &Limits::default()
    )
    .is_err());
    assert!(registry.is_empty());
    assert!(empty_book.is_empty());
}
#[test]
fn reserved_metadata_remains_blocked_and_old_tokens_cannot_be_recreated_or_adopted() {
    let wasm = output::invalid_output_module();
    let original = admission::registry(&wasm);
    let registered = original.agent([0xb0; 32]).unwrap();
    let mut original_book = AdmissionBook::new(3).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = original_book
        .admit_actor_wake(registered, input, grant)
        .unwrap();
    let data = CheckpointManifest::capture(registered, &original_book, &caps()).unwrap();
    let bytes = data.encode(&caps()).unwrap();
    let pin = pin(&data, &bytes);
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.place(registered.snapshot()).unwrap();
    disk.manifest(caps().wire).replace(None, &bytes).unwrap();
    let mut registry = Registry::new(2).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    assert_eq!(
        adopt_local(
            &wasm,
            &mut disk,
            &mut registry,
            &mut book,
            pin,
            &caps(),
            &Limits::default()
        )
        .unwrap(),
        pin.anchor
    );
    let registered = registry.agent(pin.anchor.agent_id).unwrap();
    assert_eq!(
        book.record(pin.anchor.agent_id, [0x60; 32]).unwrap().status,
        AdmissionStatus::Reserved
    );
    assert_eq!(
        book.record(pin.anchor.agent_id, [0x60; 32])
            .unwrap()
            .launch_attempts,
        0
    );
    assert!(matches!(
        book.start_run(&wasm, registered, &token),
        Err(AdmissionError::ForeignToken)
    ));
    let input = admission::input(0x61, 0x71);
    let grant = input.budget_admission();
    assert!(matches!(
        book.admit_actor_wake(registered, input, grant),
        Err(AdmissionError::Busy)
    ));
    assert_eq!(
        CheckpointManifest::capture(registered, &book, &caps())
            .unwrap()
            .encode(&caps())
            .unwrap(),
        bytes
    );
    assert!(matches!(
        adopt_local(
            &wasm,
            &mut disk,
            &mut registry,
            &mut book,
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::Registry(RegistryError::AlreadyRegistered))
    ));
    assert_eq!(registry.len(), 1);
    assert_eq!(book.len(), 1);
}
