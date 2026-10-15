//! Closed bootstrap metadata, no credential/transport access; omitted effort preserves disabled mode.

use super::{fail, read_bounded, Result};
use raiv::definition::{Definition, MemoryProfile};
use raiv_agent_host::inference::{http::ProviderConfig, sse::StreamLimits};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

pub struct Config {
    pub profile: MemoryProfile,
    pub agent: [u8; 32],
    pub module: PathBuf,
    pub module_digest: [u8; 32],
    pub directory: PathBuf,
    pub world: Option<WorldConfig>,
    pub live: Option<ProviderConfig>,
    pub mandate: Option<String>,
    pub reasoning_effort: Option<guest::ReasoningEffort>,
    /// Closed provider family; an absent `provider` key keeps the original `opencode-go` meaning.
    pub provider: guest::Provider,
}

pub struct WorldConfig {
    pub directory: PathBuf,
    pub id: [u8; 32],
    pub units: u64,
    pub capacity: usize,
}

pub fn decimal(value: &str) -> Result<u64> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(fail("unsigned decimal text must be canonical"));
    }
    value
        .parse()
        .map_err(|_| fail("unsigned decimal text exceeds u64"))
}

fn positive(data: &toml::Table, key: &str) -> Result<u64> {
    data.get(key)
        .and_then(toml::Value::as_integer)
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| fail("configuration requires positive bounded integers"))
}

fn table<'a>(value: &'a toml::Table, key: &str, keys: &[&str]) -> Result<&'a toml::Table> {
    let data = value
        .get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| fail("missing configuration section"))?;
    if data.len() != keys.len() || keys.iter().any(|key| !data.contains_key(*key)) {
        return Err(fail("unsupported or incomplete bootstrap configuration"));
    }
    Ok(data)
}

fn text<'a>(data: &'a toml::Table, key: &str) -> Result<&'a str> {
    data.get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| fail("configuration requires text fields"))
}

fn exact(data: &toml::Table, key: &str, wanted: &str) -> Result<()> {
    if text(data, key)? != wanted {
        return Err(fail("unsupported bootstrap configuration selector"));
    }
    Ok(())
}

pub fn id(text: &str) -> Result<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(fail(
            "identifier must be exactly 64 lowercase hexadecimal characters",
        ));
    }
    let mut out = [0; 32];
    for (index, value) in out.iter_mut().enumerate() {
        *value = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| fail("invalid identifier"))?;
    }
    Ok(out)
}

