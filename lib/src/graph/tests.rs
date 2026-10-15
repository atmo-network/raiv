use super::*;

#[path = "inspection_tests.rs"]
mod inspection;
#[path = "role_tests.rs"]
mod roles;

use crate::definition::{DefinitionSpec, MemoryProfile};
use crate::format::DaslCodec;
use alloc::{collections::BTreeMap, vec, vec::Vec};
use serde_json::Value;

struct Store(BTreeMap<DaslCid, Vec<u8>>);

impl MaterializedBlocks for Store {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.0.get(&cid).map(Vec::as_slice)
    }
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/genesis.json")).unwrap()
}

fn store(fixture: &Value) -> Store {
    Store(
        fixture["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["cid"].as_str().unwrap().parse().unwrap(),
                    hex(b["hex"].as_str().unwrap()),
                )
            })
            .collect(),
    )
}

fn root(fixture: &Value) -> DaslCid {
    fixture["root"].as_str().unwrap().parse().unwrap()
}

fn parts(fixture: &Value) -> (Definition, Option<Memory>) {
    let find = |kind| {
        fixture["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["kind"] == kind)
            .map(|b| hex(b["hex"].as_str().unwrap()))
    };
    (
        Definition::decode(&find("definition").unwrap(), &DecodeLimits::default()).unwrap(),
        find("memory").map(|bytes| Memory::decode(&bytes, &DecodeLimits::default()).unwrap()),
    )
}

fn build(fixture: &Value, limits: &Limits) -> Result<Genesis, GraphError> {
    let (definition, memory) = parts(fixture);
    Genesis::build(
        hex(fixture["agent_id"].as_str().unwrap())
            .try_into()
            .unwrap(),
        definition,
        memory,
        limits,
    )
}

fn altered_root(store: &mut Store, spec: &AgentStateSpec<'_>) -> DaslCid {
    let state = AgentState::encode(spec, &DecodeLimits::default()).unwrap();
    let cid = state.cid();
    store.0.insert(cid, state.as_bytes().to_vec());
    cid
}

fn policy(definition: &Definition, limits: Limits) -> Definition {
    let tools = definition.tools().collect::<Vec<_>>();
    Definition::encode(
        &DefinitionSpec {
            execution_domain: definition.execution_domain(),
            policy: definition.policy(),
            mandate: definition.mandate(),
            memory_profile: definition.memory_profile(),
            tools: &tools,
            inference_profile: definition.inference_profile_bytes(),
            limits,
        },
        &DecodeLimits::default(),
    )
    .unwrap()
}

#[test]
fn both_complete_genesis_closures_match_independent_roots_and_build_restore_exactly() {
    for fixture in fixtures().as_array().unwrap() {
        let store = store(fixture);
        for (cid, bytes) in &store.0 {
            cid.verify(bytes).unwrap();
        }
        let restored = Genesis::restore(root(fixture), &store, &Limits::default()).unwrap();
        let built = build(fixture, &Limits::default()).unwrap();
        assert_eq!(restored, built);
        assert_eq!(built.root(), root(fixture));
        assert_eq!(
            built
                .blocks()
                .map(|(cid, b)| (cid, b.to_vec()))
                .collect::<BTreeMap<_, _>>(),
            store.0
        );
        assert_eq!(built.state().revision(), 0);
        assert_eq!(built.state().continuation(), Continuation::AwaitActor);
        assert_eq!(built.state().history_count(), 0);
        assert!(built.state().current_episode_link().is_none());
        assert_eq!(
            built.definition().memory_profile() == MemoryProfile::IntentsStrategyTactics,
            built.memory().is_some()
        );
        assert_eq!(
            built.blocks().map(|(_, b)| b.len() as u64).sum::<u64>(),
            fixture["closure_bytes"].as_u64().unwrap()
        );
        assert_eq!(build(fixture, &Limits::default()).unwrap(), built);
    }
}

#[test]
fn every_missing_or_corrupt_required_block_fails_without_replacement_or_partial_success() {
    for fixture in fixtures().as_array().unwrap() {
        let source = store(fixture);
        for cid in source.0.keys() {
            let mut absent = store(fixture);
            absent.0.remove(cid);
            assert_eq!(
                Genesis::restore(root(fixture), &absent, &Limits::default()),
                Err(GraphError::InputUnavailable(*cid))
            );
            let mut corrupt = store(fixture);
            corrupt.0.get_mut(cid).unwrap()[0] ^= 1;
            assert_eq!(
                Genesis::restore(root(fixture), &corrupt, &Limits::default()),
                Err(GraphError::IntegrityFailure(*cid))
            );
        }
    }
}

#[test]
fn verified_bytes_still_require_correct_typed_targets_and_root_codec() {
    let fixture = &fixtures()[1];
    let source = Genesis::restore(root(fixture), &store(fixture), &Limits::default()).unwrap();
    let mut blocks = store(fixture);
    let wrong = altered_root(
        &mut blocks,
        &AgentStateSpec {
            agent_id: source.state().agent_id(),
            revision: 0,
            definition: source.memory().unwrap().cid(),
            memory: source.state().memory_link(),
            continuation: Continuation::AwaitActor,
            current_episode: None,
            history: &[],
        },
    );
    assert_eq!(
        Genesis::restore(wrong, &blocks, &Limits::default()),
        Err(GraphError::Definition(DefinitionError::InvalidSchema))
    );
    let wrong = altered_root(
        &mut blocks,
        &AgentStateSpec {
            agent_id: source.state().agent_id(),
            revision: 0,
            definition: source.definition().cid(),
            memory: Some(source.definition().cid()),
            continuation: Continuation::AwaitActor,
            current_episode: None,
            history: &[],
        },
    );
    assert_eq!(
        Genesis::restore(wrong, &blocks, &Limits::default()),
        Err(GraphError::Memory(MemoryError::InvalidSchema))
    );
    let raw = DaslCid::from_digest(DaslCodec::Raw, root(fixture).digest());
    assert_eq!(
        Genesis::restore(raw, &blocks, &Limits::default()),
        Err(GraphError::Codec(CodecError::InvalidLink))
    );
}

#[test]
fn memory_presence_matches_definition_profile_in_both_build_and_restore() {
    let fixtures = fixtures();
    let (none, _) = parts(&fixtures[0]);
    let (_, memory) = parts(&fixtures[1]);
    assert_eq!(
        Genesis::build([0xb0; 32], none, memory, &Limits::default()),
        Err(GraphError::MemoryProfileMismatch)
    );
    let (enabled, _) = parts(&fixtures[1]);
    assert_eq!(
        Genesis::build([0xb0; 32], enabled, None, &Limits::default()),
        Err(GraphError::MemoryProfileMismatch)
    );
    for index in [0, 1] {
        let source = build(&fixtures[index], &Limits::default()).unwrap();
        let mut blocks = store(&fixtures[index]);
        let memory = if index == 0 {
            parts(&fixtures[1]).1.map(|memory| memory.cid())
        } else {
            None
        };
        let root = altered_root(
            &mut blocks,
            &AgentStateSpec {
                agent_id: source.state().agent_id(),
                revision: 0,
                definition: source.definition().cid(),
                memory,
                continuation: Continuation::AwaitActor,
                current_episode: None,
                history: &[],
            },
        );
        assert_eq!(
            Genesis::restore(root, &blocks, &Limits::default()),
            Err(GraphError::MemoryProfileMismatch)
        );
    }
}

#[test]
fn post_genesis_roots_are_explicitly_rejected_before_unavailable_episode_reads() {
    let fixture = &fixtures()[0];
    let source = build(fixture, &Limits::default()).unwrap();
    let episode = DaslCid::from_digest(DaslCodec::Drisl, [1; 32]);
    for (revision, continuation, current, history) in [
        (1, Continuation::AwaitActor, None, vec![]),
        (0, Continuation::AwaitActor, None, vec![episode]),
        (
            0,
            Continuation::AwaitAction { action_id: [2; 32] },
            Some(episode),
            vec![],
        ),
    ] {
        let mut blocks = store(fixture);
        let root = altered_root(
            &mut blocks,
            &AgentStateSpec {
                agent_id: source.state().agent_id(),
                revision,
                definition: source.definition().cid(),
                memory: None,
                continuation,
                current_episode: current,
                history: &history,
            },
        );
        assert_eq!(
            Genesis::restore(root, &blocks, &Limits::default()),
            Err(GraphError::NotGenesis)
        );
    }
}

#[test]
fn bootstrap_count_aggregate_per_block_and_memory_limits_bound_both_paths() {
    for fixture in fixtures().as_array().unwrap() {
        let blocks = store(fixture);
        let source = build(fixture, &Limits::default()).unwrap();
        let exact = Limits {
            max_blocks: blocks.0.len() as u64,
            max_materialized_bytes: fixture["closure_bytes"].as_u64().unwrap(),
            max_block_bytes: blocks.0.values().map(|b| b.len() as u64).max().unwrap(),
            max_memory_bytes: source
                .memory()
                .map(|m| m.as_bytes().len() as u64)
                .unwrap_or(0),
            ..Limits::default()
        };
        assert_eq!(
            Genesis::restore(root(fixture), &blocks, &exact).unwrap(),
            source
        );
        assert_eq!(build(fixture, &exact).unwrap(), source);
        let short = Limits {
            max_blocks: exact.max_blocks - 1,
            ..exact
        };
        assert_eq!(
            Genesis::restore(root(fixture), &blocks, &short),
            Err(GraphError::BlockCountLimit)
        );
        assert_eq!(build(fixture, &short), Err(GraphError::BlockCountLimit));
        let short = Limits {
            max_materialized_bytes: exact.max_materialized_bytes - 1,
            ..exact
        };
        assert_eq!(
            Genesis::restore(root(fixture), &blocks, &short),
            Err(GraphError::MaterializedByteLimit)
        );
        assert_eq!(
            build(fixture, &short),
            Err(GraphError::MaterializedByteLimit)
        );
        let short = Limits {
            max_block_bytes: exact.max_block_bytes - 1,
            ..exact
        };
        assert!(Genesis::restore(root(fixture), &blocks, &short).is_err());
        assert!(build(fixture, &short).is_err());
        if exact.max_memory_bytes != 0 {
            let short = Limits {
                max_memory_bytes: exact.max_memory_bytes - 1,
                ..exact
            };
            assert!(Genesis::restore(root(fixture), &blocks, &short).is_err());
            assert!(build(fixture, &short).is_err());
        }
    }
}

#[test]
fn linked_policy_only_tightens_bootstrap_resource_limits() {
    for fixture in fixtures().as_array().unwrap() {
        let (definition, memory) = parts(fixture);
        let changed = policy(
            &definition,
            Limits {
                max_blocks: 1,
                ..*definition.limits()
            },
        );
        let source = build(fixture, &Limits::default()).unwrap();
        let mut blocks = store(fixture);
        blocks.0.insert(changed.cid(), changed.as_bytes().to_vec());
        let root = altered_root(
            &mut blocks,
            &AgentStateSpec {
                agent_id: source.state().agent_id(),
                revision: 0,
                definition: changed.cid(),
                memory: source.state().memory_link(),
                continuation: Continuation::AwaitActor,
                current_episode: None,
                history: &[],
            },
        );
        assert_eq!(
            Genesis::restore(root, &blocks, &Limits::default()),
            Err(GraphError::BlockCountLimit)
        );
        assert_eq!(
            Genesis::build([0xb0; 32], changed, memory, &Limits::default()),
            Err(GraphError::BlockCountLimit)
        );
        let changed = policy(
            &definition,
            Limits {
                max_materialized_bytes: 1,
                ..*definition.limits()
            },
        );
        let mut blocks = store(fixture);
        blocks.0.insert(changed.cid(), changed.as_bytes().to_vec());
        let root = altered_root(
            &mut blocks,
            &AgentStateSpec {
                agent_id: source.state().agent_id(),
                revision: 0,
                definition: changed.cid(),
                memory: source.state().memory_link(),
                continuation: Continuation::AwaitActor,
                current_episode: None,
                history: &[],
            },
        );
        assert_eq!(
            Genesis::restore(root, &blocks, &Limits::default()),
            Err(GraphError::MaterializedByteLimit)
        );
        let (_, memory) = parts(fixture);
        assert_eq!(
            Genesis::build([0xb0; 32], changed, memory, &Limits::default()),
            Err(GraphError::MaterializedByteLimit)
        );
    }
}

#[test]
fn learned_wire_limits_recheck_already_decoded_root_and_definition() {
    let fixture = &fixtures()[0];
    let source = build(fixture, &Limits::default()).unwrap();
    for (limits, error) in [
        (
            Limits {
                max_block_bytes: 1,
                ..Limits::default()
            },
            CodecError::ByteLimit,
        ),
        (
            Limits {
                max_depth: 1,
                ..Limits::default()
            },
            CodecError::DepthLimit,
        ),
        (
            Limits {
                max_collection_entries: 1,
                ..Limits::default()
            },
            CodecError::CollectionLimit,
        ),
    ] {
        let (definition, _) = parts(fixture);
        let changed = policy(&definition, limits);
        let mut blocks = store(fixture);
        blocks.0.insert(changed.cid(), changed.as_bytes().to_vec());
        let root = altered_root(
            &mut blocks,
            &AgentStateSpec {
                agent_id: source.state().agent_id(),
                revision: 0,
                definition: changed.cid(),
                memory: None,
                continuation: Continuation::AwaitActor,
                current_episode: None,
                history: &[],
            },
        );
        assert_eq!(
            Genesis::restore(root, &blocks, &Limits::default()),
            Err(GraphError::Codec(error))
        );
        assert_eq!(
            Genesis::build([0xb0; 32], changed, None, &Limits::default()),
            Err(GraphError::Codec(error))
        );
    }
}

#[test]
fn zero_block_budget_prevents_store_access_and_unrelated_data_is_not_a_hidden_closure() {
    struct NeverRead;
    impl MaterializedBlocks for NeverRead {
        fn get(&self, _: DaslCid) -> Option<&[u8]> {
            panic!("no lookup permitted")
        }
    }
    let fixture = &fixtures()[0];
    assert_eq!(
        Genesis::restore(
            root(fixture),
            &NeverRead,
            &Limits {
                max_blocks: 0,
                ..Limits::default()
            }
        ),
        Err(GraphError::BlockCountLimit)
    );
    let mut blocks = store(fixture);
    let unrelated = vec![0xff; 100_000];
    blocks
        .0
        .insert(DaslCid::for_block(DaslCodec::Drisl, &unrelated), unrelated);
    let caps = Limits {
        max_blocks: 2,
        max_materialized_bytes: fixture["closure_bytes"].as_u64().unwrap(),
        ..Limits::default()
    };
    assert_eq!(
        Genesis::restore(root(fixture), &blocks, &caps).unwrap(),
        build(fixture, &caps).unwrap()
    );
}
