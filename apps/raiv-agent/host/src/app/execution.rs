//! Native-approved synthetic unit settlement, never provider-selected tool semantics or live effects.

use super::{
    bounds,
    config::{id, Config},
    fail, read_bounded,
    world::selected_pin,
    Result,
};
use raiv::{
    boundary::Exit,
    command::CommandView,
    definition::Definition,
    format::{ModelValue, ValueRef},
    graph::RestoredAgent,
    identity::command_digest_view,
    limits::Limits,
    position::Position,
    receipt::{Outcome, Receipt, ReceiptSpec},
};
use raiv_agent_host::{
    admission::AdmissionBook,
    disk::DiskBlockStore,
    recovery::adopt_local,
    registry::Registry,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{ScriptedResult, ScriptedWorld, WorldRecoveryPin},
        Status,
    },
};
use std::{path::Path, str::FromStr};

pub struct Execute<'a> {
    pub plan: &'a Path,
    pub pin: &'a Path,
    pub world_digest: &'a str,
}

struct Plan {
    action: [u8; 32],
    digest: [u8; 32],
    outcome: Outcome,
    debit: u64,
    result: Vec<u8>,
}

impl Plan {
    fn load(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, 16384)?;
        let source = std::str::from_utf8(&bytes).map_err(|_| fail("plan must be UTF-8"))?;
        Self::parse(source)
    }

    fn parse(source: &str) -> Result<Self> {
        let data = toml::Table::from_str(source).map_err(|_| fail("invalid native-plan TOML"))?;
        let keys = [
            "action_id",
            "command_digest",
            "outcome",
            "debit",
            "result_hex",
        ];
        if data.len() != keys.len() || keys.iter().any(|k| !data.contains_key(*k)) {
            return Err(fail(
                "native plan requires exact action/digest/outcome/debit/result",
            ));
        }
        let text = |key: &str| {
            data[key]
                .as_str()
                .ok_or_else(|| fail("native-plan fields must be text"))
        };
        let outcome = match text("outcome")? {
            "succeeded" => Outcome::Succeeded,
            "failed" => Outcome::Failed,
            "not_executed" => Outcome::NotExecuted,
            _ => return Err(fail("unsupported native terminal outcome")),
        };
        let debit = match text("debit")? {
            "0" => 0,
            "1" => 1,
            _ => return Err(fail("synthetic debit must be 0 or 1")),
        };
        if outcome == Outcome::NotExecuted && debit != 0 {
            return Err(fail(
                "NotExecuted requires known zero debit; unknown is not zero",
            ));
        }
        let encoded = text("result_hex")?;
        if encoded.len() > 8192
            || encoded.len() % 2 != 0
            || !encoded
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(fail("bounded result must be canonical lowercase hex"));
        }
        let mut result = Vec::new();
        result.try_reserve_exact(encoded.len() / 2)?;
        for i in (0..encoded.len()).step_by(2) {
            result.push(
                u8::from_str_radix(&encoded[i..i + 2], 16)
                    .map_err(|_| fail("invalid result hex"))?,
            );
        }
        Ok(Self {
            action: id(text("action_id")?)?,
            digest: id(text("command_digest")?)?,
            outcome,
            debit,
            result,
        })
    }
}

pub(super) fn pending<'a>(source: &'a RestoredAgent, caller: &Limits) -> Result<CommandView<'a>> {
    let episode = source
        .current_episode()
        .ok_or_else(|| fail("accepted source has no pending action"))?;
    let last = episode
        .boundaries()
        .last()
        .ok_or_else(|| fail("pending episode has no boundary"))?;
    let boundary = source
        .boundary(last)
        .ok_or_else(|| fail("pending boundary unavailable"))?;
    let Exit::AwaitAction(bytes) = boundary.exit() else {
        return Err(fail("execute-pending requires accepted AwaitAction"));
    };
    Ok(CommandView::decode(bytes, caller)?)
}

