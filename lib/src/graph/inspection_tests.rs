use super::*;
use core::cell::Cell;

fn closures() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/closure.json")).unwrap()
}
struct CountingStore {
    blocks: Store,
    calls: Cell<u64>,
}
impl MaterializedBlocks for CountingStore {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.calls.set(self.calls.get() + 1);
        self.blocks.get(cid)
    }
}
fn changed_definition(blocks: &mut Store, root: DaslCid, limits: Limits) -> DaslCid {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let definition = Definition::decode(
        blocks.get(state.definition_link()).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let changed = policy(&definition, limits);
    blocks.0.insert(changed.cid(), changed.as_bytes().to_vec());
    altered_root(
        blocks,
        &AgentStateSpec {
            agent_id: state.agent_id(),
            revision: state.revision(),
            definition: changed.cid(),
            memory: state.memory_link(),
            continuation: state.continuation(),
            current_episode: state.current_episode_link(),
            history: &state.history().collect::<Vec<_>>(),
        },
    )
}

#[test]
fn complete_candidate_and_genesis_closures_inspect_exact_bytes_once_without_semantic_acceptance() {
    let candidates = closures();
    let genesis = fixtures();
    for fixture in candidates
        .as_array()
        .unwrap()
        .iter()
        .chain(genesis.as_array().unwrap())
    {
        let source = CountingStore {
            blocks: store(fixture),
            calls: Cell::new(0),
        };
        let inspected =
            InspectedClosure::inspect(root(fixture), &source, &Limits::default()).unwrap();
        assert_eq!(inspected.root(), root(fixture));
        assert_eq!(inspected.state().cid(), root(fixture));
        assert_eq!(
            inspected.definition().cid(),
            inspected.state().definition_link()
        );
        assert_eq!(
            inspected.block_count(),
            fixture["blocks"].as_array().unwrap().len() as u64
        );
        assert_eq!(
            inspected.encoded_bytes(),
            fixture["closure_bytes"].as_u64().unwrap()
        );
        assert_eq!(source.calls.get(), inspected.block_count());
        assert_eq!(
            inspected
                .blocks()
                .map(|(cid, bytes)| (cid, bytes.to_vec()))
                .collect::<BTreeMap<_, _>>(),
            source.blocks.0
        );
    }
    for fixture in candidates.as_array().unwrap() {
        assert_eq!(
            Genesis::restore(root(fixture), &store(fixture), &Limits::default()),
            Err(GraphError::NotGenesis)
        );
    }
}

#[test]
fn every_missing_or_corrupt_nested_candidate_block_fails_without_partial_inspection() {
    for fixture in closures().as_array().unwrap() {
        let original = store(fixture);
        for cid in original.0.keys() {
            let mut absent = store(fixture);
            absent.0.remove(cid);
            assert_eq!(
                InspectedClosure::inspect(root(fixture), &absent, &Limits::default()),
                Err(GraphError::InputUnavailable(*cid))
            );
            let mut corrupt = store(fixture);
            corrupt.0.get_mut(cid).unwrap()[0] ^= 1;
            assert_eq!(
                InspectedClosure::inspect(root(fixture), &corrupt, &Limits::default()),
                Err(GraphError::IntegrityFailure(*cid))
            );
        }
    }
}

#[test]
fn unique_closure_byte_and_count_caps_are_exact_and_ignore_unrelated_store_bytes() {
    for fixture in closures().as_array().unwrap() {
        let mut source = store(fixture);
        let count = source.0.len() as u64;
        let bytes = fixture["closure_bytes"].as_u64().unwrap();
        source.0.insert(
            DaslCid::from_digest(DaslCodec::Drisl, [0; 32]),
            vec![0xff; 100_000],
        );
        let exact = Limits {
            max_blocks: count,
            max_materialized_bytes: bytes,
            ..Limits::default()
        };
        assert_eq!(
            InspectedClosure::inspect(root(fixture), &source, &exact)
                .unwrap()
                .block_count(),
            count
        );
        assert_eq!(
            InspectedClosure::inspect(
                root(fixture),
                &source,
                &Limits {
                    max_blocks: count - 1,
                    ..exact
                }
            ),
            Err(GraphError::BlockCountLimit)
        );
        assert_eq!(
            InspectedClosure::inspect(
                root(fixture),
                &source,
                &Limits {
                    max_materialized_bytes: bytes - 1,
                    ..exact
                }
            ),
            Err(GraphError::MaterializedByteLimit)
        );
    }
}

#[test]
fn definition_caps_only_tighten_bootstrap_and_recheck_already_decoded_root_and_definition() {
    let fixture = &closures()[0];
    for (limits, error) in [
        (
            Limits {
                max_blocks: 1,
                ..Limits::default()
            },
            GraphError::BlockCountLimit,
        ),
        (
            Limits {
                max_materialized_bytes: 1,
                ..Limits::default()
            },
            GraphError::MaterializedByteLimit,
        ),
        (
            Limits {
                max_block_bytes: 0,
                ..Limits::default()
            },
            GraphError::Codec(CodecError::ByteLimit),
        ),
        (
            Limits {
                max_depth: 0,
                ..Limits::default()
            },
            GraphError::Codec(CodecError::DepthLimit),
        ),
        (
            Limits {
                max_collection_entries: 0,
                ..Limits::default()
            },
            GraphError::Codec(CodecError::CollectionLimit),
        ),
    ] {
        let mut source = store(fixture);
        let changed = changed_definition(&mut source, root(fixture), limits);
        assert_eq!(
            InspectedClosure::inspect(changed, &source, &Limits::default()),
            Err(error)
        );
    }
    let caller = Limits {
        max_blocks: 4,
        ..Limits::default()
    };
    assert_eq!(
        InspectedClosure::inspect(root(fixture), &store(fixture), &caller),
        Err(GraphError::BlockCountLimit)
    );
}

#[test]
fn nested_memory_command_observation_and_receipt_caps_apply_to_inspection() {
    let fixture = &closures()[1];
    for (limits, error) in [
        (
            Limits {
                max_memory_bytes: 0,
                ..Limits::default()
            },
            GraphError::Memory(MemoryError::Codec(CodecError::ByteLimit)),
        ),
        (
            Limits {
                max_command_bytes: 0,
                ..Limits::default()
            },
            GraphError::Boundary(crate::boundary::BoundaryError::Command(
                crate::command::CommandError::Codec(CodecError::ByteLimit),
            )),
        ),
        (
            Limits {
                max_observation_bytes: 0,
                ..Limits::default()
            },
            GraphError::Input(crate::input::InputError::Codec(CodecError::ByteLimit)),
        ),
        (
            Limits {
                max_receipt_bytes: 0,
                ..Limits::default()
            },
            GraphError::Input(crate::input::InputError::Receipt(
                crate::receipt::ReceiptError::Codec(CodecError::ByteLimit),
            )),
        ),
    ] {
        assert_eq!(
            InspectedClosure::inspect(root(fixture), &store(fixture), &limits),
            Err(error)
        );
        let mut source = store(fixture);
        let changed = changed_definition(&mut source, root(fixture), limits);
        assert_eq!(
            InspectedClosure::inspect(changed, &source, &Limits::default()),
            Err(error)
        );
    }
}

#[test]
fn cross_role_cid_reuse_and_hash_valid_wrong_targets_never_pass_deduplication() {
    let fixture = &closures()[1];
    let mut source = store(fixture);
    let state =
        AgentState::decode(source.get(root(fixture)).unwrap(), &DecodeLimits::default()).unwrap();
    let changed = altered_root(
        &mut source,
        &AgentStateSpec {
            agent_id: state.agent_id(),
            revision: state.revision(),
            definition: state.definition_link(),
            memory: state.memory_link(),
            continuation: state.continuation(),
            current_episode: None,
            history: &[state.memory_link().unwrap()],
        },
    );
    assert_eq!(
        InspectedClosure::inspect(changed, &source, &Limits::default()),
        Err(GraphError::TargetKindMismatch(state.memory_link().unwrap()))
    );
    assert!(matches!(
        InspectedClosure::inspect(state.definition_link(), &source, &Limits::default()),
        Err(GraphError::State(_))
    ));
    // A valid Input CID cannot satisfy an Episode link even without a competing role.
    let input = fixture["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "activation_input")
        .unwrap()["cid"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let changed = altered_root(
        &mut source,
        &AgentStateSpec {
            agent_id: state.agent_id(),
            revision: state.revision(),
            definition: state.definition_link(),
            memory: state.memory_link(),
            continuation: state.continuation(),
            current_episode: None,
            history: &[input],
        },
    );
    assert!(matches!(
        InspectedClosure::inspect(changed, &source, &Limits::default()),
        Err(GraphError::Episode(_))
    ));
}

#[test]
fn zero_budget_and_raw_roots_are_rejected_without_lookup() {
    struct NeverRead;
    impl MaterializedBlocks for NeverRead {
        fn get(&self, _: DaslCid) -> Option<&[u8]> {
            panic!("no lookup allowed")
        }
    }
    assert_eq!(
        InspectedClosure::inspect(
            root(&closures()[0]),
            &NeverRead,
            &Limits {
                max_blocks: 0,
                ..Limits::default()
            }
        ),
        Err(GraphError::BlockCountLimit)
    );
    assert_eq!(
        InspectedClosure::inspect(
            DaslCid::from_digest(DaslCodec::Raw, [0; 32]),
            &NeverRead,
            &Limits::default()
        ),
        Err(GraphError::Codec(CodecError::InvalidLink))
    );
}
