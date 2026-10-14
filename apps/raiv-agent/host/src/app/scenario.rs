//! Bounded native scripted-administrator recipe. Pins advance only from its approved RAM owners.
mod recipe;
use super::{bounds, config::Config, fail, initialize, Result};
use minicbor::encode::write::Write;
use raiv::{
    budget::BudgetAdmission,
    definition::{Definition, MemoryProfile},
    format::DecodeLimits,
    input::{ActivationInput, ActivationInputSpec, Cause},
    limits::Limits,
    position::Position,
    receipt::{Outcome, Receipt},
    usage::Usage,
};
use raiv_agent_host::{
    acceptance::accept_local,
    admission::AdmissionBook,
    disk::DiskBlockStore,
    launch::{launch_local, record_reserved},
    policy::current_pin,
    recovery::adopt_local,
    registry::Registry,
    simulator::{
        checkpoint::ExecutorLimits,
        local::LocalExecutorLimits,
        world::{LedgerActionResult, ScriptedResult, ScriptedWorld, WorldGenesis},
    },
};
fn reply(id: u64, tool: Option<&str>, args: &[u8]) -> Result<Vec<u8>> {
    let mut e = minicbor::Encoder::new(minicbor::encode::write::Cursor::new([0u8; 4096]));
    e.map(4)?
        .str("text")?
        .str("")?
        .str("calls")?
        .array(u64::from(tool.is_some()))?;
    if let Some(tool) = tool {
        e.map(3)?
            .str("call_id")?
            .str("model-call")?
            .str("tool_id")?
            .str(tool)?
            .str("arguments")?;
        e.writer_mut().write_all(args)?;
    }
    e.str("usage")?;
    let usage = Usage {
        model_calls: 0,
        local_tool_calls: 0,
        input_tokens: None,
        output_tokens: None,
        billed_units: None,
    }
    .encode(&DecodeLimits::default())?;
    e.writer_mut().write_all(&usage)?;
    e.str("request_id")?.u64(id)?;
    let data = &e.writer().get_ref()[..e.writer().position()];
    raiv::activation::ModelReply::decode(data, &Limits::default())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(data.len())?;
    bytes.extend_from_slice(data);
    Ok(bytes)
}
pub fn execute(config: &Config, definition: &Definition, module: &[u8]) -> Result<String> {
    let selected = config
        .world
        .as_ref()
        .ok_or_else(|| fail("scenario requires explicit independent scripted world"))?;
    if config.profile != MemoryProfile::IntentsStrategyTactics
        || selected.units < 3
        || selected.capacity < 3
    {
        return Err(fail("retry-and-sleep requires Survivor, at least three virtual units and three ledger slots"));
    }
    let caller = Limits::default();
    let limits = LocalExecutorLimits {
        manifest: bounds(),
        executor: ExecutorLimits {
            wire: caller.decode_limits(),
            max_actions: selected.capacity as u64,
        },
        caller,
    };
    let mut frames = Vec::new();
    frames.try_reserve_exact(5)?;
    for (patch, tool, args) in recipe::STEPS {
        frames.push([
            reply(1, Some("patch_memory"), patch)?,
            reply(2, tool, args)?,
        ]);
    }
    // Explicit CLI approval is limited to this fixed recipe and empty namespaces, never current DATA discovery.
    let mut pair = {
        let mut agent = DiskBlockStore::open_existing(&config.directory, caller.decode_limits())?;
        let mut world_store =
            DiskBlockStore::open_existing(&selected.directory, limits.executor.wire)?;
        if !agent.is_empty_namespace()? || !world_store.is_empty_namespace()? {
            return Err(fail(
                "run-scenario requires two empty approved namespaces; never repair/replay/reseed",
            ));
        }
        let (registry, admissions) = initialize(config, definition, module, &mut agent)?;
        let registered = registry
            .agent(config.agent)
            .ok_or_else(|| fail("approved genesis unavailable"))?;
        let source = current_pin(
            registered,
            &admissions,
            &mut agent,
            &limits.manifest,
            &caller,
        )?;
        let mut world = ScriptedWorld::bootstrap(
            registered,
            &admissions,
            &mut agent,
            &mut world_store,
            &WorldGenesis {
                source,
                world_id: selected.id,
                virtual_units: selected.units,
                capacity: selected.capacity,
            },
            &limits,
        )?;
        world.current_pin(registered, &admissions, &mut agent, &limits)?
    };
    let mut output=String::from("mode=scripted-unit-world-scenario\nscenario=retry-and-sleep\napproval=fixed-native-scripted-administrator; not external Actor authentication\n");
    for (index, replies) in frames.iter().enumerate() {
        // Drop every VM/session/book/FD at the checkpoint; new origins, no serialized capabilities.
        let mut agent = DiskBlockStore::open_existing(&config.directory, caller.decode_limits())?;
        let mut world_store =
            DiskBlockStore::open_existing(&selected.directory, limits.executor.wire)?;
        let mut registry = Registry::new(1)?;
        let mut admissions = AdmissionBook::new(1024)?;
        adopt_local(
            module,
            &mut agent,
            &mut registry,
            &mut admissions,
            pair.source,
            &limits.manifest,
            &caller,
        )?;
        let reg = registry
            .agent(config.agent)
            .ok_or_else(|| fail("selected approved source unavailable"))?;
        let mut world = ScriptedWorld::adopt(
            reg,
            &admissions,
            &mut agent,
            &mut world_store,
            selected.capacity,
            &pair,
            &limits,
        )?;
        if world.state().world_id() != selected.id
            || world.state().initial_units() != selected.units
        {
            return Err(fail("world configuration differs; no reseed"));
        }
        let is_result = (1..=3).contains(&index);
        let receipt = if is_result {
            let command = super::execution::pending(reg.snapshot(), &caller)?;
            let installed = world
                .state()
                .ledger()
                .receipt_bytes(command.action_id())
                .ok_or_else(|| fail("scenario requires installed terminal, no imported receipt"))?;
            Some(Receipt::decode(installed, &caller)?)
        } else {
            None
        };
        let grant = BudgetAdmission {
            admission_id: [0x90 + index as u8; 32],
            activation_units: 100,
            remaining_episode_units: 100,
            remaining_model_calls: 16,
            remaining_actions: 4,
        };
        let input = ActivationInput::encode(
            &ActivationInputSpec {
                event_id: [170 + index as u8; 32],
                cause: if is_result {
                    Cause::ActionResult
                } else {
                    Cause::ActorWake
                },
                observed_at: Position {
                    domain: [0x30; 32],
                    number: index as u64,
                    reference: None,
                },
                observation: &[0xa0],
                receipt: receipt.as_ref(),
                budget_admission: grant,
            },
            &caller,
        )?;
        let token = if is_result {
            let ticket = world.authorize_pending(reg, &admissions, &mut agent, &limits)?;
            world.admit_action_result(
                ticket,
                reg,
                &mut admissions,
                &mut agent,
                LedgerActionResult {
                    input,
                    approved_grant: grant,
                },
                &limits,
            )?
        } else {
            admissions.admit_actor_wake(reg, input, grant)?
        };
        record_reserved(
            reg,
            &admissions,
            &token,
            &mut agent,
            &limits.manifest,
            &caller,
        )?;
        let mut run = launch_local(
            module,
            reg,
            &mut admissions,
            &token,
            &mut agent,
            &limits.manifest,
            &caller,
        )?;
        for frame in replies {
            let (ticket, _) = run
                .pending()
                .ok_or_else(|| fail("unexpected early boundary; no extra reply"))?;
            run.reply(ticket, frame)?;
        }
        if run.phase() != raiv_agent_host::run::Phase::Boundary {
            return Err(fail(
                "script exhausted without a boundary; no Finish/relaunch",
            ));
        }
        let mut sealed = run.into_sealed()?;
        accept_local(
            &mut registry,
            &mut admissions,
            &token,
            &mut sealed,
            &mut agent,
            &limits.manifest,
            &caller,
        )?;
        let reg = registry
            .agent(config.agent)
            .ok_or_else(|| fail("accepted source unavailable"))?;
        let expected = recipe::STEPS[index].1;
        if index < 3 {
            let pending = super::execution::pending(reg.snapshot(), &caller)?;
            if Some(pending.tool_id()) != expected {
                return Err(fail(
                    "guest boundary differs from approved script; no substitution",
                ));
            }
            let ticket = world.authorize_pending(reg, &admissions, &mut agent, &limits)?;
            world.begin(ticket, reg, &admissions, &mut agent, &limits)?;
            let plan = ScriptedResult {
                outcome: if index == 0 {
                    Outcome::Failed
                } else {
                    Outcome::Succeeded
                },
                result: &[0xa0],
                debit: if index == 2 { 0 } else { 1 },
            };
            world.settle(ticket, reg, &admissions, &mut agent, plan, &limits)?;
        } else if reg.snapshot().state().continuation() != raiv::state::Continuation::AwaitActor {
            return Err(fail("guest did not explicitly Finish; no implicit sleep"));
        }
        // Approval comes from this authorized native owner after known acceptance/settlement.
        // Failed/unknown acknowledgement stops; later execution never guesses a replacement pin.
        pair = world.current_pin(reg, &admissions, &mut agent, &limits)?;
        use std::fmt::Write as _;
        writeln!(&mut output,"checkpoint.{}.root={}\ncheckpoint.{}.revision={}\ncheckpoint.{}.world_digest={}\ncheckpoint.{}.virtual_units_available={}",index+1,reg.snapshot().root(),index+1,reg.snapshot().state().revision(),index+1,super::world::hex(&pair.world_digest),index+1,world.state().available_units())?;
    }
    output.push_str("boundaries=5\nactor_wakes=2\nactions=3\nfresh_adoptions=5\nexecution=synthetic-units-only; no market/portfolio/wake backend\nsleep=scripted-tick-data; no real timer or wake authority\nnext-run=refuses existing namespaces; future mutation requires independent pin approval\n");
    Ok(output)
}
