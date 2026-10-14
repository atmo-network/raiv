//! Installed primary-ledger terminal -> one fresh guest activation retaining the accepted Episode.
use super::*;
use super::{
    execution::{command_binding, execute, plan},
    wake::{fields, frame, grant, input, script, wake},
};
use raiv::{
    boundary::Exit,
    command::CommandView,
    format::DecodeLimits,
    input::{ActivationInput, ActivationInputSpec, Cause},
    limits::Limits,
    position::Position,
    receipt::{Outcome, Receipt, ReceiptSpec},
};
use raiv_agent_host::{
    admission::AdmissionStatus,
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::DiskBlockStore,
};
fn resume(home: &Path, digest: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .arg("resume")
        .arg("--receipt")
        .arg(home.join("result-input.cbor"))
        .arg("--grant")
        .arg(home.join("result-grant.cbor"))
        .arg("--replies")
        .arg(home.join("result-replies.cbor"))
        .arg("--pin")
        .arg(home.join("approved-main.toml"))
        .arg("--world-pin")
        .arg(digest)
        .arg("--config")
        .arg(home.join("agent.toml"))
        .output()
        .unwrap()
}
fn result_input(home: &Path, receipt: &Receipt) {
    let mut approved = grant();
    approved.admission_id = [0x76; 32];
    let input = ActivationInput::encode(
        &ActivationInputSpec {
            event_id: [0x66; 32],
            cause: Cause::ActionResult,
            observed_at: Position {
                domain: [1; 32],
                number: 1,
                reference: None,
            },
            observation: &[0xa0],
            receipt: Some(receipt),
            budget_admission: approved,
        },
        &Limits::default(),
    )
    .unwrap();
    fs::write(home.join("result-input.cbor"), input.as_bytes()).unwrap();
    fs::write(
        home.join("result-grant.cbor"),
        approved.encode(&DecodeLimits::default()).unwrap(),
    )
    .unwrap();
}
fn from_hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
        .collect()
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_resume_requires_installed_exact_receipt_and_retains_episode_memory_counts_for_finish_or_retry(
) {
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("real guest required"));
    let pin = DaslCid::for_block(DaslCodec::Raw, &fs::read(&module).unwrap()).digest();
    let genesis: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../fixtures/format/genesis.json")).unwrap();
    for (profile, retry) in [("veto", false), ("survivor", false), ("survivor", true)] {
        let home = Directory::new();
        let dir = home.0.join("agent");
        let world_dir = home.0.join("world");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&world_dir).unwrap();
        let cfg = home.0.join("agent.toml");
        fs::write(&cfg, world_config(&module, &dir, &world_dir, profile, pin)).unwrap();
        let initialized = command("init", &cfg);
        assert!(
            initialized.status.success(),
            "{}",
            String::from_utf8_lossy(&initialized.stderr)
        );
        let approval = home.0.join("approved-main.toml");
        select_main_pin(
            &approval,
            &dir,
            genesis[if profile == "veto" { 0 } else { 1 }]["root"]
                .as_str()
                .unwrap(),
            0,
        );
        let boot = bootstrap_world(&cfg, &approval);
        assert!(boot.status.success());
        let initial_pin = fields(&boot, "world_digest");
        fs::write(home.0.join("event.cbor"), input().as_bytes()).unwrap();
        fs::write(
            home.0.join("grant.cbor"),
            grant().encode(&DecodeLimits::default()).unwrap(),
        )
        .unwrap();
        let first = if profile == "veto" {
            vec![frame(1, "veto", &[0xa0])]
        } else {
            vec![
                frame(1, "patch_memory", b"\xa1\x67intents\xa1\x64keep\xf4"),
                frame(2, "trade", &[0xa0]),
            ]
        };
        script(&home.0.join("replies.cbor"), &first);
        assert!(wake(&home.0, &initial_pin).status.success());
        let (action, digest, root) = command_binding(&dir);
        select_main_pin(&approval, &dir, &root, 1);
        let costs = b"\xa1\x6dvirtual_units\x01";
        let fabricated = Receipt::encode(
            &ReceiptSpec {
                action_id: action,
                command_digest: digest,
                outcome: Outcome::Failed,
                result: b"\xa1\x64fake\xf5",
                execution_reference: None,
                finalized_at: Position {
                    domain: [0xcc; 32],
                    number: 1,
                    reference: None,
                },
                costs,
            },
            &Limits::default(),
        )
        .unwrap();
        result_input(&home.0, &fabricated);
        script(
            &home.0.join("result-replies.cbor"),
            &[frame(1, if retry { "trade" } else { "" }, &[0xa0])],
        );
        let before = contents(&dir);
        let no_terminal = contents(&world_dir);
        assert!(!resume(&home.0, &initial_pin).status.success());
        assert_eq!(contents(&dir), before);
        assert_eq!(contents(&world_dir), no_terminal);
        fs::write(
            home.0.join("plan.toml"),
            plan(action, digest, "failed", 1, "a0"),
        )
        .unwrap();
        let settled = execute(&home.0, &initial_pin);
        assert!(settled.status.success());
        let exact = Receipt::decode(
            &from_hex(&fields(&settled, "receipt_hex")),
            &Limits::default(),
        )
        .unwrap();
        let status = command("world-status", &cfg);
        let terminal_pin = fields(&status, "world_digest");
        let terminal = contents(&world_dir);
        // Even matching action/digest/outcome cannot replace exact installed result bytes.
        assert!(!resume(&home.0, &terminal_pin).status.success());
        assert_eq!(contents(&dir), before);
        assert_eq!(contents(&world_dir), terminal);
        fs::write(home.0.join("result-input.cbor"), input().as_bytes()).unwrap();
        assert!(!resume(&home.0, &terminal_pin).status.success());
        assert_eq!(contents(&dir), before);
        result_input(&home.0, &exact);
        assert!(!resume(&home.0, &initial_pin).status.success());
        assert_eq!(contents(&dir), before);
        fs::write(
            home.0.join("result-grant.cbor"),
            grant().encode(&DecodeLimits::default()).unwrap(),
        )
        .unwrap();
        assert!(!resume(&home.0, &terminal_pin).status.success());
        assert_eq!(contents(&dir), before);
        result_input(&home.0, &exact);
        let returned = resume(&home.0, &terminal_pin);
        assert!(
            returned.status.success(),
            "{:?}",
            String::from_utf8(returned.stderr)
        );
        assert_eq!(fields(&returned, "revision"), "2");
        assert_eq!(contents(&world_dir), terminal);
        let bounds = ManifestLimits {
            wire: DecodeLimits::default(),
            max_blocks: 4096,
            max_admissions: 1024,
        };
        let mut disk = DiskBlockStore::open_existing(&dir, DecodeLimits::default()).unwrap();
        let bytes = disk.manifest(bounds.wire).read().unwrap().unwrap();
        let manifest = CheckpointManifest::decode(&bytes, &bounds).unwrap();
        let snapshot = manifest.restore(&disk, &Limits::default()).unwrap();
        assert_eq!(manifest.admissions().len(), 2);
        assert!(manifest
            .admissions()
            .iter()
            .all(|r| r.status == AdmissionStatus::Consumed));
        let episode = if retry {
            snapshot.current_episode().unwrap()
        } else {
            snapshot.history().last().unwrap()
        };
        assert_eq!(episode.boundaries().count(), 2);
        assert_eq!(
            episode.id(),
            raiv::identity::episode_id([1; 32], [0xb0; 32], [0x65; 32])
        );
        let first_boundary = snapshot
            .boundary(episode.boundaries().next().unwrap())
            .unwrap();
        let second = snapshot
            .boundary(episode.boundaries().last().unwrap())
            .unwrap();
        assert_eq!(first_boundary.usage().model_calls, first.len() as u64);
        assert_eq!(second.usage().model_calls, 1);
        assert_eq!(second.usage().local_tool_calls, 0);
        if retry {
            let Exit::AwaitAction(bytes) = second.exit() else {
                panic!()
            };
            let next = CommandView::decode(bytes, &Limits::default()).unwrap();
            assert_ne!(next.action_id(), action);
        } else {
            assert_eq!(second.exit(), Exit::AwaitActor);
            assert!(snapshot.current_episode().is_none());
        }
        if profile == "survivor" {
            assert_eq!(
                snapshot.memory().unwrap().value_bytes(),
                b"\xa1\x67intents\xa1\x64keep\xf4"
            );
        }
        let new_root = snapshot.root().to_string();
        drop(disk);
        select_main_pin(&approval, &dir, &new_root, 2);
        let accepted = contents(&dir);
        assert!(!resume(&home.0, &terminal_pin).status.success());
        assert_eq!(contents(&dir), accepted);
        assert_eq!(contents(&world_dir), terminal);
    }
}
