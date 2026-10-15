//! Separate CLI settlement is synthetic ledger atomicity, never genuine tool/backend semantics.

use super::wake::{fields, frame, grant, input, script, wake};
use super::*;
use raiv::{
    boundary::Exit, command::CommandView, format::DecodeLimits, identity::command_digest_view,
    limits::Limits, receipt::Receipt,
};
use raiv_agent_host::{
    admission::AdmissionBook,
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::DiskBlockStore,
    recovery::{adopt_local, LocalRecoveryPin},
    registry::Registry,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{ScriptedWorld, WorldRecoveryPin},
    },
    VmLimits,
};

fn bounds() -> LocalExecutorLimits {
    LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: DecodeLimits::default(),
            max_actions: 16,
        },
        manifest: ManifestLimits {
            wire: DecodeLimits::default(),
            max_blocks: 4096,
            max_admissions: 1024,
        },
        caller: Limits::default(),
    }
}

pub(super) fn command_binding(dir: &Path) -> ([u8; 32], [u8; 32], String) {
    let b = bounds();
    let mut disk = DiskBlockStore::open_existing(dir, b.executor.wire).unwrap();
    let raw = disk.manifest(b.manifest.wire).read().unwrap().unwrap();
    let manifest = CheckpointManifest::decode(&raw, &b.manifest).unwrap();
    let source = manifest.restore(&disk, &b.caller).unwrap();
    let ep = source.current_episode().unwrap();
    let boundary = source.boundary(ep.boundaries().last().unwrap()).unwrap();
    let Exit::AwaitAction(bytes) = boundary.exit() else {
        panic!()
    };
    let cmd = CommandView::decode(bytes, &b.caller).unwrap();
    (
        cmd.action_id(),
        command_digest_view(source.definition().execution_domain(), cmd),
        source.root().to_string(),
    )
}

pub(super) fn plan(
    action: [u8; 32],
    digest: [u8; 32],
    outcome: &str,
    debit: u64,
    result: &str,
) -> String {
    format!("action_id=\"{}\"\ncommand_digest=\"{}\"\noutcome=\"{outcome}\"\ndebit=\"{debit}\"\nresult_hex=\"{result}\"\n",hex(&action),hex(&digest))
}

pub(super) fn execute(home: &Path, pin: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .args(["execute-pending", "--plan"])
        .arg(home.join("plan.toml"))
        .arg("--pin")
        .arg(home.join("approved-main.toml"))
        .arg("--world-pin")
        .arg(pin)
        .arg("--config")
        .arg(home.join("agent.toml"))
        .output()
        .unwrap()
}

fn leave_submitted(module: &[u8], dir: &Path, world_dir: &Path, digest: [u8; 32]) {
    let b = bounds();
    let mut disk = DiskBlockStore::open_existing(dir, b.caller.decode_limits()).unwrap();
    let bytes = disk.manifest(b.manifest.wire).read().unwrap().unwrap();
    let manifest = CheckpointManifest::decode(&bytes, &b.manifest).unwrap();
    let selected = LocalRecoveryPin {
        manifest_digest: DaslCid::for_block(DaslCodec::Raw, &bytes).digest(),
        anchor: manifest.anchor(),
        program: manifest.program_binding(),
        resources: VmLimits::default(),
    };
    let mut registry = Registry::new(1).unwrap();
    let mut book = AdmissionBook::new(1024).unwrap();
    adopt_local(
        module,
        &mut disk,
        &mut registry,
        &mut book,
        selected,
        &b.manifest,
        &b.caller,
    )
    .unwrap();
    let reg = registry.agent([0xb0; 32]).unwrap();
    let mut store = DiskBlockStore::open_existing(world_dir, b.executor.wire).unwrap();
    let mut world = ScriptedWorld::adopt(
        reg,
        &book,
        &mut disk,
        &mut store,
        16,
        &WorldRecoveryPin {
            source: selected,
            world_digest: digest,
        },
        &b,
    )
    .unwrap();
    let ticket = world.authorize_pending(reg, &book, &mut disk, &b).unwrap();
    world.begin(ticket, reg, &book, &mut disk, &b).unwrap();
}

