//! Real runnable bounded reference recipe, checked against an independent preferred-CBOR oracle.
use super::wake::fields;
use super::*;
use raiv::{format::DecodeLimits, limits::Limits, state::Continuation};
use raiv_agent_host::{
    admission::AdmissionStatus,
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::DiskBlockStore,
    simulator::{checkpoint::ExecutorLimits, world::WorldState},
};
fn run(config: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .args([
            "run-scenario",
            "retry-and-sleep",
            "--inference",
            "scripted",
            "--config",
        ])
        .arg(config)
        .output()
        .unwrap()
}
fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
#[test]
#[ignore = "build actual Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_retry_and_sleep_runner_fresh_adopts_five_times_reaches_second_wake_and_matches_independent_bytes_without_replay(
) {
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("real guest required"));
    let pin = DaslCid::for_block(DaslCodec::Raw, &fs::read(&module).unwrap()).digest();
    let home = Directory::new();
    let dir = home.0.join("agent");
    let world_dir = home.0.join("world");
    fs::create_dir(&dir).unwrap();
    fs::create_dir(&world_dir).unwrap();
    let cfg = home.0.join("agent.toml");
    fs::write(&cfg, world_config(&module, &dir, &world_dir, "veto", pin)).unwrap();
    assert!(!run(&cfg).status.success());
    assert!(contents(&dir).is_empty());
    assert!(contents(&world_dir).is_empty());
    fs::write(
        &cfg,
        world_config(&module, &dir, &world_dir, "survivor", pin)
            .replace("virtual_units=\"10\"", "virtual_units=\"2\""),
    )
    .unwrap();
    assert!(!run(&cfg).status.success());
    assert!(contents(&dir).is_empty());
    assert!(contents(&world_dir).is_empty());
    fs::write(
        &cfg,
        world_config(&module, &dir, &world_dir, "survivor", pin),
    )
    .unwrap();
    // All preconditions are checked before initialization; no partial reseed for stray DATA.
    fs::write(world_dir.join("stray"), b"not authority").unwrap();
    assert!(!run(&cfg).status.success());
    assert!(contents(&dir).is_empty());
    assert_eq!(contents(&world_dir).len(), 1);
    fs::remove_file(world_dir.join("stray")).unwrap();
    let result = run(&cfg);
    assert!(
        result.status.success(),
        "{:?}",
        String::from_utf8(result.stderr)
    );
    assert_eq!(fields(&result, "boundaries"), "5");
    assert_eq!(fields(&result, "actor_wakes"), "2");
    assert_eq!(fields(&result, "fresh_adoptions"), "5");
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/format/demo_scenario.json"
    ))
    .unwrap();
    let stored = contents(&dir);
    for (index, step) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            fields(&result, &format!("checkpoint.{}.root", index + 1)),
            step["state"]["cid"].as_str().unwrap()
        );
        for name in ["input", "memory", "boundary", "episode", "state"] {
            let expected = bytes(step[name]["hex"].as_str().unwrap());
            assert!(
                stored.iter().any(|(_, v)| v == &expected),
                "missing exact {name} at {index}"
            );
        }
        assert_eq!(
            fields(
                &result,
                &format!("checkpoint.{}.virtual_units_available", index + 1)
            ),
            if index == 0 { "9" } else { "8" }
        );
    }
    assert_eq!(
        fields(&result, "checkpoint.3.world_digest"),
        fields(&result, "checkpoint.4.world_digest")
    );
    assert_eq!(
        fields(&result, "checkpoint.4.world_digest"),
        fields(&result, "checkpoint.5.world_digest")
    );
    let limits = ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 4096,
        max_admissions: 1024,
    };
    let mut disk = DiskBlockStore::open_existing(&dir, limits.wire).unwrap();
    let manifest = CheckpointManifest::decode(
        &disk.manifest(limits.wire).read().unwrap().unwrap(),
        &limits,
    )
    .unwrap();
    assert_eq!(manifest.anchor().revision, 5);
    assert_eq!(manifest.admissions().len(), 5);
    assert!(manifest
        .admissions()
        .iter()
        .all(|r| r.status == AdmissionStatus::Consumed && r.launch_attempts == 1));
    let snapshot = manifest.restore(&disk, &Limits::default()).unwrap();
    assert_eq!(snapshot.state().continuation(), Continuation::AwaitActor);
    assert!(snapshot.current_episode().is_none());
    assert_eq!(snapshot.history().count(), 2);
    assert_eq!(
        snapshot
            .history()
            .map(|e| e.boundaries().count())
            .collect::<Vec<_>>(),
        vec![4, 1]
    );
    drop(disk);
    let mut world_store = DiskBlockStore::open_existing(&world_dir, limits.wire).unwrap();
    let world = WorldState::read(
        &mut world_store,
        &ExecutorLimits {
            wire: limits.wire,
            max_actions: 8,
        },
    )
    .unwrap();
    assert_eq!(world.available_units(), 8);
    assert_eq!(world.ledger().records().count(), 3);
    assert!(world
        .ledger()
        .records()
        .all(|row| row.submission_intents == 1
            && row.status == raiv_agent_host::simulator::Status::Terminal));
    drop(world_store);
    let original = contents(&dir);
    let terminal = contents(&world_dir);
    assert!(!run(&cfg).status.success());
    assert_eq!(contents(&dir), original);
    assert_eq!(contents(&world_dir), terminal);
    assert!(command("status", &cfg).status.success());
    assert!(command("world-status", &cfg).status.success());
}
