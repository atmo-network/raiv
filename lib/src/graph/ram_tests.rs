extern crate std;

use super::*;
use crate::{
    activation::{
        prepare, ActivationError, InferencePhase, RamDecision, RamProgress, RequestSequence,
    },
    format::oracle::json_bytes,
    tools::{PlanningContext, ToolFailure, ToolImplementation, ToolSet},
};
use core::sync::atomic::{AtomicUsize, Ordering};
use serde_json::json;

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

static VALIDATE: AtomicUsize = AtomicUsize::new(0);

static PLAN: AtomicUsize = AtomicUsize::new(0);

fn validate(_: &[u8]) -> Result<(), ToolFailure> {
    VALIDATE.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

fn plan(ctx: PlanningContext<'_>, args: &[u8]) -> Result<crate::format::ModelValue, ToolFailure> {
    PLAN.fetch_add(1, Ordering::SeqCst);
    assert!(ctx.memory.is_some());
    crate::format::ModelValue::decode(args, &ctx.limits.decode_limits())
        .map_err(|_| ToolFailure::PlanningFailed)
}

fn invalid(_: PlanningContext<'_>, _: &[u8]) -> Result<crate::format::ModelValue, ToolFailure> {
    PLAN.fetch_add(1, Ordering::SeqCst);
    crate::format::ModelValue::decode(&[0xf6], &DecodeLimits::default())
        .map_err(|_| ToolFailure::PlanningFailed)
}

fn none_plan(ctx: PlanningContext<'_>, _: &[u8]) -> Result<crate::format::ModelValue, ToolFailure> {
    PLAN.fetch_add(1, Ordering::SeqCst);
    assert!(ctx.memory.is_none());
    crate::format::ModelValue::decode(&[0xf6], &DecodeLimits::default())
        .map_err(|_| ToolFailure::PlanningFailed)
}

fn args_plan(
    _: PlanningContext<'_>,
    args: &[u8],
) -> Result<crate::format::ModelValue, ToolFailure> {
    PLAN.fetch_add(1, Ordering::SeqCst);
    crate::format::ModelValue::decode(args, &DecodeLimits::default())
        .map_err(|_| ToolFailure::PlanningFailed)
}

fn funded(snapshot: &RestoredAgent) -> Cbor {
    let mut input = activation::incoming(snapshot);
    let grant = map_mut(&mut input).get_mut("budget_admission").unwrap();
    for (key, value) in [
        ("activation_units", 100),
        ("remaining_episode_units", 100),
        ("remaining_model_calls", 8),
    ] {
        map_mut(grant).insert(key.into(), Cbor::Integer(value.into()));
    }
    input
}

fn frame(request: u64, id: &str, tool: &str, args: Cbor) -> Vec<u8> {
    let mut call = reply::call(id, args);
    map_mut(&mut call).insert("tool_id".into(), Cbor::Text(tool.into()));
    encode(&reply::frame(request, vec![call]))
}

#[test]
fn registered_patch_continue_updates_bounded_trajectory_then_no_call_finish_preserves_memory_without_journal(
) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::MemoryPatch;
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let prefix = run.initial_prefix().to_vec();
    let old = run.snapshot().memory().unwrap().cid();
    let patch_name = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Continue)
        .unwrap()
        .id;
    let key = run.request().unwrap().binding();
    let args = cbor(&json_bytes(
        &json!({"intents":{"live":"accepted RAM"},"tactics":{"live":{"next":"observe"}},"goals":[null]}),
    ));
    let bytes = frame(key.request_id, "patch_1", patch_name, args.clone());
    assert_eq!(run.advance_ram(key, &bytes), Ok(RamProgress::InferAgain));
    assert_eq!(run.request().unwrap().binding().request_id, 2);
    assert_ne!(run.working_memory().unwrap().cid(), old);
    assert_eq!(run.snapshot().memory().unwrap().cid(), old);
    assert_eq!(run.initial_prefix(), prefix);
    assert_eq!(run.usage().model_calls, 2);
    assert_eq!(run.usage().local_tool_calls, 1);
    assert_eq!(run.units().reserved_units(), 3);
    let mut expected = cbor(run.snapshot().memory().unwrap().value_bytes());
    map_mut(&mut expected).insert(
        "intents".into(),
        Cbor::Map(vec![(
            Cbor::Text("live".into()),
            Cbor::Text("accepted RAM".into()),
        )]),
    );
    map_mut(&mut expected).insert(
        "tactics".into(),
        cbor(&json_bytes(&json!({"live":{"next":"observe"}}))),
    );
    assert_eq!(
        run.working_memory().unwrap().value_bytes(),
        encode(&expected)
    );
    let turn = cbor(run.trajectory_bytes());
    let Cbor::Array(turns) = turn else { panic!() };
    assert_eq!(turns.len(), 1);
    let Cbor::Array(results) = field(&turns[0], "results") else {
        panic!()
    };
    let report = field(&results[0], "result");
    assert_eq!(field(report, "memory"), &expected);
    assert_eq!(field(report, "changed"), &Cbor::Bool(true));
    assert_eq!(
        field(report, "processed_planes"),
        &Cbor::Array(vec![
            Cbor::Text("intents".into()),
            Cbor::Text("tactics".into())
        ])
    );
    assert_eq!(
        field(report, "skipped_planes"),
        &cbor(&json_bytes(
            &json!([{"plane":"goals","reason":"unsupported_root"}])
        ))
    );
    assert_eq!(run.session().budget().used().corrective_turns, 0);
    let Cbor::Array(calls) = field(&turns[0], "calls") else {
        panic!()
    };
    assert_eq!(field(&calls[0], "arguments"), &args);
    assert_eq!(field(&results[0], "call_id"), &Cbor::Text("patch_1".into()));
    // No-effect Continue still spends work and keeps the same working buffer/CID.
    let cid = run.working_memory().unwrap().cid();
    let pointer = run.working_memory().unwrap().as_bytes().as_ptr();
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(
            key,
            &frame(
                key.request_id,
                "empty",
                patch_name,
                cbor(&json_bytes(
                    &json!({"intent":{"ignored":1},"Tactics":[null]})
                ))
            )
        ),
        Ok(RamProgress::InferAgain)
    );
    assert_eq!(run.working_memory().unwrap().cid(), cid);
    assert_eq!(run.working_memory().unwrap().as_bytes().as_ptr(), pointer);
    assert_eq!(run.usage().model_calls, 3);
    assert_eq!(run.usage().local_tool_calls, 2);
    assert_eq!(run.units().reserved_units(), 5);
    assert_eq!(run.session().budget().used().corrective_turns, 0);
    let Cbor::Array(turns) = cbor(run.trajectory_bytes()) else {
        panic!()
    };
    let Cbor::Array(results) = field(&turns[1], "results") else {
        panic!()
    };
    assert_eq!(field(&results[0], "ok"), &Cbor::Bool(true));
    let report = field(&results[0], "result");
    assert_eq!(field(report, "memory"), &expected);
    assert_eq!(field(report, "changed"), &Cbor::Bool(false));
    assert_eq!(field(report, "processed_planes"), &Cbor::Array(vec![]));
    assert_eq!(
        field(report, "skipped_planes"),
        &cbor(&json_bytes(
            &json!([{"plane":"intent","reason":"unsupported_root"},{"plane":"Tactics","reason":"unsupported_root"}])
        ))
    );
    // Removing tactics cannot cancel the already accepted AwaitAction instruction.
    let key = run.request().unwrap().binding();
    let deletion = cbor(&json_bytes(&json!({"tactics":null})));
    assert_eq!(
        run.advance_ram(key, &frame(key.request_id, "delete", patch_name, deletion)),
        Ok(RamProgress::InferAgain)
    );
    assert_eq!(
        run.working_memory().unwrap().value_bytes(),
        json_bytes(&json!({"intents":{"live":"accepted RAM"}}))
    );
    assert!(matches!(
        run.snapshot().state().continuation(),
        crate::state::Continuation::AwaitAction { .. }
    ));
    assert_eq!(run.snapshot().root(), root);
    assert_eq!(run.initial_prefix(), prefix);
    let final_memory = run.working_memory().unwrap().cid();
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(key, &encode(&reply::frame(key.request_id, vec![]))),
        Ok(RamProgress::Sealed)
    );
    assert_eq!(run.session().budget().used().corrective_turns, 0);
    assert_eq!(run.usage().local_tool_calls, 3);
    let candidate = run.candidate().unwrap();
    assert_eq!(candidate.snapshot().memory().unwrap().cid(), final_memory);
    assert!(candidate.snapshot().current_episode().is_none());
    assert_eq!(run.session().phase(), InferencePhase::Sealed);
    assert!(run.request().is_err());
    assert!(run.advance_ram(key, &bytes).is_err());
    assert_eq!(run.snapshot().root(), root);
    assert_eq!(run.initial_prefix(), prefix);
    assert_eq!(run.working_memory().unwrap().cid(), final_memory);
    assert_eq!(run.release_sequence().unwrap().last_id(), 4);
    assert_eq!(VALIDATE.load(Ordering::SeqCst), 0);
    assert_eq!(PLAN.load(Ordering::SeqCst), 0);
}

