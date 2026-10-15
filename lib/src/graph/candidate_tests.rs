use super::*;
use crate::{
    activation::{
        prepare, Activation, CandidateError, RamDecision, RamProgress, RequestSequence, RunPolicy,
    },
    commit::CommitCandidate,
    definition::ToolControl,
    format::{oracle::json_bytes, ModelValue},
    tools::{PlanningContext, ToolFailure, ToolImplementation, ToolSet},
};
use serde_json::json;

pub(super) fn validate(_: &[u8]) -> Result<(), ToolFailure> {
    Ok(())
}

pub(super) fn plan(ctx: PlanningContext<'_>, args: &[u8]) -> Result<ModelValue, ToolFailure> {
    ModelValue::decode(args, &ctx.limits.decode_limits()).map_err(|_| ToolFailure::PlanningFailed)
}

pub(super) fn incoming(snapshot: &RestoredAgent, event: u8) -> Cbor {
    let mut source = activation::incoming(snapshot);
    map_mut(&mut source).insert("event_id".into(), Cbor::Bytes(vec![event; 32]));
    let grant = map_mut(&mut source).get_mut("budget_admission").unwrap();
    for (key, value) in [
        ("activation_units", 100),
        ("remaining_episode_units", 100),
        ("remaining_model_calls", 16),
    ] {
        map_mut(grant).insert(key.into(), Cbor::Integer(value.into()));
    }
    source
}

pub(super) fn advance(run: &mut Activation<'_>, tool: &str, args: Cbor) -> RamProgress {
    let key = run.request().unwrap().binding();
    let mut call = reply::call("RAM_ONLY_NATIVE_ID", args);
    map_mut(&mut call).insert("tool_id".into(), Cbor::Text(tool.into()));
    let mut frame = reply::frame(key.request_id, vec![call]);
    map_mut(&mut frame).insert("text".into(), Cbor::Text("RAM_ONLY_REPLY_TEXT".into()));
    run.advance_ram(key, &encode(&frame)).unwrap()
}

pub(super) fn finish(run: &mut Activation<'_>) -> RamProgress {
    let key = run.request().unwrap().binding();
    let mut frame = reply::frame(key.request_id, vec![]);
    map_mut(&mut frame).insert("text".into(), Cbor::Text("RAM_ONLY_REPLY_TEXT".into()));
    run.advance_ram(key, &encode(&frame)).unwrap()
}

