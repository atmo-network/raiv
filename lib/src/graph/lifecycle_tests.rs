//! Native scripted checkpoint-DATA recovery. No host acceptance, effects or WASM proof.

use super::*;
use crate::{
    activation::{
        prepare, CandidateError, InferenceError, RamProgress, RequestBinding, RequestSequence,
    },
    format::oracle::json_bytes,
    preparation::{InputCompatibilityError, PrepareError},
    state::Continuation,
    tools::{ToolImplementation, ToolSet},
};
use alloc::string::ToString;
use serde_json::{json, Value};

fn witnesses() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/lifecycle.json")).unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    let text = value["hex"].as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

fn check_golden(actual: &[u8], expected: &Value) {
    assert_eq!(actual, bytes(expected));
    assert_eq!(
        DaslCid::for_block(DaslCodec::Drisl, actual).to_string(),
        expected["cid"]
    );
    assert_eq!(encode(&cbor(actual)), actual);
}

fn recovered_fixture(index: usize) -> (Store, DaslCid) {
    let golden = witnesses();
    let mut blocks = store(&fixtures()[1]);
    for step in &golden["steps"].as_array().unwrap()[..=index] {
        for field in ["input", "memory", "boundary", "episode", "state"] {
            let block = &step[field];
            let cid = block["cid"].as_str().unwrap().parse().unwrap();
            blocks.0.insert(cid, bytes(block));
        }
    }
    (
        blocks,
        golden["steps"][index]["state"]["cid"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
}

fn script(window: u64, fixed_golden: bool) {
    let golden = witnesses();
    let initial = &fixtures()[1];
    let mut blocks = store(initial);
    let mut root = root_of(initial);
    if window != 2 {
        root = definition_change(&mut blocks, root, |r| {
            map_mut(map_mut(r).get_mut("limits").unwrap())
                .insert("history_episodes".into(), Cbor::Integer(window.into()));
        });
    }
    let mut snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        tool.implementation = match tool.descriptor.control {
            crate::definition::ToolControl::Continue => ToolImplementation::MemoryPatch,
            crate::definition::ToolControl::Yield => ToolImplementation::Yield {
                validate: candidate::validate,
                plan: candidate::plan,
            },
            crate::definition::ToolControl::Finish => {
                panic!("Finish is intrinsic, not a registered tool")
            }
        };
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let mut sequence = RequestSequence::new([93; 32]);
    let mut first_episode = None;
    let mut first_action = None;
    let mut closed = Vec::new();
    for (index, step) in golden["steps"].as_array().unwrap().iter().enumerate() {
        let source_root = snapshot.root();
        let previous_id = sequence.last_id();
        let mut input = candidate::incoming(&snapshot, step["event"].as_u64().unwrap() as u8);
        if let Some(outcome) = step["outcome"].as_str() {
            let receipt = map_mut(&mut input).get_mut("receipt").unwrap();
            map_mut(receipt).insert("outcome".into(), Cbor::Text(outcome.into()));
        }
        golden_input(&input, &step["input"]);
        let packet = preparation::packet(&snapshot, &input);
        let mut run = prepare(snapshot, packet, &tools, activation::policy(), sequence)
            .ok()
            .unwrap();
        let pending = run.request().unwrap().binding();
        assert_eq!(pending.request_id, previous_id + 1);
        assert_eq!(pending.instance_id, [93; 32]);
        assert_eq!(run.trajectory_bytes(), &[0x80]); // Rebuild context, never restore old tool messages.
        let prefix = run.initial_prefix().to_vec();
        let accepted = run.session().budget().accepted_episode_counts();
        assert_eq!(
            accepted.model_calls,
            if index <= 3 { (index * 2) as u64 } else { 0 }
        );
        assert_eq!(accepted.actions, if index <= 3 { index as u64 } else { 0 });
        if previous_id != 0 {
            let before = run.usage();
            let units = run.units().reserved_units();
            let stale = RequestBinding {
                request_id: previous_id,
                ..pending
            };
            assert!(matches!(
                run.advance_ram(stale, &[0xff]),
                Err(crate::activation::ActivationError::Inference(
                    InferenceError::WrongRequest
                ))
            ));
            assert_eq!(run.request().unwrap().binding(), pending);
            assert_eq!(run.usage(), before);
            assert_eq!(run.units().reserved_units(), units);
        }
        let before = run.working_memory().unwrap().cid();
        assert_eq!(
            candidate::advance(&mut run, "patch_memory", cbor(&json_bytes(&step["patch"]))),
            RamProgress::InferAgain
        );
        assert_eq!(run.candidate(), Err(CandidateError::NotSealed));
        assert_eq!(run.session().budget().used().corrective_turns, 0);
        let Cbor::Array(turns) = cbor(run.trajectory_bytes()) else {
            panic!()
        };
        let Cbor::Array(results) = field(&turns[0], "results") else {
            panic!()
        };
        assert_eq!(field(&results[0], "ok"), &Cbor::Bool(true));
        let report = field(&results[0], "result");
        assert_eq!(encode(report), bytes(&step["patch_result"]));
        assert_eq!(encode(report), json_bytes(&step["patch_result"]["value"]));
        assert_eq!(run.snapshot().root(), source_root);
        assert_eq!(run.initial_prefix(), prefix);
        assert_eq!(run.request().unwrap().binding().request_id, previous_id + 2);
        assert_eq!(
            run.working_memory().unwrap().value_bytes(),
            json_bytes(&step["memory_value"])
        );
        if index >= 4 {
            assert_eq!(run.working_memory().unwrap().cid(), before);
        }
        let finish = step["tool_id"].is_null();
        assert_eq!(
            match step["tool_id"].as_str() {
                Some(tool) =>
                    candidate::advance(&mut run, tool, cbor(&json_bytes(&step["arguments"]))),
                None => candidate::finish(&mut run),
            },
            RamProgress::Sealed
        );
        assert_eq!(run.usage().model_calls, 2);
        assert_eq!(run.usage().local_tool_calls, 1);
        assert_eq!(run.units().reserved_units(), 4);
        let output = run.candidate().unwrap();
        candidate::verify(&run, &output, &input, finish);
        let next = output.snapshot();
        let episode = if finish {
            next.history().last().unwrap()
        } else {
            next.current_episode().unwrap()
        };
        if index == 0 {
            first_episode = Some(episode.id());
        }
        if index <= 3 {
            assert_eq!(Some(episode.id()), first_episode);
        } else {
            assert_ne!(Some(episode.id()), first_episode);
        }
        if let Some(command) = output.command_bytes() {
            let command = cbor(command);
            let id = field(&command, "action_id").clone();
            if index == 0 {
                first_action = Some(id.clone());
            } else {
                assert_ne!(Some(id), first_action);
            }
        }
        assert_eq!(next.state().revision(), index as u64 + 1);
        if fixed_golden {
            check_golden(next.state().as_bytes(), &step["state"]);
            check_golden(episode.as_bytes(), &step["episode"]);
            check_golden(
                next.boundary(episode.boundaries().last().unwrap())
                    .unwrap()
                    .as_bytes(),
                &step["boundary"],
            );
            check_golden(next.memory().unwrap().as_bytes(), &step["memory"]);
            let mut cids = next
                .blocks()
                .map(|(cid, _)| cid.to_string())
                .collect::<Vec<_>>();
            cids.sort();
            let expected = step["required_cids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            assert_eq!(cids, expected);
            assert_eq!(
                next.encoded_bytes(),
                step["closure_bytes"].as_u64().unwrap()
            );
        }
        if finish {
            closed.push(episode.cid());
        }
        let retained = closed.len().min(window.max(1) as usize);
        assert_eq!(
            next.state().history().collect::<Vec<_>>(),
            closed[closed.len() - retained..]
        );
        assert_eq!(
            HistoryProjection::new(next).completed().count(),
            if window == 0 { 0 } else { retained }
        );
        if index == 3 {
            let actions = HistoryProjection::new(next)
                .completed()
                .last()
                .map(|e| e.actions().count());
            assert_eq!(actions, if window == 0 { None } else { Some(3) });
            assert_eq!(episode.boundary_count(), 4);
            assert_eq!(
                next.memory().unwrap().value_bytes(),
                json_bytes(
                    &json!({"strategy":{"retry":{"rule":"Inspect receipt before retry.","observed":"succeeded_after_retry"}}})
                )
            );
            assert_eq!(output.disposition(), Continuation::AwaitActor);
        }
        if index >= 4 {
            assert_eq!(episode.boundary_count(), 1);
            assert!(episode.closed());
            assert_eq!(episode.entry_memory_link(), next.state().memory_link());
            assert!(!output
                .new_blocks()
                .any(|(cid, _)| cid == next.memory().unwrap().cid()));
        }
        let saved = Store(output.blocks().map(|(cid, b)| (cid, b.to_vec())).collect());
        let recovered =
            RestoredAgent::restore(output.new_root(), &saved, &Limits::default()).unwrap();
        assert_eq!(recovered, *next);
        sequence = run.release_sequence().unwrap();
        assert_eq!(sequence.last_id(), previous_id + 2);
        drop(run);
        drop(output); // No live Activation/prefix/trajectory crosses this recovery.
        snapshot = recovered;
    }
    assert_eq!(sequence.last_id(), 12);
    assert!(snapshot.current_episode().is_none());
}

fn golden_input(input: &Cbor, expected: &Value) {
    check_golden(&encode(input), expected);
}

#[test]
fn scripted_retry_success_reschedule_finish_and_new_wakes_restore_exact_independent_golden_roots() {
    script(2, true);
}

#[test]
fn zero_and_minimum_history_keep_last_checkpoint_without_resurrecting_transcript_or_clearing_memory(
) {
    for window in [0, 1] {
        script(window, false);
    }
}

#[test]
fn disabled_memory_chain_preserves_episode_counts_without_fabricating_patch_tool_or_planes() {
    let fixture = &fixtures()[0];
    let mut snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let mut registered = preparation::registrations(&definition);
    for tool in &mut registered {
        if tool.descriptor.control == crate::definition::ToolControl::Yield {
            tool.implementation = ToolImplementation::Yield {
                validate: candidate::validate,
                plan: candidate::plan,
            };
        }
    }
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    assert!(tools.lookup("patch_memory").is_none());
    let mut sequence = RequestSequence::new([94; 32]);
    let mut episode_id = None;
    let mut action_id = None;
    for (index, tool) in [Some("veto"), Some("pass"), None, None]
        .into_iter()
        .enumerate()
    {
        let mut input = candidate::incoming(&snapshot, 180 + index as u8);
        if index == 1 {
            map_mut(map_mut(&mut input).get_mut("receipt").unwrap())
                .insert("outcome".into(), Cbor::Text("failed".into()));
        }
        let packet = preparation::packet(&snapshot, &input);
        let mut run = prepare(snapshot, packet, &tools, activation::policy(), sequence)
            .ok()
            .unwrap();
        assert_eq!(
            run.request().unwrap().binding().request_id,
            index as u64 + 1
        );
        let accepted = run.session().budget().accepted_episode_counts();
        assert_eq!(
            accepted.model_calls,
            if index < 3 { index as u64 } else { 0 }
        );
        assert_eq!(accepted.actions, if index < 3 { index as u64 } else { 0 });
        assert!(run.working_memory().is_none());
        assert_eq!(
            match tool {
                Some(tool) => candidate::advance(&mut run, tool, Cbor::Map(vec![])),
                None => candidate::finish(&mut run),
            },
            RamProgress::Sealed
        );
        assert_eq!(run.usage().model_calls, 1);
        assert_eq!(run.usage().local_tool_calls, 0);
        let output = run.candidate().unwrap();
        candidate::verify(&run, &output, &input, index >= 2);
        let next = output.snapshot();
        assert!(next.memory().is_none());
        assert!(!next
            .blocks()
            .any(|(_, bytes)| field(&cbor(bytes), "kind") == &Cbor::Text("memory".into())));
        let episode = if index < 2 {
            next.current_episode().unwrap()
        } else {
            next.history().last().unwrap()
        };
        if index == 0 {
            episode_id = Some(episode.id());
        }
        if index < 3 {
            assert_eq!(Some(episode.id()), episode_id);
        } else {
            assert_ne!(Some(episode.id()), episode_id);
        }
        if let Some(bytes) = output.command_bytes() {
            let id = field(&cbor(bytes), "action_id").clone();
            if index == 0 {
                action_id = Some(id);
            } else {
                assert_ne!(Some(id), action_id);
            }
        }
        let saved = Store(output.blocks().map(|(cid, b)| (cid, b.to_vec())).collect());
        let recovered =
            RestoredAgent::restore(output.new_root(), &saved, &Limits::default()).unwrap();
        sequence = run.release_sequence().unwrap();
        drop(run);
        drop(output);
        snapshot = recovered;
    }
    assert_eq!(sequence.last_id(), 4);
    assert_eq!(snapshot.state().history_count(), 2);
}

#[test]
fn restored_pending_input_phase_receipt_and_known_event_replay_fail_before_spending_sequence() {
    let (blocks, root) = recovered_fixture(0);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    for which in ["actor", "id", "digest"] {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let mut input = candidate::incoming(&snapshot, 200);
        if which == "actor" {
            map_mut(&mut input).insert("cause".into(), Cbor::Text("actor_wake".into()));
            map_mut(&mut input).insert("receipt".into(), Cbor::Null);
        } else {
            let receipt = map_mut(&mut input).get_mut("receipt").unwrap();
            map_mut(receipt).insert(
                if which == "id" {
                    "action_id"
                } else {
                    "command_digest"
                }
                .into(),
                Cbor::Bytes(vec![88; 32]),
            );
        }
        let packet = preparation::packet(&snapshot, &input);
        let error = prepare(
            snapshot,
            packet,
            &tools,
            activation::policy(),
            RequestSequence::new([93; 32]),
        )
        .err()
        .unwrap();
        let expected = if which == "actor" {
            InputCompatibilityError::WrongPhase
        } else {
            InputCompatibilityError::ReceiptMismatch
        };
        assert_eq!(
            error.reason,
            crate::activation::ActivationError::Prepared(PrepareError::Compatibility(expected))
        );
        assert_eq!(error.sequence.last_id(), 0);
    }
    let (blocks, root) = recovered_fixture(3);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let input = candidate::incoming(&snapshot, 170);
    let packet = preparation::packet(&snapshot, &input);
    let error = prepare(
        snapshot,
        packet,
        &tools,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .err()
    .unwrap();
    assert_eq!(
        error.reason,
        crate::activation::ActivationError::Prepared(PrepareError::KnownEventReplay)
    );
    assert_eq!(error.sequence.last_id(), 0);
}

#[test]
fn fixed_checkpoint_recovery_requires_every_exact_block_and_never_substitutes_genesis() {
    let (blocks, root) = recovered_fixture(3);
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let required = snapshot.blocks().map(|(cid, _)| cid).collect::<Vec<_>>();
    for cid in required {
        let mut missing = Store(blocks.0.clone());
        missing.0.remove(&cid);
        assert!(RestoredAgent::restore(root, &missing, &Limits::default()).is_err());
        let mut corrupt = Store(blocks.0.clone());
        corrupt.0.get_mut(&cid).unwrap()[0] ^= 1;
        assert!(RestoredAgent::restore(root, &corrupt, &Limits::default()).is_err());
    }
}