impl Config {
    /// Deterministic metadata only. No environment key lookup, provider or native Guest decisions.
    pub fn definition(&self) -> Result<Definition> {
        Ok(if let Some(live) = &self.live {
            let mandate = self
                .mandate
                .as_deref()
                .ok_or_else(|| fail("live mandate missing"))?;
            match self.reasoning_effort {
                None if self.provider == guest::Provider::OpenAiCompatible => {
                    guest::openai_compatible_definition(
                        self.profile,
                        &live.base_url,
                        &live.model,
                        mandate,
                    )?
                }
                Some(effort) => guest::opencode_reasoning_definition(
                    self.profile,
                    &live.base_url,
                    &live.model,
                    mandate,
                    effort,
                )?,
                None => {
                    guest::opencode_definition(self.profile, &live.base_url, &live.model, mandate)?
                }
            }
        } else {
            if self.reasoning_effort.is_some() {
                return Err(fail("reasoning requires a live profile"));
            }
            guest::scripted_definition(self.profile)?
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, 16384)?;
        let source =
            std::str::from_utf8(&bytes).map_err(|_| fail("configuration must be UTF-8"))?;
        Self::parse(source, path.parent().unwrap_or(Path::new(".")))
    }

    fn parse(source: &str, base: &Path) -> Result<Self> {
        if source.len() > 16384 {
            return Err(fail("configuration byte limit"));
        }
        let root = toml::Table::from_str(source).map_err(|_| fail("invalid TOML configuration"))?;
        let keys = [
            "schema_version",
            "state_format",
            "agent",
            "execution",
            "runtime",
            "inference",
            "storage",
            "acceptance",
        ];
        if keys.iter().any(|k| !root.contains_key(*k))
            || root
                .keys()
                .any(|key| !keys.contains(&key.as_str()) && key != "executor")
        {
            return Err(fail("unsupported or incomplete bootstrap configuration"));
        }
        if root["schema_version"].as_integer() != Some(1) {
            return Err(fail("unsupported configuration version"));
        }
        exact(&root, "state_format", "raiv-state/1")?;
        let protocol = root["inference"]
            .as_table()
            .ok_or_else(|| fail("missing inference table"))?;
        let scripted = text(protocol, "protocol")? == "scripted";
        let provider = match protocol.get("provider").map(toml::Value::as_str) {
            None => guest::Provider::OpencodeGo,
            Some(Some("opencode-go")) => guest::Provider::OpencodeGo,
            Some(Some("openai-compatible")) => guest::Provider::OpenAiCompatible,
            Some(_) => {
                return Err(fail(
                    "provider must be opencode-go or openai-compatible; omit for opencode-go",
                ))
            }
        };
        let agent = table(
            &root,
            "agent",
            if scripted {
                &["profile", "id"]
            } else {
                &["profile", "id", "mandate"]
            },
        )?;
        let profile = match text(agent, "profile")? {
            "veto" => MemoryProfile::None,
            "survivor" => MemoryProfile::IntentsStrategyTactics,
            _ => return Err(fail("unsupported scripted profile")),
        };
        let runtime = table(&root, "runtime", &["guest_module", "module_digest"])?;
        exact(
            table(&root, "execution", &["adapter"])?,
            "adapter",
            "local-trusted",
        )?;
        let live = if scripted {
            let inference = table(&root, "inference", &["protocol", "model"])?;
            exact(inference, "model", "fixture-model")?;
            None
        } else {
            let mut keys = vec![
                "protocol",
                "model",
                "base_url",
                "api_key_env",
                "stream",
                "max_tokens",
                "max_request_bytes",
                "max_response_bytes",
                "max_header_bytes",
                "timeout_ms",
                "max_stream_event_bytes",
                "max_stream_events",
            ];
            if protocol.contains_key("reasoning_effort") {
                keys.push("reasoning_effort");
            }
            if protocol.contains_key("provider") {
                keys.push("provider");
            }
            let inference = table(&root, "inference", &keys)?;
            if provider == guest::Provider::OpencodeGo {
                exact(inference, "api_key_env", "OPENCODE_API_KEY")?;
            }
            let max_request_bytes = positive(inference, "max_request_bytes")?;
            let max_response_bytes = positive(inference, "max_response_bytes")?;
            if max_request_bytes > 8_388_608 || max_response_bytes > 8_388_608 {
                return Err(fail(
                    "native live request/response byte cap exceeds application frame limit",
                ));
            }
            let config = ProviderConfig {
                protocol: text(inference, "protocol")?.into(),
                model: text(inference, "model")?.into(),
                base_url: text(inference, "base_url")?.into(),
                api_key_env: text(inference, "api_key_env")?.into(),
                stream: inference["stream"]
                    .as_bool()
                    .ok_or_else(|| fail("stream requires an explicit boolean"))?,
                max_tokens: positive(inference, "max_tokens")?,
                max_request_bytes,
                max_response_bytes,
                max_header_bytes: usize::try_from(positive(inference, "max_header_bytes")?)?,
                timeout: Duration::from_millis(positive(inference, "timeout_ms")?),
                stream_limits: StreamLimits {
                    max_wire_bytes: max_response_bytes,
                    max_event_bytes: positive(inference, "max_stream_event_bytes")?,
                    max_events: positive(inference, "max_stream_events")?,
                },
            };
            config.validate()?;
            Some(config)
        };
        let reasoning_effort = match protocol.get("reasoning_effort") {
            None => None,
            Some(value) => Some(match value.as_str() {
                Some("low") => guest::ReasoningEffort::Low,
                Some("high") => guest::ReasoningEffort::High,
                _ => return Err(fail(
                    "reasoning_effort must be explicitly low or high; omit for disabled profile",
                )),
            }),
        };
        if reasoning_effort.is_some() && provider != guest::Provider::OpencodeGo {
            return Err(fail(
                "reasoning_effort is an opencode-go extension; openai-compatible has none",
            ));
        }
        let mandate = if scripted {
            None
        } else {
            let value = text(agent, "mandate")?;
            if value.is_empty() {
                return Err(fail("live mandate must be explicit nonempty text"));
            }
            Some(value.to_owned())
        };
        let storage = table(&root, "storage", &["adapter", "directory"])?;
        exact(storage, "adapter", "disk")?;
        exact(
            table(&root, "acceptance", &["adapter"])?,
            "adapter",
            "local",
        )?;
        let resolve = |value: &str| -> Result<PathBuf> {
            if value.is_empty() {
                Err(fail("empty configured path"))
            } else {
                Ok(base.join(value))
            }
        };
        let world = if root.contains_key("executor") {
            let executor = table(
                &root,
                "executor",
                &[
                    "adapter",
                    "directory",
                    "world_id",
                    "virtual_units",
                    "capacity",
                ],
            )?;
            exact(executor, "adapter", "scripted-unit-world")?;
            let capacity = executor["capacity"]
                .as_integer()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n > 0 && *n <= 1024)
                .ok_or_else(|| fail("scripted world capacity must be 1..1024"))?;
            Some(WorldConfig {
                directory: resolve(text(executor, "directory")?)?,
                id: id(text(executor, "world_id")?)?,
                units: decimal(text(executor, "virtual_units")?)?,
                capacity,
            })
        } else {
            None
        };
        let config = Self {
            profile,
            agent: id(text(agent, "id")?)?,
            module: resolve(text(runtime, "guest_module")?)?,
            module_digest: id(text(runtime, "module_digest")?)?,
            directory: resolve(text(storage, "directory")?)?,
            world,
            live,
            mandate,
            reasoning_effort,
            provider,
        };
        // Enabled mode must be supported by the compiled guest, not guessed from catalogue DATA.
        if reasoning_effort.is_some() {
            config.definition()?;
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        format!(
            r#"schema_version=1
state_format="raiv-state/1"
[agent]
profile="survivor"
id="{}"
[execution]
adapter="local-trusted"
[runtime]
guest_module="guest.wasm"
module_digest="{}"
[inference]
protocol="scripted"
model="fixture-model"
[storage]
adapter="disk"
directory="blocks"
[acceptance]
adapter="local"
"#,
            "b0".repeat(32),
            "01".repeat(32)
        )
    }

    #[test]
    fn exact_offline_subset_preserves_paths_and_rejects_duplicates_unknown_fields_and_live_selectors(
    ) {
        let s = sample();
        let c = Config::parse(&s, Path::new("/owned")).unwrap();
        assert_eq!(c.module, Path::new("/owned/guest.wasm"));
        assert_eq!(c.directory, Path::new("/owned/blocks"));
        for bad in [
            s.replace("schema_version=1", "schema_version=1\nschema_version=1"),
            s.clone() + "\nsecret=\"do-not-echo-me\"\n",
            s.replace("scripted", "openai-responses"),
            s.replace("profile=\"survivor\"", "profile=\"alias\""),
            s.replace("directory=\"blocks\"", "directory=\"\""),
            s.replace("schema_version=1", "schema_version=2"),
            "x".repeat(16385),
        ] {
            let error = Config::parse(&bad, Path::new("/owned"))
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("do-not-echo-me"));
        }
    }