#[test]
fn multiple_unknown_and_invalid_arguments_recover_with_all_native_ids_without_any_callback_or_local_debit(
) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::Pure { validate, plan };
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let before = run.working_memory().unwrap().cid();
    let key = run.request().unwrap().binding();
    let bytes = encode(&reply::frame(
        key.request_id,
        vec![
            reply::call("a", Cbor::Map(vec![])),
            reply::call("b", Cbor::Bytes(vec![1])),
        ],
    ));
    assert_eq!(run.advance_ram(key, &bytes), Ok(RamProgress::InferAgain));
    let Cbor::Array(turns) = cbor(run.trajectory_bytes()) else {
        panic!()
    };
    let Cbor::Array(results) = field(&turns[0], "results") else {
        panic!()
    };
    assert_eq!(results.len(), 2);
    assert_eq!(
        field(&results[0], "error"),
        &Cbor::Text("multiple_tool_calls".into())
    );
    assert_eq!(field(&results[1], "call_id"), &Cbor::Text("b".into()));
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(
            key,
            &frame(
                key.request_id,
                "unknown",
                "not_registered",
                Cbor::Map(vec![])
            )
        ),
        Ok(RamProgress::InferAgain)
    );
    assert_eq!(run.usage().local_tool_calls, 0);
    assert_eq!(run.working_memory().unwrap().cid(), before);
    let key = run.request().unwrap().binding();
    assert!(run
        .advance_ram(
            key,
            &frame(key.request_id, "bad", "not_registered", Cbor::Null)
        )
        .is_err());
    assert_eq!(run.session().phase(), InferencePhase::Failed);
    assert_eq!(VALIDATE.load(Ordering::SeqCst), 0);
    assert_eq!(PLAN.load(Ordering::SeqCst), 0);
    assert_eq!(run.snapshot().root(), root);
}

