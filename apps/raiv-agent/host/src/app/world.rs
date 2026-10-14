//! Explicit native world genesis approval; data inspection neither adopts nor issues tickets.
use super::{
    bounds,
    config::{decimal, id, Config},
    fail, read_bounded, Result,
};
use raiv::{
    definition::Definition,
    format::{DaslCid, DaslCodec},
    limits::Limits,
};
use raiv_agent_host::{
    admission::AdmissionBook,
    disk::DiskBlockStore,
    recovery::{adopt_local, LocalRecoveryPin},
    registry::{Anchor, Registry},
    run::ProgramBinding,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{ScriptedWorld, WorldGenesis, WorldState},
    },
    VmLimits,
};
use std::{path::Path, str::FromStr};
pub(super) fn selected_pin(
    path: &Path,
    config: &Config,
    definition: &Definition,
) -> Result<LocalRecoveryPin> {
    let bytes = read_bounded(path, 4096)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| fail("pin must be UTF-8"))?;
    parse_pin(text, config, definition)
}
fn parse_pin(text: &str, config: &Config, definition: &Definition) -> Result<LocalRecoveryPin> {
    let table = toml::Table::from_str(text).map_err(|_| fail("invalid pin TOML"))?;
    if table.len() != 3
        || !["root", "revision", "manifest_digest"]
            .iter()
            .all(|k| table.contains_key(*k))
    {
        return Err(fail(
            "pin requires exact root/revision/manifest_digest; DATA never selects current approval",
        ));
    }
    let value = |key: &str| {
        table[key]
            .as_str()
            .ok_or_else(|| fail("pin fields must be text"))
    };
    Ok(LocalRecoveryPin {
        manifest_digest: id(value("manifest_digest")?)?,
        anchor: Anchor {
            agent_id: config.agent,
            root: value("root")?
                .parse::<DaslCid>()
                .map_err(|_| fail("invalid exact root CID"))?,
            revision: decimal(value("revision")?)?,
        },
        program: ProgramBinding {
            module_digest: config.module_digest,
            definition: definition.cid(),
            execution_domain: definition.execution_domain(),
        },
        resources: VmLimits::default(),
    })
}
pub fn execute(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    pin: Option<&Path>,
) -> Result<String> {
    let selected = config
        .world
        .as_ref()
        .ok_or_else(|| fail("explicit scripted-unit-world configuration required"))?;
    let limits = LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: Limits::default().decode_limits(),
            max_actions: selected.capacity as u64,
        },
        manifest: bounds(),
        caller: Limits::default(),
    };
    // Independent directory/FD; same-directory aliases refuse via held cooperative locks.
    if let Some(path) = pin {
        let source = selected_pin(path, config, definition)?;
        let mut agent_store =
            DiskBlockStore::open_existing(&config.directory, limits.caller.decode_limits())?;
        let mut world_store =
            DiskBlockStore::open_existing(&selected.directory, limits.executor.wire)?;
        let mut registry = Registry::new(1)?;
        let mut book = AdmissionBook::new(1024)?;
        adopt_local(
            module,
            &mut agent_store,
            &mut registry,
            &mut book,
            source,
            &limits.manifest,
            &limits.caller,
        )?;
        let registered = registry
            .agent(config.agent)
            .ok_or_else(|| fail("selected source unavailable"))?;
        let genesis = WorldGenesis {
            world_id: selected.id,
            virtual_units: selected.units,
            capacity: selected.capacity,
            source,
        };
        let world = ScriptedWorld::bootstrap(
            registered,
            &book,
            &mut agent_store,
            &mut world_store,
            &genesis,
            &limits,
        )?;
        let bytes = world.state().encode(&limits.executor)?;
        return Ok(format!("mode=scripted-world-genesis\nworld_digest={}\nsource_manifest_digest={}\nsource_root={}\nsource_revision={}\nvirtual_units_initial={}\nvirtual_units_available={}\nledger_rows={}\nauthority=explicit-local-bootstrap; no execution or live permission\n",
            hex(&DaslCid::for_block(DaslCodec::Raw,&bytes).digest()),hex(&source.manifest_digest),source.anchor.root,source.anchor.revision,
            world.state().initial_units(),world.state().available_units(),world.state().ledger().records().count()));
    }
    // Independent primary DATA can be inspected even when Core-store bytes are unavailable.
    let mut store = DiskBlockStore::open_existing(&selected.directory, limits.executor.wire)?;
    let state = WorldState::read(&mut store, &limits.executor)?;
    if state.world_id() != selected.id
        || state.initial_units() != selected.units
        || state.ledger().agent_id() != config.agent
        || state.ledger().program_binding()
            != (ProgramBinding {
                module_digest: config.module_digest,
                definition: definition.cid(),
                execution_domain: definition.execution_domain(),
            })
    {
        return Err(fail(
            "world DATA differs from selected configuration; no rebinding",
        ));
    }
    Ok(format!("mode=scripted-world-data\nworld_digest={}\nvirtual_units_initial={}\nvirtual_units_available={}\nledger_rows={}\nauthority=status-does-not-adopt-reserve-submit-or-dispatch\n",
        hex(&DaslCid::for_block(DaslCodec::Raw,&state.encode(&limits.executor)?).digest()),state.initial_units(),state.available_units(),state.ledger().records().count()))
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_pin_requires_complete_canonical_fields_and_full_u64_revision() {
        let config = Config {
            profile: raiv::definition::MemoryProfile::None,
            agent: [1; 32],
            module: "unused".into(),
            module_digest: [2; 32],
            directory: "unused".into(),
            world: None,
            live: None,
            mandate: None,
            reasoning_effort: None,
        };
        let def = guest::scripted_definition(config.profile).unwrap();
        let root = DaslCid::for_block(DaslCodec::Drisl, &[0xa0]);
        let good = format!(
            "root=\"{root}\"\nrevision=\"18446744073709551615\"\nmanifest_digest=\"{}\"\n",
            "03".repeat(32)
        );
        assert_eq!(
            parse_pin(&good, &config, &def).unwrap().anchor.revision,
            u64::MAX
        );
        for bad in [
            good.replace("18446744073709551615", "18446744073709551616"),
            good.replace("revision=\"18446744073709551615\"", "revision=\"01\""),
            good.replace("revision=\"18446744073709551615\"", "revision=1"),
            good.clone() + "extra=\"not-a-capability\"",
            good.clone() + "root=\"duplicate\"",
        ] {
            assert!(parse_pin(&bad, &config, &def).is_err());
        }
    }
}
