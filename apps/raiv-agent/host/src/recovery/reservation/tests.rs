use super::*;
use crate::{
    disk::tests::Directory,
    launch::record_reserved,
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

#[test]
fn zero_attempt_input_reissue_is_atomic_single_use_and_never_recovers_an_original_issued_token() {
    let wasm = output::invalid_output_module();
    let original = admission::registry(&wasm);
    let registered = original.agent([0xb0; 32]).unwrap();
    let mut original_book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    disk.place(registered.snapshot()).unwrap();
    let base = CheckpointManifest::capture(registered, &original_book, &caps())
        .unwrap()
        .encode(&caps())
        .unwrap();
    disk.manifest(caps().wire).replace(None, &base).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let original_token = original_book
        .admit_actor_wake(registered, input, grant)
        .unwrap();
    record_reserved(
        registered,
        &original_book,
        &original_token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let bytes = disk.manifest(caps().wire).read().unwrap().unwrap();
    let data = CheckpointManifest::decode(&bytes, &caps()).unwrap();
    let pin = LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: data.anchor(),
        program: data.program_binding(),
        resources: data.resources(),
    };
    assert!(matches!(
        resume_reserved(
            registered,
            &mut original_book,
            &mut disk,
            [0x60; 32],
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::Admission(AdmissionError::TokenAlreadyIssued))
    ));
    let mut registry = Registry::new(2).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    adopt_local(
        &wasm,
        &mut disk,
        &mut registry,
        &mut book,
        pin,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let registered = registry.agent([0xb0; 32]).unwrap();
    let before = book.record([0xb0; 32], [0x60; 32]);
    let mut wrong = pin;
    wrong.manifest_digest[0] ^= 1;
    assert!(matches!(
        resume_reserved(
            registered,
            &mut book,
            &mut disk,
            [0x60; 32],
            wrong,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::PinMismatch)
    ));
    let cid = original_token.packet().input().cid();
    let body = original_token.packet().input().as_bytes();
    let path = directory.0.join(cid.to_string());
    std::fs::remove_file(&path).unwrap();
    assert!(resume_reserved(
        registered,
        &mut book,
        &mut disk,
        [0x60; 32],
        pin,
        &caps(),
        &Limits::default()
    )
    .is_err());
    disk.put(cid, body).unwrap();
    std::fs::write(&path, [0x80]).unwrap();
    assert!(resume_reserved(
        registered,
        &mut book,
        &mut disk,
        [0x60; 32],
        pin,
        &caps(),
        &Limits::default()
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), [0x80]);
    std::fs::remove_file(&path).unwrap();
    disk.put(cid, body).unwrap();
    let tight = Limits {
        max_observation_bytes: 0,
        ..Limits::default()
    };
    assert!(resume_reserved(
        registered,
        &mut book,
        &mut disk,
        [0x60; 32],
        pin,
        &caps(),
        &tight
    )
    .is_err());
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), before);
    let token = resume_reserved(
        registered,
        &mut book,
        &mut disk,
        [0x60; 32],
        pin,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(token.packet().binding(), original_token.packet().binding());
    assert_eq!(token.packet().input().as_bytes(), body);
    assert_eq!(token.packet().input().budget_admission(), grant);
    assert_eq!(book.record([0xb0; 32], [0x60; 32]), before);
    assert_eq!(disk.manifest(caps().wire).read().unwrap().unwrap(), bytes);
    drop(token);
    assert!(matches!(
        resume_reserved(
            registered,
            &mut book,
            &mut disk,
            [0x60; 32],
            pin,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::Admission(AdmissionError::TokenAlreadyIssued))
    ));
}
