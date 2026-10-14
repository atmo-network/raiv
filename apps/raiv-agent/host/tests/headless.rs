//! Executable bootstrap/status evidence; not live inference or world recovery.
#![cfg(target_os = "linux")]
#[path = "headless/execution.rs"]
mod execution;
#[path = "headless/live_bootstrap.rs"]
mod live_bootstrap;
#[path = "headless/resume.rs"]
mod resume;
#[path = "headless/scenario.rs"]
mod scenario;
#[path = "headless/wake.rs"]
mod wake;
use raiv::{
    definition::MemoryProfile,
    format::{DaslCid, DaslCodec},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "raiv-headless-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn quote(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_str()
            .unwrap()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}
fn config(module: &Path, dir: &Path, profile: &str, digest: [u8; 32]) -> String {
    format!(
        r#"schema_version=1
state_format="raiv-state/1"
[agent]
profile="{profile}"
id="{}"
[execution]
adapter="local-trusted"
[runtime]
guest_module={}
module_digest="{}"
[inference]
protocol="scripted"
model="fixture-model"
[storage]
adapter="disk"
directory={}
[acceptance]
adapter="local"
"#,
        "b0".repeat(32),
        quote(module),
        hex(&digest),
        quote(dir)
    )
}
fn command(kind: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .args([kind, "--config"])
        .arg(path)
        .output()
        .unwrap()
}
fn contents(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}
fn world_config(module: &Path, dir: &Path, world: &Path, profile: &str, pin: [u8; 32]) -> String {
    config(module,dir,profile,pin)+&format!("\n[executor]\nadapter=\"scripted-unit-world\"\ndirectory={}\nworld_id=\"{}\"\nvirtual_units=\"10\"\ncapacity=16\n",quote(world),"cc".repeat(32))
}
fn bootstrap_world(cfg: &Path, pin: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .args(["init-world", "--pin"])
        .arg(pin)
        .arg("--config")
        .arg(cfg)
        .output()
        .unwrap()
}
fn select_main_pin(file: &Path, dir: &Path, root: &str, revision: u64) {
    let bytes = fs::read(dir.join("raiv-manifest.cbor")).unwrap();
    fs::write(
        file,
        format!(
            "root=\"{root}\"\nrevision=\"{revision}\"\nmanifest_digest=\"{}\"\n",
            hex(&DaslCid::for_block(DaslCodec::Raw, &bytes).digest())
        ),
    )
    .unwrap();
}
fn accept_finish(module: &[u8], dir: &Path, profile: MemoryProfile) -> String {
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
        acceptance::accept_local,
        admission::AdmissionBook,
        checkpoint::{CheckpointManifest, ManifestLimits},
        disk::DiskBlockStore,
        launch::{launch_local, record_reserved},
        recovery::{adopt_local, LocalRecoveryPin},
        registry::Registry,
        VmLimits,
    };
    let caller = Limits::default();
    let limits = ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 4096,
        max_admissions: 1024,
    };
    let mut disk = DiskBlockStore::open_existing(dir, caller.decode_limits()).unwrap();
    let raw = disk.manifest(limits.wire).read().unwrap().unwrap();
    let data = CheckpointManifest::decode(&raw, &limits).unwrap();
    let selected = LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &raw).digest(),
        anchor: data.anchor(),
        program: data.program_binding(),
        resources: VmLimits::default(),
    };
    assert_eq!(
        selected.program.definition,
        guest::scripted_definition(profile).unwrap().cid()
    );
    let mut registry = Registry::new(1).unwrap();
    let mut book = AdmissionBook::new(1024).unwrap();
    adopt_local(
        module,
        &mut disk,
        &mut registry,
        &mut book,
        selected,
        &limits,
        &caller,
    )
    .unwrap();
    let grant = BudgetAdmission {
        admission_id: [0x75; 32],
        activation_units: 100,
        remaining_episode_units: 100,
        remaining_model_calls: 16,
        remaining_actions: 4,
    };
    let input = ActivationInput::encode(
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
            budget_admission: grant,
        },
        &caller,
    )
    .unwrap();
    let reg = registry.agent([0xb0; 32]).unwrap();
    let token = book.admit_actor_wake(reg, input, grant).unwrap();
    record_reserved(reg, &book, &token, &mut disk, &limits, &caller).unwrap();
    let mut run =
        launch_local(module, reg, &mut book, &token, &mut disk, &limits, &caller).unwrap();
    let (ticket, _) = run.pending().unwrap();
    let usage = Usage {
        model_calls: 0,
        local_tool_calls: 0,
        input_tokens: None,
        output_tokens: None,
        billed_units: None,
    }
    .encode(&caller.decode_limits())
    .unwrap();
    let mut reply = minicbor::Encoder::new(minicbor::encode::write::Cursor::new([0u8; 1024]));
    reply
        .map(4)
        .unwrap()
        .str("text")
        .unwrap()
        .str("")
        .unwrap()
        .str("calls")
        .unwrap()
        .array(0)
        .unwrap()
        .str("usage")
        .unwrap();
    reply.writer_mut().write_all(&usage).unwrap();
    reply
        .str("request_id")
        .unwrap()
        .u64(ticket.request_id())
        .unwrap();
    run.reply(
        ticket,
        &reply.writer().get_ref()[..reply.writer().position()],
    )
    .unwrap();
    let mut sealed = run.into_sealed().unwrap();
    accept_local(
        &mut registry,
        &mut book,
        &token,
        &mut sealed,
        &mut disk,
        &limits,
        &caller,
    )
    .unwrap();
    registry
        .agent([0xb0; 32])
        .unwrap()
        .anchor()
        .root
        .to_string()
}
#[test]
fn scripted_profile_factory_equals_independent_pinned_metadata() {
    let definitions: serde_json::Value =
        serde_json::from_str(include_str!("../../../../fixtures/format/definition.json")).unwrap();
    for (i, profile) in [MemoryProfile::None, MemoryProfile::IntentsStrategyTactics]
        .into_iter()
        .enumerate()
    {
        let expected = definitions[i]["hex"].as_str().unwrap();
        assert_eq!(
            hex(guest::scripted_definition(profile).unwrap().as_bytes()),
            expected
        );
    }
}
#[test]
fn executable_help_and_invalid_commands_are_bounded_without_configuration_io() {
    let result = Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("interactive requires"));
    let result = command("execute-pending", Path::new("/not/a/configuration"));
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8(result.stderr).unwrap().contains("Usage"));
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_executable_world_bootstrap_needs_approved_main_pin_and_status_is_independent_without_reseed(
) {
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("real guest required"));
    let bytes = fs::read(&module).unwrap();
    let pin = DaslCid::for_block(DaslCodec::Raw, &bytes).digest();
    let genesis: serde_json::Value =
        serde_json::from_str(include_str!("../../../../fixtures/format/genesis.json")).unwrap();
    for (i, profile) in ["veto", "survivor"].into_iter().enumerate() {
        let home = Directory::new();
        let dir = home.0.join("agent");
        let world = home.0.join("world");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&world).unwrap();
        let cfg = home.0.join("agent.toml");
        fs::write(&cfg, world_config(&module, &dir, &world, profile, pin)).unwrap();
        assert!(command("init", &cfg).status.success());
        let agent_before = contents(&dir);
        let approval = home.0.join("approved-main.toml");
        select_main_pin(&approval, &dir, genesis[i]["root"].as_str().unwrap(), 0);
        let selected = fs::read_to_string(&approval).unwrap();
        fs::write(
            &approval,
            selected.replace("revision=\"0\"", "revision=\"1\""),
        )
        .unwrap();
        assert!(!bootstrap_world(&cfg, &approval).status.success());
        assert!(contents(&world).is_empty());
        assert_eq!(contents(&dir), agent_before);
        fs::write(&approval, &selected).unwrap();
        let boot = bootstrap_world(&cfg, &approval);
        assert!(
            boot.status.success(),
            "{:?}",
            String::from_utf8(boot.stderr)
        );
        assert!(String::from_utf8(boot.stdout)
            .unwrap()
            .contains("virtual_units_available=10"));
        let world_before = contents(&world);
        assert_eq!(world_before.len(), 1);
        assert_eq!(contents(&dir), agent_before);
        let status = command("world-status", &cfg);
        assert!(status.status.success());
        assert!(String::from_utf8(status.stdout)
            .unwrap()
            .contains("status-does-not-adopt"));
        assert_eq!(contents(&world), world_before);
        assert_eq!(contents(&dir), agent_before);
        assert!(bootstrap_world(&cfg, &approval).status.success());
        assert_eq!(contents(&world), world_before);
        fs::write(
            &cfg,
            world_config(&module, &dir, &world, profile, pin)
                .replace("virtual_units=\"10\"", "virtual_units=\"11\""),
        )
        .unwrap();
        assert!(!bootstrap_world(&cfg, &approval).status.success());
        assert!(!command("world-status", &cfg).status.success());
        assert_eq!(contents(&world), world_before);
        fs::write(&cfg, world_config(&module, &dir, &dir, profile, pin)).unwrap();
        assert!(!bootstrap_world(&cfg, &approval).status.success());
        assert_eq!(contents(&dir), agent_before);
        let dirty = home.0.join("dirty-world");
        fs::create_dir(&dirty).unwrap();
        fs::write(dirty.join("stray"), b"owned").unwrap();
        fs::write(&cfg, world_config(&module, &dir, &dirty, profile, pin)).unwrap();
        assert!(!bootstrap_world(&cfg, &approval).status.success());
        assert_eq!(contents(&dirty).len(), 1);
        fs::write(&cfg, world_config(&module, &dir, &world, profile, pin)).unwrap();
        let next = accept_finish(
            &bytes,
            &dir,
            if profile == "veto" {
                MemoryProfile::None
            } else {
                MemoryProfile::IntentsStrategyTactics
            },
        );
        select_main_pin(&approval, &dir, &next, 1);
        let advanced = contents(&dir);
        assert!(!bootstrap_world(&cfg, &approval).status.success());
        assert_eq!(contents(&dir), advanced);
        assert_eq!(contents(&world), world_before);
        fs::remove_dir_all(&dir).unwrap();
        assert!(command("world-status", &cfg).status.success());
        assert_eq!(contents(&world), world_before);
        assert!(!bootstrap_world(&cfg, &approval).status.success());
        assert!(!dir.exists());
        fs::write(world.join("raiv-world.cbor"), [0xff]).unwrap();
        let corrupted = contents(&world);
        assert!(!command("world-status", &cfg).status.success());
        assert_eq!(contents(&world), corrupted);
    }
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_executable_init_status_both_profiles_preserve_namespace_and_refuse_rebinding_repair_or_bad_pins(
) {
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("real guest required"));
    let bytes = fs::read(&module).unwrap();
    let pin = DaslCid::for_block(DaslCodec::Raw, &bytes).digest();
    let genesis: serde_json::Value =
        serde_json::from_str(include_str!("../../../../fixtures/format/genesis.json")).unwrap();
    for (i, profile) in ["veto", "survivor"].into_iter().enumerate() {
        let home = Directory::new();
        let dir = home.0.join("blocks");
        fs::create_dir(&dir).unwrap();
        let cfg = home.0.join("agent.toml");
        fs::write(&cfg, config(&module, &dir, profile, pin)).unwrap();
        assert!(!command("status", &cfg).status.success());
        assert!(contents(&dir).is_empty());
        let initialized = command("init", &cfg);
        assert!(
            initialized.status.success(),
            "{:?}",
            String::from_utf8(initialized.stderr)
        );
        let before = contents(&dir);
        assert_eq!(before.len(), if profile == "veto" { 3 } else { 4 });
        let status = command("status", &cfg);
        assert!(status.status.success());
        let report = String::from_utf8(status.stdout).unwrap();
        assert!(report.contains(&format!("root={}", genesis[i]["root"].as_str().unwrap())));
        assert!(report.contains("revision=0"));
        assert!(report.contains("AwaitActor"));
        assert!(report.contains("lifetime-spend-unproven"));
        assert_eq!(contents(&dir), before);
        assert!(!command("init", &cfg).status.success());
        assert_eq!(contents(&dir), before);
        fs::write(
            &cfg,
            config(
                &module,
                &dir,
                if profile == "veto" {
                    "survivor"
                } else {
                    "veto"
                },
                pin,
            ),
        )
        .unwrap();
        assert!(!command("status", &cfg).status.success());
        assert!(!command("init", &cfg).status.success());
        assert_eq!(contents(&dir), before);
        fs::write(&cfg, config(&module, &dir, profile, [0; 32])).unwrap();
        assert!(!command("init", &cfg).status.success());
        assert!(!command("status", &cfg).status.success());
        assert_eq!(contents(&dir), before);
        fs::write(&cfg, config(&module, &dir, profile, pin)).unwrap();
        let block = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|p| p.file_name().unwrap() != "raiv-manifest.cbor")
            .unwrap();
        fs::remove_file(&block).unwrap();
        let missing = contents(&dir);
        assert!(!command("status", &cfg).status.success());
        assert!(!command("init", &cfg).status.success());
        assert_eq!(contents(&dir), missing);
        fs::write(dir.join("raiv-manifest.cbor"), [0xff]).unwrap();
        let corrupt = contents(&dir);
        assert!(!command("status", &cfg).status.success());
        assert!(!command("init", &cfg).status.success());
        assert_eq!(contents(&dir), corrupt);
        let nonempty = home.0.join("nonempty");
        fs::create_dir(&nonempty).unwrap();
        fs::write(nonempty.join("stray"), b"owned-test").unwrap();
        fs::write(&cfg, config(&module, &nonempty, profile, pin)).unwrap();
        assert!(!command("init", &cfg).status.success());
        assert_eq!(contents(&nonempty).len(), 1);
        let absent = home.0.join("absent");
        fs::write(&cfg, config(&module, &absent, profile, pin)).unwrap();
        assert!(!command("init", &cfg).status.success());
        assert!(!absent.exists());
    }
}
