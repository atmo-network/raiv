//! Linux native entrypoint composition; init is explicit local administration, status DATA only.

mod advance;
mod config;
mod execution;
mod interactive;
mod memory_view;
mod scenario;
mod world;

use config::Config;
use raiv::{
    definition::MemoryProfile,
    format::{DaslCid, DaslCodec, DecodeLimits},
    graph::RestoredAgent,
    limits::Limits,
    memory::Memory,
};
use raiv_agent_host::{
    admission::{AdmissionBook, AdmissionStatus},
    checkpoint::{CheckpointManifest, ManifestLimits},
    disk::DiskBlockStore,
    output::Output,
    policy::initialize_local,
    registry::Registry,
    run::ProgramBinding,
    GuestVm, VmLimits, MAX_FRAME_BYTES,
};
use std::{
    error::Error,
    ffi::OsString,
    fs::File,
    io::{self, Read},
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn fail(message: &'static str) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message))
}

fn read_bounded(path: &Path, cap: usize) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > cap as u64 {
        return Err(fail("input file type/byte limit"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(cap + 1)?;
    file.take(cap as u64 + 1).read_to_end(&mut out)?;
    if out.len() > cap {
        return Err(fail("input file grew beyond byte limit"));
    }
    Ok(out)
}

fn bounds() -> ManifestLimits {
    ManifestLimits {
        wire: DecodeLimits::default(),
        max_blocks: 4096,
        max_admissions: 1024,
    }
}

fn initialize(
    config: &Config,
    definition: &raiv::definition::Definition,
    module: &[u8],
    disk: &mut DiskBlockStore,
) -> Result<(Registry, AdmissionBook)> {
    let caller = Limits::default();
    let limits = bounds();
    if disk.manifest(limits.wire).read()?.is_some() {
        return Err(fail("checkpoint already exists; init never replaces or repairs it, use status or explicit pinned recovery"));
    }
    let memory = match config.profile {
        MemoryProfile::None => None,
        MemoryProfile::IntentsStrategyTactics => {
            Some(Memory::encode(&[], &caller.decode_limits())?)
        }
    };
    // Fixed-profile CBOR DATA frame only; guest alone constructs the genesis.
    let cap = definition
        .as_bytes()
        .len()
        .checked_add(memory.as_ref().map_or(0, |m| m.as_bytes().len()))
        .and_then(|n| n.checked_add(256))
        .filter(|n| *n <= MAX_FRAME_BYTES)
        .ok_or_else(|| fail("bootstrap frame byte overflow/bound"))?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(cap)?;
    buffer.resize(cap, 0);
    let mut encoder =
        minicbor::Encoder::new(minicbor::encode::write::Cursor::new(buffer.as_mut_slice()));
    encoder
        .map(3)?
        .str("type")?
        .str("initialize")?
        .str("payload")?
        .map(3)?
        .str("memory")?;
    if let Some(memory) = &memory {
        encoder.bytes(memory.as_bytes())?;
    } else {
        encoder.null()?;
    }
    encoder
        .str("agent_id")?
        .bytes(&config.agent)?
        .str("definition")?
        .bytes(definition.as_bytes())?
        .str("abi_version")?
        .u64(1)?;
    let mut vm = GuestVm::new(module, VmLimits::default())?;
    let response = vm.round_trip(&encoder.writer().get_ref()[..encoder.writer().position()])?;
    let Output::Initialized { root, blocks } = Output::decode(&response, &caller)? else {
        return Err(fail("guest did not return initialized DATA"));
    };
    let snapshot = RestoredAgent::restore(root, &blocks, &caller)?;
    if snapshot.state().agent_id() != config.agent
        || snapshot.definition() != definition
        || snapshot.blocks().count() as u64 != blocks.len()
        || snapshot.memory() != memory.as_ref()
    {
        return Err(fail("guest bootstrap differs from selected profile/input"));
    }
    let mut registry = Registry::new(1)?;
    registry.register_genesis(
        module,
        ProgramBinding {
            module_digest: config.module_digest,
            definition: definition.cid(),
            execution_domain: definition.execution_domain(),
        },
        snapshot,
        VmLimits::default(),
    )?;
    let book = AdmissionBook::new(1024)?;
    let registered = registry
        .agent(config.agent)
        .ok_or_else(|| fail("approved genesis unavailable"))?;
    initialize_local(registered, &book, disk, &limits, &caller)?;
    Ok((registry, book))
}

pub fn run(args: Vec<OsString>) -> Result<String> {
    const HELP:&str="Usage: raiv-agent init|status|world-status --config <file>\n       raiv-agent init-world --pin <approved-main-pin.toml> --config <file>\n       raiv-agent wake --event <input.cbor> --grant <approved-grant.cbor> --replies <script.cbor> --pin <approved-main.toml> --world-pin <hex> --config <file>\n       raiv-agent execute-pending --plan <approved-plan.toml> --pin <approved-main.toml> --world-pin <hex> --config <file>\n       raiv-agent resume --receipt <action-result-input.cbor> --grant <approved-grant.cbor> --replies <script.cbor> --pin <approved-main.toml> --world-pin <hex> --config <file>\n       raiv-agent run-scenario retry-and-sleep --inference scripted --config <file>\nLive wake/resume omit --replies and use selected bounded HTTPS inference; scripted commands require --replies.\nLinux local-test administration/synthetic world; interactive requires --grants <finite-plan.toml> --pin <main.toml> --world-pin <hex> --config <file>; no real effects.\n";
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(HELP.into());
    }
    let is_scenario = args.len() == 6
        && args[0] == "run-scenario"
        && args[1] == "retry-and-sleep"
        && args[2] == "--inference"
        && args[3] == "scripted"
        && args[4] == "--config";
    let is_wake = args.len() == 13
        && args[0] == "wake"
        && [
            "--event",
            "--grant",
            "--replies",
            "--pin",
            "--world-pin",
            "--config",
        ]
        .iter()
        .enumerate()
        .all(|(i, flag)| args[1 + i * 2] == *flag);
    let is_resume = args.len() == 13
        && args[0] == "resume"
        && [
            "--receipt",
            "--grant",
            "--replies",
            "--pin",
            "--world-pin",
            "--config",
        ]
        .iter()
        .enumerate()
        .all(|(i, flag)| args[1 + i * 2] == *flag);
    let live_cause = if args.len() == 11
        && ["--grant", "--pin", "--world-pin", "--config"]
            .iter()
            .enumerate()
            .all(|(i, flag)| args[3 + i * 2] == *flag)
    {
        if args[0] == "wake" && args[1] == "--event" {
            Some(raiv::input::Cause::ActorWake)
        } else if args[0] == "resume" && args[1] == "--receipt" {
            Some(raiv::input::Cause::ActionResult)
        } else {
            None
        }
    } else {
        None
    };
    let is_interactive = args.len() == 9
        && args[0] == "interactive"
        && ["--grants", "--pin", "--world-pin", "--config"]
            .iter()
            .enumerate()
            .all(|(i, flag)| args[1 + i * 2] == *flag);
    let is_execution = args.len() == 9
        && args[0] == "execute-pending"
        && ["--plan", "--pin", "--world-pin", "--config"]
            .iter()
            .enumerate()
            .all(|(i, flag)| args[1 + i * 2] == *flag);
    let (path, pin) = if is_scenario {
        (Path::new(&args[5]), None)
    } else if is_execution || is_interactive {
        (Path::new(&args[8]), None)
    } else if live_cause.is_some() {
        (Path::new(&args[10]), None)
    } else if is_wake || is_resume {
        (Path::new(&args[12]), None)
    } else if args.len() == 3
        && args[1] == "--config"
        && (args[0] == "init" || args[0] == "status" || args[0] == "world-status")
    {
        (Path::new(&args[2]), None)
    } else if args.len() == 5
        && args[0] == "init-world"
        && args[1] == "--pin"
        && args[3] == "--config"
    {
        (Path::new(&args[4]), Some(Path::new(&args[2])))
    } else {
        return Err(fail(HELP));
    };
    let config = Config::load(path)?;
    let module = read_bounded(&config.module, MAX_FRAME_BYTES)?;
    if DaslCid::for_block(DaslCodec::Raw, &module).digest() != config.module_digest {
        return Err(fail(
            "module differs from independently selected configuration pin",
        ));
    }
    if (config.live.is_some() && (is_scenario || is_wake || is_resume))
        || (config.live.is_none() && live_cause.is_some())
    {
        return Err(fail(
            "inference command/config mismatch; scripted replies cannot bypass the live profile",
        ));
    }
    let definition = config.definition()?;
    if is_interactive {
        return interactive::run(
            &config,
            &definition,
            &module,
            Path::new(&args[2]),
            Path::new(&args[4]),
            args[6]
                .to_str()
                .ok_or_else(|| fail("world pin must be lowercase hex text"))?,
        );
    }
    if is_scenario {
        return scenario::execute(&config, &definition, &module);
    }
    if is_execution {
        return execution::execute(
            &config,
            &definition,
            &module,
            execution::Execute {
                plan: Path::new(&args[2]),
                pin: Path::new(&args[4]),
                world_digest: args[6]
                    .to_str()
                    .ok_or_else(|| fail("world pin must be lowercase hex text"))?,
            },
        );
    }
    if let Some(cause) = live_cause {
        return advance::execute(
            &config,
            &definition,
            &module,
            advance::RequestFiles {
                event: Path::new(&args[2]),
                grant: Path::new(&args[4]),
                replies: None,
                pin: Path::new(&args[6]),
                world_digest: args[8]
                    .to_str()
                    .ok_or_else(|| fail("world pin must be lowercase hex text"))?,
            },
            cause,
        );
    }
    if is_wake || is_resume {
        return advance::execute(
            &config,
            &definition,
            &module,
            advance::RequestFiles {
                event: Path::new(&args[2]),
                grant: Path::new(&args[4]),
                replies: Some(Path::new(&args[6])),
                pin: Path::new(&args[8]),
                world_digest: args[10]
                    .to_str()
                    .ok_or_else(|| fail("world pin must be lowercase hex text"))?,
            },
            if is_resume {
                raiv::input::Cause::ActionResult
            } else {
                raiv::input::Cause::ActorWake
            },
        );
    }
    if args[0] == "init-world" || args[0] == "world-status" {
        return world::execute(&config, &definition, &module, pin);
    }
    let caller = Limits::default();
    let limits = bounds();
    let mut disk = DiskBlockStore::open_existing(&config.directory, caller.decode_limits())?;
    if args[0] == "init" {
        initialize(&config, &definition, &module, &mut disk)?;
    }
    // No VM, Registry adoption, funds, inference or pin approval on this inspection path.
    let bytes = disk
        .manifest(limits.wire)
        .read()?
        .ok_or_else(|| fail("checkpoint missing; explicit initialization/recovery required"))?;
    let manifest = CheckpointManifest::decode(&bytes, &limits)?;
    if manifest.anchor().agent_id != config.agent
        || manifest.program_binding().definition != definition.cid()
        || manifest.program_binding().module_digest != config.module_digest
        || manifest.program_binding().execution_domain != definition.execution_domain()
    {
        return Err(fail(
            "checkpoint differs from selected configuration; no rebinding",
        ));
    }
    let snapshot = manifest.restore(&disk, &caller)?;
    let unresolved = manifest
        .admissions()
        .iter()
        .filter(|r| r.status != AdmissionStatus::Consumed)
        .count();
    let pending = if matches!(
        snapshot.state().continuation(),
        raiv::state::Continuation::AwaitAction { .. }
    ) {
        let command = execution::pending(&snapshot, &caller)?;
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        format!(
            "pending_action_id={}\ncommand_digest={}\npending_tool={}\n",
            hex(&command.action_id()),
            hex(&raiv::identity::command_digest_view(
                definition.execution_domain(),
                command
            )),
            command.tool_id()
        )
    } else {
        String::from("pending_action_id=none\n")
    };
    let mode = if config.live.is_some() {
        "live-profile-data"
    } else {
        "scripted-data"
    };
    let mut states = [0usize; 4];
    for row in manifest.admissions() {
        states[match row.status {
            AdmissionStatus::Reserved => 0,
            AdmissionStatus::Attempted => 1,
            AdmissionStatus::Launched => 2,
            AdmissionStatus::Consumed => 3,
        }] += 1;
    }
    let memory_cid = snapshot
        .memory()
        .map(|memory| memory.cid().to_string())
        .unwrap_or_else(|| "none".into());
    // Compact single-line JSON of what the agent remembers; presentation, not authority.
    let memory = snapshot
        .memory()
        .map(|memory| memory.json().to_string())
        .unwrap_or_else(|| "none".into());
    let retained = format!(
        "memory_cid={memory_cid}\nmemory={memory}\ncompleted_episodes={}\n",
        snapshot.history().count()
    );
    let stages=format!("reserved_admissions={}\nattempted_admissions={}\nlaunched_admissions={}\nconsumed_admissions={}\n",states[0],states[1],states[2],states[3]);
    Ok(format!("mode={mode}\nroot={}\nrevision={}\nprofile={}\nphase={:?}\ncurrent_episode={:?}\nadmissions={}\nunresolved={}\nindexed_blocks={}\n{stages}{retained}readiness=indexed-local-bytes-readable\nbudget=checkpoint-data-only; lifetime-spend-unproven\nauthority=status-does-not-adopt-or-dispatch\n{pending}",
        snapshot.root(),snapshot.state().revision(),match config.profile{MemoryProfile::None=>"veto",MemoryProfile::IntentsStrategyTactics=>"survivor"},
        snapshot.state().continuation(),snapshot.state().current_episode_link(),manifest.admissions().len(),unresolved,manifest.required_cids().len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_shape_rejects_implicit_commands_before_any_io() {
        for args in [
            vec![],
            vec!["init"],
            vec!["status", "--config"],
            vec!["wake", "--config", "unused"],
            vec!["init", "--config", "unused", "extra"],
        ] {
            assert!(run(args.into_iter().map(OsString::from).collect()).is_err());
        }
        assert!(run(vec![OsString::from("--help")])
            .unwrap()
            .contains("interactive requires"));
    }
}