pub(super) fn verify(
    run: &Activation<'_>,
    candidate: &CommitCandidate,
    input: &Cbor,
    finish: bool,
) {
    let source = run.snapshot();
    assert_eq!(candidate.binding(), run.input().binding());
    assert_eq!(
        candidate.binding().admitted_input_digest,
        <[u8; 32]>::from(Sha256::digest(encode(input)))
    );
    let next = candidate.snapshot();
    assert_eq!(next.state().revision(), source.state().revision() + 1);
    assert_eq!(next.definition(), source.definition());
    assert_eq!(
        next.memory().map(Memory::cid),
        run.working_memory().map(Memory::cid)
    );
    let episode = if finish {
        next.history().last().unwrap()
    } else {
        next.current_episode().unwrap()
    };
    let count = source.current_episode().map_or(0, Episode::boundary_count);
    let boundary = next.boundary(episode.boundaries().last().unwrap()).unwrap();
    assert_eq!(boundary.ordinal(), count);
    assert_eq!(boundary.usage(), run.usage());
    assert_eq!(boundary.input_link(), run.input().input().cid());
    assert_eq!(
        next.input(boundary.input_link()).unwrap().as_bytes(),
        encode(input)
    );
    let old = source.blocks().collect::<BTreeMap<_, _>>();
    let all = candidate.blocks().collect::<BTreeMap<_, _>>();
    assert_eq!(all.len() as u64, next.block_count());
    assert!(!all.contains_key(&source.root()));
    let mut delta = candidate
        .new_blocks()
        .map(|(cid, _)| cid)
        .collect::<Vec<_>>();
    let actual = all
        .keys()
        .filter(|key| !old.contains_key(key))
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(delta, actual);
    delta.sort_unstable();
    assert_eq!(
        candidate
            .new_blocks()
            .map(|(cid, _)| cid)
            .collect::<Vec<_>>(),
        delta
    );
    for (cid, bytes) in candidate.blocks() {
        assert_eq!(DaslCid::for_block(DaslCodec::Drisl, bytes), cid);
        assert_eq!(encode(&cbor(bytes)), bytes); // Independent canonical-byte oracle.
        for forbidden in [
            b"RAM_ONLY_NATIVE_ID".as_slice(),
            b"RAM_ONLY_REPLY_TEXT".as_slice(),
            b"ram_trajectory".as_slice(),
            b"previous_root".as_slice(),
        ] {
            assert!(!bytes
                .windows(forbidden.len())
                .any(|window| window == forbidden));
        }
    }
    let (id, trigger, entry) = source
        .current_episode()
        .map(|e| (e.id(), e.trigger_event_id(), e.entry_memory_link()))
        .unwrap_or_else(|| {
            let id: [u8; 32] = Sha256::digest(encode(&Cbor::Array(vec![
                Cbor::Text("raiv/episode/1".into()),
                Cbor::Bytes(source.definition().execution_domain().to_vec()),
                Cbor::Bytes(source.state().agent_id().to_vec()),
                field(input, "event_id").clone(),
            ])))
            .into();
            (
                id,
                run.input().input().event_id(),
                source.state().memory_link(),
            )
        });
    assert_eq!(episode.id(), id);
    assert_eq!(episode.trigger_event_id(), trigger);
    assert_eq!(episode.entry_memory_link(), entry);
    let memory = run
        .working_memory()
        .map(Memory::cid)
        .map(link)
        .unwrap_or(Cbor::Null);
    let exit = match run.ram_decision().unwrap() {
        RamDecision::Finish => cbor(&json_bytes(&json!({"tag":"await_actor"}))),
        RamDecision::Yield(command) => Cbor::Map(vec![
            (Cbor::Text("tag".into()), Cbor::Text("await_action".into())),
            (Cbor::Text("command".into()), cbor(command.as_bytes())),
        ]),
    };
    let expected_boundary = Cbor::Map(vec![
        (Cbor::Text("kind".into()), Cbor::Text("boundary".into())),
        (Cbor::Text("version".into()), Cbor::Integer(1.into())),
        (Cbor::Text("ordinal".into()), Cbor::Integer(count.into())),
        (Cbor::Text("input".into()), link(run.input().input().cid())),
        (Cbor::Text("memory".into()), memory.clone()),
        (Cbor::Text("exit".into()), exit),
        (
            Cbor::Text("usage".into()),
            Cbor::Map(vec![
                (
                    Cbor::Text("model_calls".into()),
                    Cbor::Integer(run.usage().model_calls.into()),
                ),
                (
                    Cbor::Text("local_tool_calls".into()),
                    Cbor::Integer(run.usage().local_tool_calls.into()),
                ),
                (
                    Cbor::Text("input_tokens".into()),
                    run.usage()
                        .input_tokens
                        .map(|n| Cbor::Integer(n.into()))
                        .unwrap_or(Cbor::Null),
                ),
                (
                    Cbor::Text("output_tokens".into()),
                    run.usage()
                        .output_tokens
                        .map(|n| Cbor::Integer(n.into()))
                        .unwrap_or(Cbor::Null),
                ),
                (
                    Cbor::Text("billed_units".into()),
                    run.usage()
                        .billed_units
                        .map(|n| Cbor::Integer(n.into()))
                        .unwrap_or(Cbor::Null),
                ),
            ]),
        ),
    ]);
    assert_eq!(boundary.as_bytes(), encode(&expected_boundary));
    let mut boundaries = source
        .current_episode()
        .map(|e| e.boundaries().map(link).collect::<Vec<_>>())
        .unwrap_or_default();
    boundaries.push(link(boundary.cid()));
    let expected_episode = Cbor::Map(vec![
        (Cbor::Text("kind".into()), Cbor::Text("episode".into())),
        (Cbor::Text("version".into()), Cbor::Integer(1.into())),
        (Cbor::Text("id".into()), Cbor::Bytes(id.to_vec())),
        (
            Cbor::Text("trigger_event_id".into()),
            Cbor::Bytes(trigger.to_vec()),
        ),
        (
            Cbor::Text("entry_memory".into()),
            entry.map(link).unwrap_or(Cbor::Null),
        ),
        (Cbor::Text("boundaries".into()), Cbor::Array(boundaries)),
        (Cbor::Text("closed".into()), Cbor::Bool(finish)),
    ]);
    assert_eq!(episode.as_bytes(), encode(&expected_episode));
    let mut history = source.state().history().collect::<Vec<_>>();
    if finish {
        history.push(episode.cid());
    }
    let window = source.definition().limits().history_episodes.max(1) as usize;
    if history.len() > window {
        history.drain(..history.len() - window);
    }
    let mut expected_root = cbor(source.state().as_bytes());
    for (key, value) in [
        (
            "revision",
            Cbor::Integer((source.state().revision() + 1).into()),
        ),
        ("memory", memory),
        (
            "current_episode",
            if finish {
                Cbor::Null
            } else {
                link(episode.cid())
            },
        ),
        (
            "history",
            Cbor::Array(history.into_iter().map(link).collect()),
        ),
        (
            "continuation",
            if finish {
                cbor(&json_bytes(&json!({"tag":"await_actor"})))
            } else {
                cbor(&json_bytes(&json!({"tag":"await_action"})))
            },
        ),
    ] {
        map_mut(&mut expected_root).insert(key.into(), value);
    }
    if let Some(bytes) = candidate.command_bytes() {
        let command = cbor(bytes);
        let continuation = map_mut(&mut expected_root).get_mut("continuation").unwrap();
        map_mut(continuation).insert("action_id".into(), field(&command, "action_id").clone());
        let RamDecision::Yield(planned) = run.ram_decision().unwrap() else {
            panic!()
        };
        assert_eq!(bytes, planned.as_bytes());
    }
    assert_eq!(next.state().as_bytes(), encode(&expected_root));
    let store = Store(all.into_iter().map(|(cid, b)| (cid, b.to_vec())).collect());
    assert_eq!(
        RestoredAgent::restore(candidate.new_root(), &store, &Limits::default()).unwrap(),
        *next
    );
}