pub fn execute(
    config: &Config,
    definition: &Definition,
    module: &[u8],
    files: Execute<'_>,
) -> Result<String> {
    let selected = config
        .world
        .as_ref()
        .ok_or_else(|| fail("explicit independent scripted world required"))?;
    let source = selected_pin(files.pin, config, definition)?;
    let world_digest = id(files.world_digest)?;
    let plan = Plan::load(files.plan)?;
    let caller = Limits::default();
    let limits = LocalExecutorLimits {
        executor: ExecutorLimits {
            wire: caller.decode_limits(),
            max_actions: selected.capacity as u64,
        },
        manifest: bounds(),
        caller,
    };
    let mut agent = DiskBlockStore::open_existing(&config.directory, caller.decode_limits())?;
    let mut store = DiskBlockStore::open_existing(&selected.directory, limits.executor.wire)?;
    let mut registry = Registry::new(1)?;
    let mut admissions = AdmissionBook::new(1024)?;
    adopt_local(
        module,
        &mut agent,
        &mut registry,
        &mut admissions,
        source,
        &limits.manifest,
        &caller,
    )?;
    let reg = registry
        .agent(config.agent)
        .ok_or_else(|| fail("approved source unavailable"))?;
    let command = pending(reg.snapshot(), &caller)?;
    if command.action_id() != plan.action
        || command_digest_view(definition.execution_domain(), command) != plan.digest
    {
        return Err(fail(
            "native plan differs from exact accepted pending Command; no substitution",
        ));
    }
    // Validate the WHOLE resulting Receipt before recording Ready/submission intent.
    let fields = [("virtual_units", ValueRef::Integer(plan.debit as i64))];
    let costs = ModelValue::encode(ValueRef::Map(&fields), &caller.decode_limits())?;
    let receipt = Receipt::encode(
        &ReceiptSpec {
            action_id: plan.action,
            command_digest: plan.digest,
            outcome: plan.outcome,
            result: &plan.result,
            execution_reference: None,
            finalized_at: Position {
                domain: selected.id,
                number: reg.anchor().revision,
                reference: None,
            },
            costs: costs.as_bytes(),
        },
        &caller,
    )?;
    let receipt_hex = hex(receipt.as_bytes());
    let mut world = ScriptedWorld::adopt(
        reg,
        &admissions,
        &mut agent,
        &mut store,
        selected.capacity,
        &WorldRecoveryPin {
            source,
            world_digest,
        },
        &limits,
    )?;
    if world.state().world_id() != selected.id || world.state().initial_units() != selected.units {
        return Err(fail("world differs from separately selected seed/scope"));
    }
    if let Some(row) = world
        .state()
        .ledger()
        .records()
        .find(|row| row.action_id == plan.action)
    {
        if row.status == Status::Submitted {
            return Err(fail(
                "submission already recorded; no fresh submission or inferred terminal result",
            ));
        }
        if row.status == Status::Terminal
            && world.state().ledger().receipt_bytes(plan.action) != Some(receipt.as_bytes())
        {
            return Err(fail(
                "terminal native plan conflicts with installed immutable Receipt",
            ));
        }
    }
    let ticket = world.authorize_pending(reg, &admissions, &mut agent, &limits)?;
    let prior = world.record(ticket)?;
    if prior.status == Status::Ready {
        world.begin(ticket, reg, &admissions, &mut agent, &limits)?;
    }
    // Terminal is identical acknowledgement only, never another begin/debit.
    let record = world.settle(
        ticket,
        reg,
        &admissions,
        &mut agent,
        ScriptedResult {
            outcome: plan.outcome,
            result: &plan.result,
            debit: plan.debit,
        },
        &limits,
    )?;
    Ok(format!("mode=scripted-terminal-ledger\nroot={}\nrevision={}\naction_id={}\ncommand_digest={}\nsubmission_intents={}\nvirtual_units_available={}\nledger_rows={}\nreceipt_hex={}\nexecution=synthetic-unit-counter-only; no market/portfolio/wake/backend permission\nnext-pin=requires-independent-current-world-selection\n",
        reg.anchor().root,reg.anchor().revision,hex(&record.action_id),hex(&record.command_digest),record.submission_intents,world.state().available_units(),world.state().ledger().records().count(),receipt_hex))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_plan_is_closed_exact_and_cannot_infer_refund_from_unknown() {
        let good=format!("action_id=\"{}\"\ncommand_digest=\"{}\"\noutcome=\"failed\"\ndebit=\"1\"\nresult_hex=\"a0\"\n","01".repeat(32),"02".repeat(32));
        assert_eq!(Plan::parse(&good).unwrap().debit, 1);
        for bad in [
            good.replace("debit=\"1\"", "debit=\"2\""),
            good.replace("failed", "not_executed"),
            good.replace("failed", "unknown"),
            good.replace("a0", "A0"),
            good.clone() + "extra=\"not-authority\"",
            good.clone() + "result_hex=\"00\"",
        ] {
            assert!(Plan::parse(&bad).is_err());
        }
    }
}
