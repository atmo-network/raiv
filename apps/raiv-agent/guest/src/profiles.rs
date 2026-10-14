//! Compiled scripted/live-demo metadata/tool bindings, not host authorization or live pricing.
mod chat;
use super::AbiError;
pub use chat::{opencode_definition, opencode_reasoning_definition, ChatProfile, ReasoningEffort};
use raiv::{
    activation::{ExecutionQuotes, RunPolicy},
    definition::{
        Definition, DefinitionError, DefinitionSpec, MemoryProfile, ToolControl, ToolDescriptorView,
    },
    format::ModelValue,
    limits::Limits,
    tools::{PlanningContext, ToolFailure, ToolImplementation, ToolRegistration},
};

const SURVIVOR_POLICY: &str = "Fixture local-trusted policy. Editable Memory has optional intents (chosen goals, commitments, unfinished tasks and completion conditions), strategy (approaches, reasons, uncertainties and revision conditions), and tactics (situational implementation ideas and conditional continuations). These exact case-sensitive names provide low-cost semantic discipline, not a guarantee of good reasoning. Patch arguments are the object itself, not paths, operations or a result envelope. Under selected supported roots, objects merge, null deletes object fields, arrays replace wholly and contain no null at any depth. Empty object fields normalize away; array positions and false/0/empty text/empty arrays remain. Unsupported top-level names are skipped with explicit unsupported_root reasons, never translated or materialized; raw skipped data still obeys all model-data and resource bounds. Invalid selected content rejects the selected patch without mutation; no nested salvage. The advertised closed root schema describes intended vocabulary, not reject-all-unknown handling. Read the full returned memory, changed, processed_planes and skipped_planes; processed does not imply changed. Partial/all-skipped/no-effect stays Continue without forced correction, replay or Finish. Absent supported planes may be created. No references or cross-plane cascades. Tactics do not cancel accepted Commands or establish success. Memory is separate from mandate, observations, Receipts, phase, budgets and authority. Patching stays in RAM; a complete response without tool calls finishes an Episode without clearing Memory.";
const OBJECT: &[u8] = b"\xa2dtypefobjectjproperties\xa0";
const PATCH:&[u8]=b"\xa3dtypefobjectjproperties\xa3gintents\xa1dtype\x82fobjectdnullgtactics\xa1dtype\x82fobjectdnullhstrategy\xa1dtype\x82fobjectdnulltadditionalProperties\xf4";
const SCRIPTED: &[u8] = b"\xa3emodelmfixture-modelhproviderhscriptedhthinking\xf4";
const fn descriptor(
    id: &'static str,
    description: &'static str,
    control: ToolControl,
    authority: Option<[u8; 32]>,
    schema: &'static [u8],
) -> ToolDescriptorView<'static> {
    ToolDescriptorView {
        id,
        version: 1,
        control,
        description,
        arguments_schema: schema,
        authority_binding: authority,
    }
}
fn validate(arguments: &[u8]) -> Result<(), ToolFailure> {
    minicbor::Decoder::new(arguments)
        .map()
        .map_err(|_| ToolFailure::InvalidArguments)?;
    Ok(())
}
fn plan(context: PlanningContext<'_>, arguments: &[u8]) -> Result<ModelValue, ToolFailure> {
    // These schemas permit arbitrary bounded model-map fields. Planning only preserves
    // arguments; an eventual simulator must validate effects and produce a real Receipt.
    ModelValue::decode(arguments, &context.limits.decode_limits())
        .map_err(|_| ToolFailure::PlanningFailed)
}
const YIELD: ToolImplementation = ToolImplementation::Yield { validate, plan };
static VETO: [ToolRegistration<'static>; 2] = [
    ToolRegistration {
        descriptor: descriptor(
            "veto",
            "Fixture veto operation.",
            ToolControl::Yield,
            Some([0x11; 32]),
            OBJECT,
        ),
        implementation: YIELD,
    },
    ToolRegistration {
        descriptor: descriptor(
            "pass",
            "Fixture pass operation.",
            ToolControl::Yield,
            Some([0x22; 32]),
            OBJECT,
        ),
        implementation: YIELD,
    },
];
static SURVIVOR:[ToolRegistration<'static>;3]=[
    ToolRegistration{descriptor:descriptor("patch_memory","Patch the object itself using case-sensitive intents, strategy and tactics. Unsupported roots are skipped with unsupported_root feedback, never remapped. Selected roots must be object/null: objects merge, null deletes, arrays replace wholly and forbid nested null; empty object fields normalize away. Result contains full memory, changed, processed_planes and skipped_planes reasons; processed does not imply changed. Partial/all-skipped/no-effect remains Continue without forced retry. Absent supported planes may be created. Persisted Memory is null-free; no checkpoint or authority change.",ToolControl::Continue,None,PATCH),implementation:ToolImplementation::MemoryPatch},
    ToolRegistration{descriptor:descriptor("trade","Fixture trade operation.",ToolControl::Yield,Some([0x33;32]),OBJECT),implementation:YIELD},
    ToolRegistration{descriptor:descriptor("set_wake","Fixture set_wake operation.",ToolControl::Yield,Some([0x44;32]),OBJECT),implementation:YIELD},
];
/// Scripted profile metadata DATA for explicit application bootstrap, not approval.
/// Constructors reuse the same exact compiled registrations as real guest activation.
pub fn scripted_definition(profile: MemoryProfile) -> Result<Definition, DefinitionError> {
    let (code, policy, mandate): (&[ToolRegistration<'static>], &str, &str) = match profile {
        MemoryProfile::None => (
            &VETO,
            "Fixture local-trusted policy.",
            "Synthetic veto-definition mandate.",
        ),
        MemoryProfile::IntentsStrategyTactics => (
            &SURVIVOR,
            SURVIVOR_POLICY,
            "Synthetic survivor-definition mandate.",
        ),
    };
    build(profile, code, policy, mandate, SCRIPTED)
}
fn build(
    profile: MemoryProfile,
    code: &[ToolRegistration<'static>],
    policy: &str,
    mandate: &str,
    inference: &[u8],
) -> Result<Definition, DefinitionError> {
    let mut tools = alloc::vec::Vec::new();
    tools
        .try_reserve_exact(code.len())
        .map_err(|_| DefinitionError::Codec(raiv::format::CodecError::AllocationFailure))?;
    tools.extend(code.iter().map(|tool| tool.descriptor));
    Definition::encode(
        &DefinitionSpec {
            execution_domain: [1; 32],
            policy,
            mandate,
            memory_profile: profile,
            tools: &tools,
            inference_profile: inference,
            limits: Limits::default(),
        },
        &Limits::default().decode_limits(),
    )
}
pub fn registrations(
    definition: &Definition,
) -> Result<&'static [ToolRegistration<'static>], AbiError> {
    if definition.inference_profile_bytes() != SCRIPTED {
        ChatProfile::decode(definition.inference_profile_bytes())
            .map_err(|_| AbiError::UnsupportedProfile)?;
        return Ok(chat::registrations(definition.memory_profile()));
    }
    Ok(match definition.memory_profile() {
        MemoryProfile::None => &VETO,
        MemoryProfile::IntentsStrategyTactics => &SURVIVOR,
    })
}
pub fn policy() -> RunPolicy {
    RunPolicy {
        caller: Limits::default(),
        quotes: ExecutionQuotes {
            model: 1,
            continue_tool: 1,
            yield_tool: 1,
            finish: 1,
        },
    }
}