#[test]
fn pure_continue_and_yield_callbacks_are_admitted_and_pinned_command_identity_ignores_native_ids() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        tool.implementation = match tool.descriptor.control {
            crate::definition::ToolControl::Continue => ToolImplementation::Pure { validate, plan },
            crate::definition::ToolControl::Yield => ToolImplementation::Yield { validate, plan },
            crate::definition::ToolControl::Finish => {
                panic!("Finish is intrinsic, not a registered tool")
            }
        };
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let memory = run.working_memory().unwrap().cid();
    let local = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Continue)
        .unwrap();
    let key = run.request().unwrap().binding();
    let args = cbor(&json_bytes(&json!({"memo":"pure"})));
    assert_eq!(
        run.advance_ram(
            key,
            &frame(key.request_id, "provider local", local.id, args)
        ),
        Ok(RamProgress::InferAgain)
    );
    assert_eq!(run.working_memory().unwrap().cid(), memory);
    let tool = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Yield)
        .unwrap();
    let key = run.request().unwrap().binding();
    let args = cbor(&json_bytes(&json!({"amount":"3"})));
    assert_eq!(
        run.advance_ram(
            key,
            &frame(key.request_id, "not an action ID", tool.id, args.clone())
        )
        .unwrap(),
        RamProgress::Sealed
    );
    let Some(RamDecision::Yield(command)) = run.ram_decision() else {
        panic!()
    };
    let episode = run.snapshot().current_episode().unwrap();
    let expected: [u8; 32] = Sha256::digest(encode(&Cbor::Array(vec![
        Cbor::Text("raiv/action/1".into()),
        Cbor::Bytes(definition.execution_domain().to_vec()),
        Cbor::Bytes(run.snapshot().state().agent_id().to_vec()),
        Cbor::Bytes(episode.id().to_vec()),
        Cbor::Integer(
            run.session()
                .budget()
                .accepted_episode_counts()
                .actions
                .into(),
        ),
    ])))
    .into();
    assert_eq!(command.action_id(), expected);
    assert_eq!(command.tool_id(), tool.id);
    assert_eq!(command.tool_version(), tool.version);
    assert_eq!(command.authority_binding(), tool.authority_binding.unwrap());
    assert_eq!(command.arguments_bytes(), encode(&args));
    assert_eq!(run.session().budget().used().actions, 1);
    assert_eq!(run.session().phase(), InferencePhase::Sealed);
    assert_eq!(VALIDATE.load(Ordering::SeqCst), 2);
    assert_eq!(PLAN.load(Ordering::SeqCst), 2);
}