#[test]
fn finish_and_yield_candidates_for_every_positive_snapshot_match_independent_records_and_restore() {
    let mut snapshots = restored::valid_snapshots();
    snapshots.push(super::super::super::history::completed_pair());
    for (blocks, root) in snapshots {
        for finish in [true, false] {
            let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
            let definition =
                Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default())
                    .unwrap();
            let mut registered = preparation::registrations(&definition);
            for tool in &mut registered {
                if tool.descriptor.control == ToolControl::Yield {
                    tool.implementation = ToolImplementation::Yield { validate, plan };
                }
            }
            let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
            let tool = definition
                .tools()
                .find(|tool| tool.control == ToolControl::Yield);
            if !finish && tool.is_none() {
                continue;
            }
            let input = incoming(&snapshot, 201);
            let packet = preparation::packet(&snapshot, &input);
            let mut run = prepare(
                snapshot,
                packet,
                &tools,
                activation::policy(),
                RequestSequence::new([93; 32]),
            )
            .ok()
            .unwrap();
            assert_eq!(run.candidate(), Err(CandidateError::NotSealed));
            let args = if finish {
                Cbor::Map(vec![])
            } else {
                cbor(&json_bytes(&json!({"amount":"3"})))
            };
            assert_eq!(
                if finish {
                    self::finish(&mut run)
                } else {
                    advance(&mut run, tool.unwrap().id, args)
                },
                RamProgress::Sealed
            );
            let usage = run.usage();
            let units = run.units().reserved_units();
            let prefix = run.initial_prefix().to_vec();
            let candidate = run.candidate().unwrap();
            verify(&run, &candidate, &input, finish);
            assert_eq!(run.candidate().unwrap(), candidate);
            assert_eq!(run.usage(), usage);
            assert_eq!(run.units().reserved_units(), units);
            assert_eq!(run.initial_prefix(), prefix);
            assert_eq!(run.snapshot().root(), root);
            run.release_sequence().unwrap();
            assert_eq!(run.candidate().unwrap(), candidate); // Sequence transfer does not lose sealed data.
        }
    }
}

