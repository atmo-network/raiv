use super::*;
use crate::{
    checkpoint::ManifestLimits,
    disk::tests::Directory,
    tests::activation::{admission, output},
};
use raiv::format::DecodeLimits;
fn limits() -> LocalExecutorLimits {
    LocalExecutorLimits {
        executor: checkpoint::ExecutorLimits {
            wire: DecodeLimits::default(),
            max_actions: 10,
        },
        manifest: ManifestLimits {
            wire: DecodeLimits::default(),
            max_blocks: 100,
            max_admissions: 10,
        },
        caller: Limits::default(),
    }
}
#[test]
fn empty_genesis_pin_is_exact_paired_data_adoption_keeps_new_origin_and_nonempty_destination_is_never_merged(
) {
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let admissions = AdmissionBook::new(3).unwrap();
    let original = ExecutorBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    crate::policy::initialize_local(
        registered,
        &admissions,
        &mut disk,
        &limits().manifest,
        &limits().caller,
    )
    .unwrap();
    let data = ExecutorCheckpoint::capture(registered, &admissions, &original, &limits().executor)
        .unwrap();
    ExecutorCheckpointFile::new(&mut disk, limits().executor)
        .replace(None, &data)
        .unwrap();
    let pin = current_pin(registered, &admissions, &original, &mut disk, &limits()).unwrap();
    let mut recovered = ExecutorBook::new(3).unwrap();
    let origin = recovered.origin;
    adopt_book(
        registered,
        &admissions,
        &mut recovered,
        &mut disk,
        &pin,
        &limits(),
    )
    .unwrap();
    assert!(recovered.is_empty());
    assert_eq!(recovered.origin, origin);
    assert_ne!(recovered.origin, original.origin);
    let e = crate::simulator::tests::entry(1, 1);
    let i = recovered.stage(&e).unwrap();
    recovered.publish(i, e);
    let before = recovered.entries[0].record();
    assert!(matches!(
        adopt_book(
            registered,
            &admissions,
            &mut recovered,
            &mut disk,
            &pin,
            &limits()
        ),
        Err(ExecutorRecoveryError::NotEmpty)
    ));
    assert_eq!(recovered.entries[0].record(), before);
}
#[test]
fn missing_conflicting_pins_or_source_and_corrupt_files_refuse_without_destination_or_file_repair()
{
    let wasm = output::invalid_output_module();
    let registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let admissions = AdmissionBook::new(3).unwrap();
    let book = ExecutorBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    crate::policy::initialize_local(
        registered,
        &admissions,
        &mut disk,
        &limits().manifest,
        &limits().caller,
    )
    .unwrap();
    assert!(matches!(
        current_pin(registered, &admissions, &book, &mut disk, &limits()),
        Err(ExecutorRecoveryError::MissingCheckpoint)
    ));
    let data =
        ExecutorCheckpoint::capture(registered, &admissions, &book, &limits().executor).unwrap();
    ExecutorCheckpointFile::new(&mut disk, limits().executor)
        .replace(None, &data)
        .unwrap();
    let pin = current_pin(registered, &admissions, &book, &mut disk, &limits()).unwrap();
    for field in 0..5 {
        let mut bad = pin;
        match field {
            0 => bad.checkpoint_digest[0] ^= 1,
            1 => bad.source.manifest_digest[0] ^= 1,
            2 => bad.source.anchor.revision = 1,
            3 => bad.source.program.module_digest[0] ^= 1,
            _ => bad.source.resources.fuel_per_step += 1,
        };
        let mut next = ExecutorBook::new(3).unwrap();
        assert!(matches!(
            adopt_book(
                registered,
                &admissions,
                &mut next,
                &mut disk,
                &bad,
                &limits()
            ),
            Err(ExecutorRecoveryError::PinMismatch)
        ));
        assert!(next.is_empty());
    }
    let path = directory.0.join("raiv-executor.cbor");
    std::fs::write(&path, [0xa0]).unwrap();
    let mut next = ExecutorBook::new(3).unwrap();
    assert!(adopt_book(
        registered,
        &admissions,
        &mut next,
        &mut disk,
        &pin,
        &limits()
    )
    .is_err());
    assert!(next.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), [0xa0]);
}