#[test]
fn failed_yield_planning_does_not_skip_an_action_slot_and_remains_bounded_with_spent_units() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Yield {
            tool.implementation = ToolImplementation::Yield {
                validate,
                plan: invalid,
            };
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let tool = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Yield)
        .unwrap();
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(
            key,
            &frame(key.request_id, "failed", tool.id, Cbor::Map(vec![]))
        ),
        Ok(RamProgress::InferAgain)
    );
    assert_eq!(run.session().budget().used().actions, 0);
    assert_eq!(run.units().reserved_units(), 3);
    assert_eq!(run.session().budget().used().corrective_turns, 1);
    assert_eq!(PLAN.load(Ordering::SeqCst), 1);
    assert_eq!(run.snapshot().root(), root);
}

#[test]
fn small_inbound_patch_can_return_large_memory_but_report_overflow_never_installs_a_draft() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut blocks, root) = restored::memory_profile_snapshot();
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let old_episode = state.current_episode_link().unwrap();
    let episode =
        Episode::decode(blocks.get(old_episode).unwrap(), &DecodeLimits::default()).unwrap();
    let boundaries = episode.boundaries().collect::<Vec<_>>();
    let last = *boundaries.last().unwrap();
    let large = memories::new_memory(&mut blocks, &"x".repeat(2048));
    let last_next = patch(&mut blocks, last, |r| {
        map_mut(r).insert("memory".into(), link(large));
    });
    let episode_next = patch(&mut blocks, old_episode, |r| {
        map_mut(r).insert(
            "boundaries".into(),
            Cbor::Array(
                boundaries
                    .iter()
                    .map(|cid| link(if *cid == last { last_next } else { *cid }))
                    .collect(),
            ),
        );
    });
    let root = patch(&mut blocks, root, |r| {
        map_mut(r).insert("memory".into(), link(large));
        map_mut(r).insert("current_episode".into(), link(episode_next));
    });
    let baseline = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(baseline.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::MemoryPatch;
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let tool = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Continue)
        .unwrap();
    let source = funded(&baseline);
    for (args, overflow) in [
        (json!({"tactics":{"x":1}}), false),
        (json!({"tactics":{"extra":"y".repeat(1800)}}), true),
    ] {
        let bytes = json_bytes(&args);
        let unrestricted =
            crate::memory::PatchArguments::decode(&bytes, &Limits::default()).unwrap();
        let accepted = unrestricted.patched(baseline.memory().unwrap()).unwrap();
        let caller = if overflow {
            Limits {
                max_block_bytes: accepted.as_bytes().len() as u64,
                ..Limits::default()
            }
        } else {
            Limits {
                max_model_reply_bytes: 512,
                ..Limits::default()
            }
        };
        let parsed = crate::memory::PatchArguments::decode(&bytes, &caller).unwrap();
        assert_eq!(
            parsed.patched(baseline.memory().unwrap()).unwrap(),
            accepted
        );
        if overflow {
            assert!(matches!(
                parsed.report(baseline.memory().unwrap(), &accepted),
                Err(crate::memory::PatchError::Codec(
                    crate::format::CodecError::ByteLimit
                ))
            ));
        } else {
            assert!(
                parsed
                    .report(baseline.memory().unwrap(), &accepted)
                    .unwrap()
                    .as_bytes()
                    .len()
                    > 512
            );
        }
        let snapshot = RestoredAgent::restore(root, &blocks, &caller).unwrap();
        let packet = preparation::packet(&snapshot, &source);
        let policy = crate::activation::RunPolicy {
            caller,
            ..activation::policy()
        };
        let mut run = prepare(
            snapshot,
            packet,
            &tools,
            policy,
            RequestSequence::new([93; 32]),
        )
        .ok()
        .unwrap();
        let key = run.request().unwrap().binding();
        let original = run.working_memory().unwrap().as_bytes().to_vec();
        let ptr = run.working_memory().unwrap().as_bytes().as_ptr();
        let prefix = run.initial_prefix().to_vec();
        let input = frame(key.request_id, "patch", tool.id, cbor(&bytes));
        assert!(input.len() as u64 <= caller.max_model_reply_bytes);
        assert_eq!(run.advance_ram(key, &input), Ok(RamProgress::InferAgain));
        assert_eq!(run.usage().local_tool_calls, 1);
        assert_eq!(run.units().reserved_units(), 3);
        assert_eq!(run.initial_prefix(), prefix);
        assert_eq!(run.snapshot().root(), root);
        assert!(run.ram_decision().is_none());
        let Cbor::Array(turns) = cbor(run.trajectory_bytes()) else {
            panic!()
        };
        let Cbor::Array(results) = field(&turns[0], "results") else {
            panic!()
        };
        if overflow {
            assert_eq!(run.working_memory().unwrap().as_bytes(), original);
            assert_eq!(run.working_memory().unwrap().as_bytes().as_ptr(), ptr);
            assert_eq!(run.working_memory().unwrap().cid(), large);
            assert_eq!(run.session().budget().used().corrective_turns, 1);
            assert_eq!(field(&results[0], "ok"), &Cbor::Bool(false));
            assert_eq!(
                field(&results[0], "error"),
                &Cbor::Text(
                    crate::memory::PatchError::Codec(CodecError::ByteLimit)
                        .feedback()
                        .into()
                )
            );
            assert_eq!(field(&results[0], "result"), &Cbor::Null);
        } else {
            assert_eq!(run.working_memory().unwrap(), &accepted);
            assert_eq!(run.session().budget().used().corrective_turns, 0);
            assert_eq!(
                field(field(&results[0], "result"), "memory"),
                &cbor(accepted.value_bytes())
            );
        }
    }
}

