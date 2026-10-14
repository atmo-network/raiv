//! New independently configured live genesis uses actual guest; no credentials/inference/dispatch.
use super::*;
fn live(source: String) -> String {
    source
        .replace("\nid=", "\nmandate=\"Assist the operator.\"\nid=")
        .replace(
            "protocol=\"scripted\"\nmodel=\"fixture-model\"",
            r#"protocol="openai-chat-completions"
model="deepseek-v4.1-flash"
base_url="https://opencode.ai/zen/go/v1"
api_key_env="OPENCODE_API_KEY"
stream=true
max_tokens=1024
max_request_bytes=262144
max_response_bytes=1048576
max_header_bytes=16384
timeout_ms=60000
max_stream_event_bytes=65536
max_stream_events=4096"#,
        )
}
fn no_credential(kind: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
        .env_remove("OPENCODE_API_KEY")
        .args([kind, "--config"])
        .arg(path)
        .output()
        .unwrap()
}
#[test]
#[ignore = "build Rust wasm32v1-none guest and set RAIV_GUEST_WASM"]
fn real_executable_live_bootstrap_is_key_free_exact_new_definition_and_never_scripted_bypass() {
    for effort in [
        None,
        Some(guest::ReasoningEffort::Low),
        Some(guest::ReasoningEffort::High),
    ] {
        bootstrap_profile(effort);
    }
}
fn bootstrap_profile(effort: Option<guest::ReasoningEffort>) {
    use raiv::limits::Limits;
    let module = PathBuf::from(std::env::var_os("RAIV_GUEST_WASM").expect("actual guest"));
    let bytes = fs::read(&module).unwrap();
    let digest = DaslCid::for_block(DaslCodec::Raw, &bytes).digest();
    for (name, memory) in [
        ("veto", MemoryProfile::None),
        ("survivor", MemoryProfile::IntentsStrategyTactics),
    ] {
        let home = Directory::new();
        let dir = home.0.join("agent");
        let world = home.0.join("world");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&world).unwrap();
        let cfg = home.0.join("live.toml");
        let source = live(world_config(&module, &dir, &world, name, digest));
        let source = if let Some(effort) = effort {
            source.replace(
                "[inference]",
                &format!("[inference]\nreasoning_effort=\"{}\"", effort.as_str()),
            )
        } else {
            source
        };
        fs::write(&cfg, &source).unwrap();
        let status = no_credential("status", &cfg);
        assert!(!status.status.success());
        assert!(contents(&dir).is_empty());
        let init = no_credential("init", &cfg);
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        assert!(String::from_utf8_lossy(&init.stdout).contains("mode=live-profile-data"));
        let before = contents(&dir);
        let status = no_credential("status", &cfg);
        assert!(status.status.success());
        assert!(String::from_utf8_lossy(&status.stdout).contains("mode=live-profile-data"));
        assert!(!no_credential("init", &cfg).status.success());
        assert_eq!(contents(&dir), before);
        let limits = raiv_agent_host::checkpoint::ManifestLimits {
            wire: Limits::default().decode_limits(),
            max_blocks: 4096,
            max_admissions: 1024,
        };
        let manifest = raiv_agent_host::checkpoint::CheckpointManifest::decode(
            &before[&dir.join("raiv-manifest.cbor")],
            &limits,
        )
        .unwrap();
        let definition = match effort {
            None => guest::opencode_definition(
                memory,
                "https://opencode.ai/zen/go/v1",
                "deepseek-v4.1-flash",
                "Assist the operator.",
            ),
            Some(effort) => guest::opencode_reasoning_definition(
                memory,
                "https://opencode.ai/zen/go/v1",
                "deepseek-v4.1-flash",
                "Assist the operator.",
                effort,
            ),
        }
        .unwrap();
        assert_eq!(manifest.program_binding().definition, definition.cid());
        assert!(manifest.admissions().is_empty());
        assert_eq!(manifest.anchor().revision, 0);
        assert_ne!(
            definition.cid(),
            guest::scripted_definition(memory).unwrap().cid()
        );
        let selected = home.0.join("selected.toml");
        select_main_pin(&selected, &dir, &manifest.anchor().root.to_string(), 0);
        let provision = Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
            .env_remove("OPENCODE_API_KEY")
            .args(["init-world", "--pin"])
            .arg(&selected)
            .arg("--config")
            .arg(&cfg)
            .output()
            .unwrap();
        assert!(
            provision.status.success(),
            "{}",
            String::from_utf8_lossy(&provision.stderr)
        );
        let world_before = contents(&world);
        assert!(no_credential("world-status", &cfg).status.success());
        assert_eq!(contents(&world), world_before);
        let other_mode = match effort {
            None => source.replace("[inference]", "[inference]\nreasoning_effort=\"high\""),
            Some(mode) => source.replace(&format!("reasoning_effort=\"{}\"", mode.as_str()), ""),
        };
        for changed in [
            other_mode,
            source.replace("deepseek-v4.1-flash", "another-model"),
            source.replace("Assist the operator.", "Different mandate."),
            source.replace(
                "https://opencode.ai/zen/go/v1",
                "https://another.example/v1",
            ),
        ] {
            fs::write(&cfg, &changed).unwrap();
            let status = no_credential("status", &cfg);
            assert!(!status.status.success());
            if effort.is_none() || !changed.contains("another-model") {
                assert!(String::from_utf8_lossy(&status.stderr).contains("no rebinding"));
            }
            assert_eq!(contents(&dir), before);
            assert_eq!(contents(&world), world_before);
        }
        fs::write(&cfg, &source).unwrap();
        // Actual executable live argument path: absent env credential fails before durable input/attempt.
        let event = home.0.join("event.cbor");
        let grant = home.0.join("grant.cbor");
        fs::write(&event, wake::input().as_bytes()).unwrap();
        fs::write(
            &grant,
            wake::grant()
                .encode(&Limits::default().decode_limits())
                .unwrap(),
        )
        .unwrap();
        let world_digest = wake::fields(&provision, "world_digest");
        let grants = home.0.join("interactive-grants.toml");
        fs::write(
            &grants,
            include_str!("../../../raiv-agent.interactive-grants.toml"),
        )
        .unwrap();
        let non_tty = Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
            .env_remove("OPENCODE_API_KEY")
            .args(["interactive", "--grants"])
            .arg(&grants)
            .arg("--pin")
            .arg(&selected)
            .arg("--world-pin")
            .arg(&world_digest)
            .arg("--config")
            .arg(&cfg)
            .output()
            .unwrap();
        assert!(!non_tty.status.success());
        assert!(String::from_utf8_lossy(&non_tty.stderr).contains("TTY stdin/stdout"));
        assert_eq!(contents(&dir), before);
        assert_eq!(contents(&world), world_before);
        let missing = Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
            .env_remove("OPENCODE_API_KEY")
            .args(["wake", "--event"])
            .arg(&event)
            .arg("--grant")
            .arg(&grant)
            .arg("--pin")
            .arg(&selected)
            .arg("--world-pin")
            .arg(&world_digest)
            .arg("--config")
            .arg(&cfg)
            .output()
            .unwrap();
        assert!(!missing.status.success());
        assert!(String::from_utf8_lossy(&missing.stderr).contains("credential missing"));
        assert_eq!(contents(&dir), before);
        assert_eq!(contents(&world), world_before);
        // The valid16KiB configuration shape must not have an accidental8KiB initialize encoder cap.
        let large_dir = home.0.join("large-agent");
        fs::create_dir(&large_dir).unwrap();
        let large = home.0.join("large.toml");
        fs::write(
            &large,
            live(config(&module, &large_dir, name, digest))
                .replace("Assist the operator.", &"m".repeat(9000)),
        )
        .unwrap();
        let initialized = no_credential("init", &large);
        assert!(
            initialized.status.success(),
            "{}",
            String::from_utf8_lossy(&initialized.stderr)
        );
        assert!(no_credential("status", &large).status.success());
        for (operation, flag) in [("wake", "--event"), ("resume", "--receipt")] {
            let output = Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
                .env_remove("OPENCODE_API_KEY")
                .args([
                    operation,
                    flag,
                    "missing",
                    "--grant",
                    "missing",
                    "--replies",
                    "missing",
                    "--pin",
                    "missing",
                    "--world-pin",
                    "00",
                    "--config",
                ])
                .arg(&cfg)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("scripted replies cannot bypass")
            );
            assert_eq!(contents(&dir), before);
            assert_eq!(contents(&world), world_before);
        }
        let output = Command::new(env!("CARGO_BIN_EXE_raiv-agent"))
            .env_remove("OPENCODE_API_KEY")
            .args([
                "run-scenario",
                "retry-and-sleep",
                "--inference",
                "scripted",
                "--config",
            ])
            .arg(&cfg)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(contents(&dir), before);
        assert_eq!(contents(&world), world_before);
    }
}