#[test]
fn patched_memory_is_materialized_and_unchanged_memory_is_reused_without_persisting_ram_turns() {
    for no_effect in [false, true] {
        let (blocks, root) = restored::memory_profile_snapshot();
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let definition =
            Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
        let mut registered = preparation::registrations(&definition);
        for tool in &mut registered {
            if tool.descriptor.control == ToolControl::Continue {
                tool.implementation = ToolImplementation::MemoryPatch;
            }
        }
        let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
        let input = incoming(&snapshot, 202);
        let packet = preparation::packet(&snapshot, &input);
        let mut run = prepare(
            snapshot,
            packet,
            &tools,
            activation::policy(),
            RequestSequence::new([93; 32]),
        )
        .ok()
        .unwrap();
        let patch = definition
            .tools()
            .find(|tool| tool.control == ToolControl::Continue)
            .unwrap()
            .id;
        let args = if no_effect {
            json!({})
        } else {
            json!({"intents":{"keep":"after finish"},"tactics":{"empty":{}}})
        };
        assert_eq!(
            advance(&mut run, patch, cbor(&json_bytes(&args))),
            RamProgress::InferAgain
        );
        assert_eq!(run.candidate(), Err(CandidateError::NotSealed));
        assert_eq!(self::finish(&mut run), RamProgress::Sealed);
        let candidate = run.candidate().unwrap();
        verify(&run, &candidate, &input, true);
        let memory = candidate.snapshot().memory().unwrap();
        assert_eq!(
            candidate.new_blocks().any(|(cid, _)| cid == memory.cid()),
            !no_effect
        );
        assert!(candidate.snapshot().history().last().unwrap().closed());
    }
}

#[test]
fn candidate_materialized_byte_cap_is_checked_for_the_whole_new_closure() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let input = incoming(&snapshot, 204);
    let packet = preparation::packet(&snapshot, &input);
    let caller = Limits {
        max_materialized_bytes: snapshot.encoded_bytes(),
        ..Limits::default()
    };
    let mut run = prepare(
        snapshot,
        packet,
        &tools,
        RunPolicy {
            caller,
            ..activation::policy()
        },
        RequestSequence::new([93; 32]),
    )
    .ok()
    .unwrap();
    assert_eq!(self::finish(&mut run), RamProgress::Sealed);
    let before = run
        .snapshot()
        .blocks()
        .map(|(cid, b)| (cid, b.as_ptr()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        run.candidate(),
        Err(CandidateError::Graph(GraphError::MaterializedByteLimit))
    );
    assert_eq!(
        run.snapshot()
            .blocks()
            .map(|(cid, b)| (cid, b.as_ptr()))
            .collect::<BTreeMap<_, _>>(),
        before
    );
}

#[test]
fn candidate_closure_cap_refusal_preserves_sealed_run_and_no_failed_run_has_candidate() {
    let fixture = &fixtures()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let definition =
        Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default()).unwrap();
    let registered = preparation::registrations(&definition);
    let tools = ToolSet::bind(&definition, &registered, &Limits::default()).unwrap();
    let input = incoming(&snapshot, 203);
    let packet = preparation::packet(&snapshot, &input);
    let caller = Limits {
        max_blocks: snapshot.block_count(),
        ..Limits::default()
    };
    let policy = RunPolicy {
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
    assert_eq!(self::finish(&mut run), RamProgress::Sealed);
    let units = run.units().reserved_units();
    let prefix = run.initial_prefix().to_vec();
    assert_eq!(
        run.candidate(),
        Err(CandidateError::Graph(GraphError::BlockCountLimit))
    );
    assert_eq!(run.units().reserved_units(), units);
    assert_eq!(run.initial_prefix(), prefix);
    assert_eq!(
        run.candidate(),
        Err(CandidateError::Graph(GraphError::BlockCountLimit))
    );
    assert!(run.release_sequence().is_ok());
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    let packet = preparation::packet(&snapshot, &input);
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
    run.inference_error(key).unwrap();
    assert_eq!(run.candidate(), Err(CandidateError::NotSealed));
    run.release_sequence().unwrap();
    assert_eq!(run.candidate(), Err(CandidateError::NotSealed));
}