    #[test]
    fn world_configuration_is_optional_closed_and_bounds_full_width_funding_without_credential_lookup(
    ) {
        let s = sample();
        assert!(Config::parse(&s, Path::new(".")).unwrap().world.is_none());
        let extra=format!("\n[executor]\nadapter=\"scripted-unit-world\"\ndirectory=\"world\"\nworld_id=\"{}\"\nvirtual_units=\"18446744073709551615\"\ncapacity=16\n","cc".repeat(32));
        let full = s.clone() + &extra;
        let c = Config::parse(&full, Path::new("/owned")).unwrap();
        let world = c.world.unwrap();
        assert_eq!(world.units, u64::MAX);
        assert_eq!(world.directory, Path::new("/owned/world"));
        for bad in [
            full.replace("capacity=16", "capacity=0"),
            full.replace("capacity=16", "capacity=1025"),
            full.replace("18446744073709551615", "18446744073709551616"),
            full.replace("18446744073709551615", "01"),
            full.replace("scripted-unit-world", "simulator"),
            full + "secret=\"do-not-echo\"",
        ] {
            assert!(Config::parse(&bad, Path::new(".")).is_err());
        }
        assert_eq!(decimal("0").unwrap(), 0);
    }

    #[test]
    fn openai_compatible_provider_is_explicit_names_its_own_key_and_has_no_extensions() {
        let live = r#"protocol="openai-chat-completions"
provider="openai-compatible"
model="qwen3.8-flash"
base_url="https://api.example.com/v1"
api_key_env="EXAMPLE_API_KEY"
stream=true
max_tokens=1024
max_request_bytes=262144
max_response_bytes=1048576
max_header_bytes=16384
timeout_ms=60000
max_stream_event_bytes=65536
max_stream_events=4096"#;
        let source = sample()
            .replace("id=", "mandate=\"Assist the operator.\"\nid=")
            .replace("protocol=\"scripted\"\nmodel=\"fixture-model\"", live);
        let config = Config::parse(&source, Path::new("/owned")).unwrap();
        assert_eq!(config.provider, guest::Provider::OpenAiCompatible);
        assert_eq!(config.live.as_ref().unwrap().api_key_env, "EXAMPLE_API_KEY");
        let def = config.definition().unwrap();
        let profile = guest::ChatProfile::decode(def.inference_profile_bytes()).unwrap();
        assert_eq!(profile.provider, guest::Provider::OpenAiCompatible);
        assert_eq!(profile.model, "qwen3.8-flash");
        for bad in [
            source.replace("openai-compatible", "openai"),
            source.replace("EXAMPLE_API_KEY", "not a name"),
            source.replace("stream=true", "stream=true\nreasoning_effort=\"high\""),
            source
                .replace("provider=\"openai-compatible\"\n", "")
                .replace("EXAMPLE_API_KEY", "OTHER_API_KEY"),
        ] {
            assert!(Config::parse(&bad, Path::new("/owned")).is_err());
        }
    }