#[test]
fn report_embedding_depth_failure_preserves_live_memory_prefix_and_trajectory_after_charging() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::MemoryPatch;
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let tool = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Continue)
        .unwrap();
    let mut value = json!("leaf");
    for _ in 0..23 {
        value = json!({"child":value});
    }
    let bytes = json_bytes(&json!({"tactics":{"deep":value}}));
    let parsed = crate::memory::PatchArguments::decode(&bytes, &Limits::default()).unwrap();
    let draft = parsed.patched(run.working_memory().unwrap()).unwrap();
    let report = parsed
        .report(run.working_memory().unwrap(), &draft)
        .unwrap();
    let embedded = DecodeLimits {
        max_depth: 26,
        ..DecodeLimits::default()
    };
    assert!(crate::format::validate_model_value(&bytes, &embedded).is_ok());
    assert_eq!(
        crate::format::validate_model_value(report.as_bytes(), &embedded),
        Err(crate::format::CodecError::DepthLimit)
    );
    let memory = run.working_memory().unwrap().as_bytes().to_vec();
    let ptr = run.working_memory().unwrap().as_bytes().as_ptr();
    let cid = run.working_memory().unwrap().cid();
    let prefix = run.initial_prefix().to_vec();
    let trajectory = run.trajectory_bytes().to_vec();
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(key, &frame(key.request_id, "deep", tool.id, cbor(&bytes))),
        Err(ActivationError::Codec(
            crate::format::CodecError::DepthLimit
        ))
    );
    assert_eq!(run.session().phase(), InferencePhase::Failed);
    assert_eq!(run.usage().local_tool_calls, 1);
    assert_eq!(run.units().reserved_units(), 2);
    assert_eq!(run.working_memory().unwrap().as_bytes(), memory);
    assert_eq!(run.working_memory().unwrap().as_bytes().as_ptr(), ptr);
    assert_eq!(run.working_memory().unwrap().cid(), cid);
    assert_eq!(run.initial_prefix(), prefix);
    assert_eq!(run.trajectory_bytes(), trajectory);
    assert_eq!(run.snapshot().root(), root);
    assert!(run.ram_decision().is_none());
}

