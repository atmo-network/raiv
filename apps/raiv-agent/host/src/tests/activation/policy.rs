use super::*;
use crate::{
    acceptance::accept_local,
    admission::AdmissionBook,
    checkpoint::ManifestLimits,
    disk::{tests::Directory, DiskBlockStore},
    launch::{launch_local, record_reserved},
    policy::{current_pin, initialize_local, PolicyError},
    recovery::{adopt_local, RecoveryError},
    registry::Registry,
};
use raiv::{format::DecodeLimits, limits::Limits};
fn caps() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 100,
        max_admissions: 10,
    }
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn baseline_to_committed_current_pin_comes_only_from_matching_approved_head_book_and_old_pin_cannot_select_new_file(
) {
    let wasm = wasm();
    let mut registry = admission::registry(&wasm);
    let registered = registry.agent([0xb0; 32]).unwrap();
    let mut book = AdmissionBook::new(3).unwrap();
    let directory = Directory::new();
    let mut disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let baseline =
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    let input = admission::input(0x60, 0x70);
    let grant = input.budget_admission();
    let token = book.admit_actor_wake(registered, input, grant).unwrap();
    assert!(matches!(
        current_pin(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::Conflict)
    ));
    record_reserved(
        registered,
        &book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let reserved = current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    assert_ne!(reserved.manifest_digest, baseline.manifest_digest);
    assert_eq!(reserved.anchor, baseline.anchor);
    let mut run = launch_local(
        &wasm,
        registered,
        &mut book,
        &token,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let launched = current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    assert_ne!(launched.manifest_digest, reserved.manifest_digest);
    let (ticket, _) = run.pending().unwrap();
    run.reply(ticket, &routing::normalized(1, "", map(vec![])))
        .unwrap();
    let mut run = run.into_sealed().unwrap();
    accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut run,
        &mut disk,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    let registered = registry.agent([0xb0; 32]).unwrap();
    assert!(matches!(
        initialize_local(registered, &book, &mut disk, &caps(), &Limits::default()),
        Err(PolicyError::NotGenesis)
    ));
    let accepted = current_pin(registered, &book, &mut disk, &caps(), &Limits::default()).unwrap();
    assert_eq!(accepted.anchor.revision, 1);
    assert_ne!(accepted.manifest_digest, launched.manifest_digest);
    let mut recovered_registry = Registry::new(2).unwrap();
    let mut recovered_book = AdmissionBook::new(3).unwrap();
    assert!(matches!(
        adopt_local(
            &wasm,
            &mut disk,
            &mut recovered_registry,
            &mut recovered_book,
            baseline,
            &caps(),
            &Limits::default()
        ),
        Err(RecoveryError::PinMismatch)
    ));
    assert!(recovered_registry.is_empty());
    assert!(recovered_book.is_empty());
    adopt_local(
        &wasm,
        &mut disk,
        &mut recovered_registry,
        &mut recovered_book,
        accepted,
        &caps(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        recovered_registry.agent([0xb0; 32]).unwrap().anchor(),
        accepted.anchor
    );
}