    #[test]
    fn live_configuration_is_closed_bounded_data_only_and_definition_binding_is_exact() {
        let source = sample()
            .replace("id=", "mandate=\"Assist the operator.\"\nid=")
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
            );
        let config = Config::parse(&source, Path::new("/owned")).unwrap();
        let def = config.definition().unwrap();
        let profile = guest::ChatProfile::decode(def.inference_profile_bytes()).unwrap();
        assert_eq!(profile.model, "deepseek-v4.1-flash");
        assert_eq!(profile.base_url, "https://opencode.ai/zen/go/v1");
        assert_eq!(
            config.live.as_ref().unwrap().validate().unwrap(),
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
        assert_ne!(
            def.cid(),
            Config::parse(&sample(), Path::new("/owned"))
                .unwrap()
                .definition()
                .unwrap()
                .cid()
        );
        let changed = Config::parse(
            &source.replace("Assist the operator.", "Different mandate."),
            Path::new("/owned"),
        )
        .unwrap()
        .definition()
        .unwrap();
        assert_ne!(def.cid(), changed.cid());
        let alias = Config::parse(
            &source.replace("openai-chat-completions", "openai-responses"),
            Path::new("/owned"),
        )
        .unwrap();
        assert_eq!(def.as_bytes(), alias.definition().unwrap().as_bytes());
        for bad in [
            source.replace("mandate=\"Assist the operator.\"\n", ""),
            source.replace("Assist the operator.", ""),
            source.replace("max_tokens=1024", "max_tokens=0"),
            source.replace("timeout_ms=60000", "timeout_ms=-1"),
            source.replace("max_request_bytes=262144", "max_request_bytes=8388609"),
            source.replace(
                "max_stream_event_bytes=65536",
                "max_stream_event_bytes=1048577",
            ),
            source.replace("stream=true", "stream=\"true\""),
            source.replace("openai-chat-completions", "openai-completions"),
            source.replace("OPENCODE_API_KEY", "OTHER_API_KEY"),
            source.replace(
                "https://opencode.ai/zen/go/v1",
                "http://opencode.ai/zen/go/v1",
            ),
            source.replace(
                "https://opencode.ai/zen/go/v1",
                "https://secret@opencode.ai/zen/go/v1",
            ),
            source.replace(
                "max_tokens=1024",
                "max_tokens=1024\napi_key=\"do-not-echo-me\"",
            ),
            source.replace("max_tokens=1024", "max_tokens=1024\nmax_tokens=1024"),
        ] {
            let error = Config::parse(&bad, Path::new("/owned"))
                .err()
                .expect("closed live config")
                .to_string();
            assert!(!error.contains("do-not-echo-me"));
        }
    }