#[test]
fn context_failure_stages_patch_output_without_mutating_ram_or_root_and_stale_keys_do_not_advance()
{
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::MemoryPatch;
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let initial = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let cap = initial.request().unwrap().context_bytes().len() as u64 + 128;
    drop(initial);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &source);
    let policy = crate::activation::RunPolicy {
        caller: Limits {
            max_context_bytes: cap,
            ..Limits::default()
        },
        ..activation::policy()
    };
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        policy,
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let key = run.request().unwrap().binding();
    let memory = run.working_memory().unwrap().cid();
    let pointer = run.working_memory().unwrap().as_bytes().as_ptr();
    let prefix = run.initial_prefix().to_vec();
    let before = run.units().reserved_units();
    assert!(run
        .advance_ram(
            crate::activation::RequestBinding {
                request_id: 0,
                ..key
            },
            &[0xff]
        )
        .is_err());
    assert_eq!(run.request().unwrap().binding(), key);
    assert_eq!(run.units().reserved_units(), before);
    let tool = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Continue)
        .unwrap();
    let args = cbor(&json_bytes(&json!({"intents":{"big":"x".repeat(400)}})));
    assert!(run
        .advance_ram(key, &frame(key.request_id, "big", tool.id, args))
        .is_err());
    assert_eq!(run.session().phase(), InferencePhase::Failed);
    assert_eq!(run.working_memory().unwrap().cid(), memory);
    assert_eq!(run.working_memory().unwrap().as_bytes().as_ptr(), pointer);
    assert_eq!(run.initial_prefix(), prefix);
    assert_eq!(run.snapshot().root(), root);
    assert_eq!(run.usage().local_tool_calls, 1);
}

#[test]
fn memory_disabled_profile_runs_only_registered_pure_continue_with_no_fictitious_memory_or_patch_capability(
) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let fixture = &fixtures()[0];
    let mut blocks = store(fixture);
    let root = root_of(fixture);
    let old = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    assert!(old.memory().is_none());
    let def = patch(&mut blocks, old.state().definition_link(), |record| {
        let tools = cbor(&json_bytes(
            &json!([{"id":"compute","version":1,"control":"continue","description":"Read only compute","arguments_schema":{},"authority_binding":null}]),
        ));
        map_mut(record).insert("tools".into(), tools);
    });
    let root = patch(&mut blocks, root, |record| {
        map_mut(record).insert("definition".into(), link(def));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::Pure {
                validate,
                plan: none_plan,
            };
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(
            key,
            &frame(key.request_id, "compute", "compute", Cbor::Map(vec![]))
        ),
        Ok(RamProgress::InferAgain)
    );
    assert!(run.working_memory().is_none());
    assert!(run.snapshot().memory().is_none());
    assert_eq!(PLAN.load(Ordering::SeqCst), 1);
    assert_eq!(run.usage().local_tool_calls, 1);
    let key = run.request().unwrap().binding();
    assert_eq!(
        run.advance_ram(key, &encode(&reply::frame(key.request_id, vec![]))),
        Ok(RamProgress::Sealed)
    );
}

#[test]
fn new_actor_episode_yield_keeps_full_width_version_and_independent_cbor_domain_identity_without_memory(
) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let fixture = &roles()[0];
    let mut blocks = store(fixture);
    let root = root_of(fixture);
    let old = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let def = patch(&mut blocks, old.state().definition_link(), |record| {
        let Cbor::Array(tools) = map_mut(record).get_mut("tools").unwrap() else {
            panic!()
        };
        for tool in tools {
            if field(tool, "control") == &Cbor::Text("yield".into()) {
                map_mut(tool).insert("version".into(), Cbor::Integer(u64::MAX.into()));
            }
        }
    });
    let root = patch(&mut blocks, root, |record| {
        map_mut(record).insert("definition".into(), link(def));
    });
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    assert!(snapshot.current_episode().is_none());
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Yield {
            tool.implementation = ToolImplementation::Yield {
                validate,
                plan: args_plan,
            };
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let key = run.request().unwrap().binding();
    let tool = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Yield)
        .unwrap();
    assert_eq!(
        run.advance_ram(
            key,
            &frame(
                key.request_id,
                "provider id ignored",
                tool.id,
                Cbor::Map(vec![])
            )
        ),
        Ok(RamProgress::Sealed)
    );
    let Some(RamDecision::Yield(command)) = run.ram_decision() else {
        panic!()
    };
    let episode: [u8; 32] = Sha256::digest(encode(&Cbor::Array(vec![
        Cbor::Text("raiv/episode/1".into()),
        Cbor::Bytes(definition.execution_domain().to_vec()),
        Cbor::Bytes(run.snapshot().state().agent_id().to_vec()),
        field(&source, "event_id").clone(),
    ])))
    .into();
    let action: [u8; 32] = Sha256::digest(encode(&Cbor::Array(vec![
        Cbor::Text("raiv/action/1".into()),
        Cbor::Bytes(definition.execution_domain().to_vec()),
        Cbor::Bytes(run.snapshot().state().agent_id().to_vec()),
        Cbor::Bytes(episode.to_vec()),
        Cbor::Integer(0.into()),
    ])))
    .into();
    assert_eq!(command.action_id(), action);
    assert_eq!(command.tool_version(), u64::MAX);
    assert!(run.working_memory().is_none());
}