#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_execute_pending_debits_once_known_zero_releases_conflicts_and_submitted_do_not_reexecute() {
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("real guest required"));
    let bytes = fs::read(&module).unwrap();
    let pin = DaslCid::for_block(DaslCodec::Raw, &bytes).digest();
    let genesis: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../fixtures/format/genesis.json")).unwrap();
    for (profile, outcome, debit, submitted) in [
        ("veto", "failed", 1, false),
        ("survivor", "succeeded", 1, false),
        ("survivor", "not_executed", 0, false),
        ("survivor", "failed", 1, true),
    ] {
        let home = Directory::new();
        let dir = home.0.join("agent");
        let world_dir = home.0.join("world");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&world_dir).unwrap();
        let cfg = home.0.join("agent.toml");
        fs::write(&cfg, world_config(&module, &dir, &world_dir, profile, pin)).unwrap();
        assert!(command("init", &cfg).status.success());
        let index = if profile == "veto" { 0 } else { 1 };
        let approval = home.0.join("approved-main.toml");
        select_main_pin(&approval, &dir, genesis[index]["root"].as_str().unwrap(), 0);
        let boot = bootstrap_world(&cfg, &approval);
        assert!(boot.status.success());
        let initial_world = fields(&boot, "world_digest");
        fs::write(home.0.join("event.cbor"), input().as_bytes()).unwrap();
        fs::write(
            home.0.join("grant.cbor"),
            grant().encode(&DecodeLimits::default()).unwrap(),
        )
        .unwrap();
        script(
            &home.0.join("replies.cbor"),
            &[frame(
                1,
                if profile == "veto" { "veto" } else { "trade" },
                &[0xa0],
            )],
        );
        assert!(wake(&home.0, &initial_world).status.success());
        let (action, digest, root) = command_binding(&dir);
        select_main_pin(&approval, &dir, &root, 1);
        let status = command("status", &cfg);
        assert!(status.status.success());
        assert_eq!(fields(&status, "pending_action_id"), hex(&action));
        assert_eq!(fields(&status, "command_digest"), hex(&digest));
        let main_before = contents(&dir);
        let world_before = contents(&world_dir);
        fs::write(
            home.0.join("plan.toml"),
            plan([0; 32], digest, outcome, debit, "a0"),
        )
        .unwrap();
        assert!(!execute(&home.0, &initial_world).status.success());
        assert_eq!(contents(&dir), main_before);
        assert_eq!(contents(&world_dir), world_before);
        fs::write(
            home.0.join("plan.toml"),
            plan(action, digest, outcome, debit, "ff"),
        )
        .unwrap();
        assert!(!execute(&home.0, &initial_world).status.success());
        assert_eq!(contents(&world_dir), world_before);
        fs::write(
            home.0.join("plan.toml"),
            plan(action, digest, outcome, debit, "a0"),
        )
        .unwrap();
        if submitted {
            let d = fs::read(world_dir.join("raiv-world.cbor")).unwrap();
            leave_submitted(
                &bytes,
                &dir,
                &world_dir,
                DaslCid::for_block(DaslCodec::Raw, &d).digest(),
            );
            let status = command("world-status", &cfg);
            assert!(status.status.success());
            let selected = fields(&status, "world_digest");
            let retained = contents(&world_dir);
            assert!(!execute(&home.0, &selected).status.success());
            assert_eq!(contents(&world_dir), retained);
            assert_eq!(contents(&dir), main_before);
            continue;
        }
        let result = execute(&home.0, &initial_world);
        assert!(
            result.status.success(),
            "{:?}",
            String::from_utf8(result.stderr)
        );
        assert_eq!(fields(&result, "submission_intents"), "1");
        assert_eq!(
            fields(&result, "virtual_units_available"),
            (10 - debit).to_string()
        );
        let receipt_hex = fields(&result, "receipt_hex");
        let receipt_bytes = (0..receipt_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&receipt_hex[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let receipt = Receipt::decode(&receipt_bytes, &Limits::default()).unwrap();
        assert_eq!(receipt.action_id(), action);
        assert_eq!(receipt.command_digest(), digest);
        assert_eq!(contents(&dir), main_before);
        let status = command("world-status", &cfg);
        let terminal_pin = fields(&status, "world_digest");
        let terminal = contents(&world_dir);
        let repeat = execute(&home.0, &terminal_pin);
        assert!(repeat.status.success());
        assert_eq!(fields(&repeat, "receipt_hex"), receipt_hex);
        assert_eq!(contents(&world_dir), terminal);
        fs::write(
            home.0.join("plan.toml"),
            plan(
                action,
                digest,
                "succeeded",
                if debit == 0 { 1 } else { 0 },
                "a0",
            ),
        )
        .unwrap();
        assert!(!execute(&home.0, &terminal_pin).status.success());
        assert_eq!(contents(&world_dir), terminal);
        assert_eq!(contents(&dir), main_before);
    }
}
