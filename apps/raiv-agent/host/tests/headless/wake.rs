//! Executable ActorWake -> one durable boundary; scripted provider data, no execution.
use super::*;
use minicbor::encode::Write;
use raiv::{
    budget::BudgetAdmission,
    format::DecodeLimits,
    input::{ActivationInput, ActivationInputSpec, Cause},
    limits::Limits,
    position::Position,
    usage::Usage,
};
use raiv_agent_host::{
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::DiskBlockStore,
};
fn limits() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 4096,
        max_admissions: 1024,
    }
}
pub(super) fn grant() -> BudgetAdmission {
    BudgetAdmission {
        admission_id: [0x75; 32],
        activation_units: 100,
        remaining_episode_units: 100,
        remaining_model_calls: 16,
        remaining_actions: 4,
    }
}
pub(super) fn input() -> ActivationInput {
    ActivationInput::encode(
        &ActivationInputSpec {
            event_id: [0x65; 32],
            cause: Cause::ActorWake,
            observed_at: Position {
                domain: [1; 32],
                number: 0,
                reference: None,
            },
            observation: &[0xa0],
            receipt: None,
            budget_admission: grant(),
        },
        &Limits::default(),
    )
    .unwrap()
}
pub(super) fn frame(id: u64, tool: &str, args: &[u8]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(minicbor::encode::write::Cursor::new([0u8; 4096]));
    e.map(4)
        .unwrap()
        .str("text")
        .unwrap()
        .str("")
        .unwrap()
        .str("calls")
        .unwrap()
        .array(u64::from(!tool.is_empty()))
        .unwrap();
    if !tool.is_empty() {
        e.map(3)
            .unwrap()
            .str("call_id")
            .unwrap()
            .str("model-call")
            .unwrap()
            .str("tool_id")
            .unwrap()
            .str(tool)
            .unwrap()
            .str("arguments")
            .unwrap();
        e.writer_mut().write_all(args).unwrap();
    }
    e.str("usage").unwrap();
    let usage = Usage {
        model_calls: 0,
        local_tool_calls: 0,
        input_tokens: None,
        output_tokens: None,
        billed_units: None,
    }
    .encode(&DecodeLimits::default())
    .unwrap();
    e.writer_mut().write_all(&usage).unwrap();
    e.str("request_id").unwrap().u64(id).unwrap();
    e.writer().get_ref()[..e.writer().position()].to_vec()
}
pub(super) fn script(path: &Path, frames: &[Vec<u8>]) {
    let mut e = minicbor::Encoder::new(minicbor::encode::write::Cursor::new([0u8; 16384]));
    e.array(frames.len() as u64).unwrap();
    for frame in frames {
        e.bytes(frame).unwrap();
    }
    fs::write(path, &e.writer().get_ref()[..e.writer().position()]).unwrap();
}
pub(super) fn wake(home: &Path, digest: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .arg("wake")
        .arg("--event")
        .arg(home.join("event.cbor"))
        .arg("--grant")
        .arg(home.join("grant.cbor"))
        .arg("--replies")
        .arg(home.join("replies.cbor"))
        .arg("--pin")
        .arg(home.join("approved-main.toml"))
        .arg("--world-pin")
        .arg(digest)
        .arg("--config")
        .arg(home.join("agent.toml"))
        .output()
        .unwrap()
}
pub(super) fn fields(output: &Output, key: &str) -> String {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap()
        .into()
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_wake_exactly_one_accepted_boundary_and_incomplete_run_never_relaunches_or_executes() {
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("real guest required"));
    let pin = DaslCid::for_block(DaslCodec::Raw, &fs::read(&module).unwrap()).digest();
    let genesis: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../fixtures/format/genesis.json")).unwrap();
    // Failure partitions isolate exhaustion and unused replies after early sealing.
    for (profile, complete, extra) in [
        ("veto", true, false),
        ("survivor", true, false),
        ("survivor", false, false),
        ("veto", false, true),
    ] {
        let home = Directory::new();
        let dir = home.0.join("agent");
        let world = home.0.join("world");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&world).unwrap();
        let cfg = home.0.join("agent.toml");
        fs::write(&cfg, world_config(&module, &dir, &world, profile, pin)).unwrap();
        assert!(command("init", &cfg).status.success());
        let index = if profile == "veto" { 0 } else { 1 };
        let approval = home.0.join("approved-main.toml");
        select_main_pin(&approval, &dir, genesis[index]["root"].as_str().unwrap(), 0);
        let boot = bootstrap_world(&cfg, &approval);
        assert!(boot.status.success());
        let world_pin = fields(&boot, "world_digest");
        let before = contents(&dir);
        let world_before = contents(&world);
        fs::write(home.0.join("event.cbor"), input().as_bytes()).unwrap();
        fs::write(
            home.0.join("grant.cbor"),
            grant().encode(&DecodeLimits::default()).unwrap(),
        )
        .unwrap();
        let script_file = home.0.join("replies.cbor");
        script(&script_file, &[frame(2, "", &[0xa0])]);
        assert!(!wake(&home.0, &world_pin).status.success());
        assert_eq!(contents(&dir), before);
        fs::write(&script_file, [0x81, 0x41, 0xa0]).unwrap();
        assert!(!wake(&home.0, &world_pin).status.success());
        assert_eq!(contents(&dir), before);
        let mut wrong = grant();
        wrong.activation_units = 1;
        fs::write(
            home.0.join("grant.cbor"),
            wrong.encode(&DecodeLimits::default()).unwrap(),
        )
        .unwrap();
        script(&script_file, &[frame(1, "", &[0xa0])]);
        assert!(!wake(&home.0, &world_pin).status.success());
        assert_eq!(contents(&dir), before);
        fs::write(
            home.0.join("grant.cbor"),
            grant().encode(&DecodeLimits::default()).unwrap(),
        )
        .unwrap();
        assert!(!wake(&home.0, &"00".repeat(32)).status.success());
        assert_eq!(contents(&dir), before);
        assert_eq!(contents(&world), world_before);
        // Canonical mixed patch: {goals:[null],intents:{keep:false}}, without remapping.
        let patch = b"\xa2\x65goals\x81\xf6\x67intents\xa1\x64keep\xf4";
        let mut frames = if profile == "veto" {
            vec![frame(1, "veto", &[0xa0])]
        } else {
            vec![frame(1, "patch_memory", patch)]
        };
        if profile == "survivor" && complete {
            frames.push(frame(2, "trade", &[0xa0]));
        }
        if extra {
            frames.push(frame(2, "", &[0xa0]));
        }
        script(&script_file, &frames);
        let result = wake(&home.0, &world_pin);
        assert_eq!(
            result.status.success(),
            complete,
            "{:?}",
            String::from_utf8(result.stderr.clone())
        );
        assert_eq!(contents(&world), world_before);
        let status = command("status", &cfg);
        assert!(status.status.success());
        assert_eq!(
            fields(&status, "launched_admissions"),
            if complete { "0" } else { "1" }
        );
        assert_eq!(
            fields(&status, "consumed_admissions"),
            if complete { "1" } else { "0" }
        );
        let mut disk = DiskBlockStore::open_existing(&dir, DecodeLimits::default()).unwrap();
        let bytes = disk.manifest(limits().wire).read().unwrap().unwrap();
        let manifest = CheckpointManifest::decode(&bytes, &limits()).unwrap();
        let snapshot = manifest.restore(&disk, &Limits::default()).unwrap();
        assert_eq!(
            fields(&status, "memory_cid"),
            snapshot
                .memory()
                .map(|m| m.cid().to_string())
                .unwrap_or_else(|| "none".into())
        );
        assert_eq!(
            fields(&status, "completed_episodes"),
            snapshot.history().count().to_string()
        );
        assert_eq!(manifest.admissions().len(), 1);
        if complete {
            assert!(String::from_utf8(result.stdout)
                .unwrap()
                .contains("execution=not-performed"));
            assert_eq!(snapshot.state().revision(), 1);
            assert!(matches!(
                snapshot.state().continuation(),
                raiv::state::Continuation::AwaitAction { .. }
            ));
            assert_eq!(
                manifest.admissions()[0].status,
                raiv_agent_host::admission::AdmissionStatus::Consumed
            );
            if profile == "survivor" {
                assert_eq!(
                    snapshot.memory().unwrap().value_bytes(),
                    b"\xa1\x67intents\xa1\x64keep\xf4"
                );
            }
            let episode = snapshot.current_episode().unwrap();
            assert_eq!(episode.boundaries().count(), 1);
            let boundary = snapshot
                .boundary(episode.boundaries().last().unwrap())
                .unwrap();
            assert_eq!(boundary.usage().model_calls, frames.len() as u64);
            assert_eq!(
                boundary.usage().local_tool_calls,
                if profile == "survivor" { 1 } else { 0 }
            );
        } else {
            assert_eq!(snapshot.state().revision(), 0);
            assert_eq!(
                manifest.admissions()[0].status,
                raiv_agent_host::admission::AdmissionStatus::Launched
            );
            assert_eq!(manifest.admissions()[0].launch_attempts, 1);
        }
        let root = snapshot.root().to_string();
        let revision = snapshot.state().revision();
        drop(disk);
        select_main_pin(&approval, &dir, &root, revision);
        let retained = contents(&dir);
        assert!(!wake(&home.0, &world_pin).status.success());
        assert_eq!(contents(&dir), retained);
        assert_eq!(contents(&world), world_before);
    }
}