#[test]
fn disabled_callbacks_and_removed_finish_tool_are_rejected_before_intrinsic_completion() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VALIDATE.store(0, Ordering::SeqCst);
    PLAN.store(0, Ordering::SeqCst);
    let (blocks, root) = restored::memory_profile_snapshot();
    let baseline = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(baseline.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Continue {
            tool.implementation = ToolImplementation::Pure { validate, plan };
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&baseline);
    let local = definition
        .tools()
        .find(|t| t.control == crate::definition::ToolControl::Continue)
        .unwrap();
    for policy in [
        crate::activation::RunPolicy {
            caller: Limits {
                max_local_tools_per_activation: 0,
                ..Limits::default()
            },
            ..activation::policy()
        },
        crate::activation::RunPolicy {
            quotes: crate::activation::ExecutionQuotes {
                continue_tool: 100,
                ..activation::policy().quotes
            },
            ..activation::policy()
        },
    ] {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let packet = preparation::packet(&snapshot, &source);
        let mut run = prepare(
            snapshot,
            packet,
            &tools,
            policy,
            RequestSequence::new([93; 32]),
        )
        .ok()
        .unwrap();
        assert!(!run
            .request()
            .unwrap()
            .tools()
            .any(|tool| tool.id == local.id));
        let key = run.request().unwrap().binding();
        assert_eq!(
            run.advance_ram(
                key,
                &frame(key.request_id, "disabled", local.id, Cbor::Map(vec![]))
            ),
            Ok(RamProgress::InferAgain)
        );
        assert_eq!(run.usage().local_tool_calls, 0);
        assert_eq!(VALIDATE.load(Ordering::SeqCst), 0);
        assert_eq!(PLAN.load(Ordering::SeqCst), 0);
        let key = run.request().unwrap().binding();
        let args = Cbor::Map(vec![]); // Removed no_op({}) is an unknown tool, not Finish.
        assert_eq!(
            run.advance_ram(key, &frame(key.request_id, "removed_tool", "no_op", args)),
            Ok(RamProgress::InferAgain)
        );
        assert!(run.ram_decision().is_none());
        let Cbor::Array(turns) = cbor(run.trajectory_bytes()) else {
            panic!()
        };
        assert_eq!(
            field(turns.last().unwrap(), "error"),
            &Cbor::Text("unknown_tool".into())
        );
        let key = run.request().unwrap().binding();
        assert_eq!(
            run.advance_ram(key, &encode(&reply::frame(key.request_id, vec![]))),
            Ok(RamProgress::Sealed)
        );
        assert!(matches!(run.ram_decision(), Some(RamDecision::Finish)));
        assert_eq!(run.snapshot().root(), root);
    }
}

#[test]
fn malformed_matched_reply_is_fatal_and_known_cost_overflow_never_becomes_no_op_or_an_estimate() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let source = funded(&snapshot);
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    let key = run.request().unwrap().binding();
    assert!(run.advance_ram(key, &[0xff]).is_err());
    assert_eq!(run.session().phase(), InferencePhase::Failed);
    assert_eq!(run.snapshot().root(), root);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &source);
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    for cost in [u64::MAX, 1] {
        let key = run.request().unwrap().binding();
        let mut body = reply::frame(key.request_id, vec![reply::call("invalid", Cbor::Null)]);
        map_mut(map_mut(&mut body).get_mut("usage").unwrap())
            .insert("billed_units".into(), Cbor::Integer(cost.into()));
        let result = run.advance_ram(key, &encode(&body));
        if cost == u64::MAX {
            assert_eq!(result, Ok(RamProgress::InferAgain));
            assert_eq!(run.usage().billed_units, Some(u64::MAX));
        } else {
            assert_eq!(result, Err(ActivationError::UsageOverflow));
            assert_eq!(run.session().phase(), InferencePhase::Failed);
        }
    }
}