    #[test]
    fn reasoning_config_is_explicit_closed_data_only_and_preserves_absent_mode_definition() {
        let source = include_str!("../../../raiv-agent.opencode.toml");
        let off = Config::parse(source, Path::new(".")).unwrap();
        assert_eq!(off.reasoning_effort, None);
        assert_eq!(
            off.definition().unwrap().as_bytes(),
            guest::opencode_definition(
                off.profile,
                &off.live.as_ref().unwrap().base_url,
                &off.live.as_ref().unwrap().model,
                off.mandate.as_deref().unwrap()
            )
            .unwrap()
            .as_bytes()
        );
        let mut ids = Vec::new();
        for (name, effort) in [
            ("low", guest::ReasoningEffort::Low),
            ("high", guest::ReasoningEffort::High),
        ] {
            let source = source.replace(
                "[inference]",
                &format!("[inference]\nreasoning_effort=\"{name}\""),
            );
            let config = Config::parse(&source, Path::new(".")).unwrap();
            assert_eq!(config.reasoning_effort, Some(effort));
            let def = config.definition().unwrap();
            assert_eq!(
                guest::ChatProfile::decode(def.inference_profile_bytes())
                    .unwrap()
                    .reasoning_effort,
                Some(effort)
            );
            assert_ne!(def.cid(), off.definition().unwrap().cid());
            ids.push(def.cid());
            assert!(Config::parse(
                &source.replace("deepseek-v4.1-flash", "other-model"),
                Path::new(".")
            )
            .is_err());
        }
        assert_ne!(ids[0], ids[1]);
        for field in [
            "reasoning_effort=\"off\"",
            "reasoning_effort=\"max\"",
            "reasoning_effort=\"HIGH\"",
            "reasoning_effort=true",
            "reasoning_effort=1",
            "thinking=true",
            "reasoning_effort=\"high\"\nreasoning_effort=\"low\"",
        ] {
            assert!(Config::parse(
                &source.replace("[inference]", &format!("[inference]\n{field}")),
                Path::new(".")
            )
            .is_err());
        }
        assert!(Config::parse(
            &sample().replace("[inference]", "[inference]\nreasoning_effort=\"high\""),
            Path::new(".")
        )
        .is_err());
    }

    #[test]
    fn identifiers_are_exact_not_synonyms_or_unicode_slices() {
        for v in [
            "a".repeat(63),
            "A".repeat(64),
            "ж".repeat(32),
            "g".repeat(64),
        ] {
            assert!(id(&v).is_err());
        }
        assert_eq!(id(&"ff".repeat(32)).unwrap(), [255; 32]);
    }
}
