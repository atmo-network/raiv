use super::*;

fn check_memory(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_memory_consistency()
}

fn text(value: &str) -> Cbor {
    Cbor::Text(value.into())
}

pub(super) fn new_memory(blocks: &mut Store, witness: &str) -> DaslCid {
    let bytes = encode(&Cbor::Map(vec![
        (text("kind"), text("memory")),
        (text("version"), Cbor::Integer(1.into())),
        (
            text("value"),
            Cbor::Map(vec![(
                text("intents"),
                Cbor::Map(vec![(text("witness"), text(witness))]),
            )]),
        ),
    ]));
    let cid = DaslCid::for_block(DaslCodec::Drisl, &bytes);
    blocks.0.insert(cid, bytes);
    cid
}

fn retarget_episode(blocks: &mut Store, root: DaslCid, old: DaslCid, next: DaslCid) -> DaslCid {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let history = state
        .history()
        .map(|cid| if cid == old { next } else { cid })
        .collect::<Vec<_>>();
    let current = state
        .current_episode_link()
        .map(|cid| if cid == old { next } else { cid });
    patch(blocks, root, |record| {
        map_mut(record).insert(
            "history".into(),
            Cbor::Array(history.into_iter().map(link).collect()),
        );
        map_mut(record).insert(
            "current_episode".into(),
            current.map(link).unwrap_or(Cbor::Null),
        );
    })
}

#[test]
fn every_existing_role_identity_and_genesis_graph_keeps_exact_bytes_and_needs_no_new_read() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    let role = roles();
    let actions = identity_graphs();
    let genesis = fixtures();
    for fixture in role
        .as_array()
        .unwrap()
        .iter()
        .chain(actions.as_array().unwrap())
        .chain(genesis.as_array().unwrap())
    {
        let blocks = Counting(store(fixture), Cell::new(0));
        let inspected =
            InspectedClosure::inspect(root_of(fixture), &blocks, &Limits::default()).unwrap();
        let bytes = inspected
            .blocks()
            .map(|(cid, b)| (cid, b.to_vec()))
            .collect::<BTreeMap<_, _>>();
        let reads = blocks.1.get();
        assert_eq!(inspected.check_memory_consistency(), Ok(()));
        assert_eq!(blocks.1.get(), reads);
        assert_eq!(
            bytes,
            inspected
                .blocks()
                .map(|(cid, b)| (cid, b.to_vec()))
                .collect()
        );
    }
}

#[test]
fn disabled_profile_rejects_nonnull_root_episode_or_boundary_memory() {
    let fixture = &roles()[0];
    for location in ["root", "episode", "boundary"] {
        let (mut blocks, episode_cid, episode) = episode_for(fixture);
        let memory = new_memory(&mut blocks, "forbidden");
        let (root, owner) = match location {
            "root" => {
                let root = patch(&mut blocks, root_of(fixture), |r| {
                    map_mut(r).insert("memory".into(), link(memory));
                });
                (root, root)
            }
            "episode" => {
                let next = patch(&mut blocks, episode_cid, |r| {
                    map_mut(r).insert("entry_memory".into(), link(memory));
                });
                (
                    retarget_episode(&mut blocks, root_of(fixture), episode_cid, next),
                    next,
                )
            }
            _ => {
                let boundary = patch(&mut blocks, episode.boundaries().next().unwrap(), |r| {
                    map_mut(r).insert("memory".into(), link(memory));
                });
                let next = patch(&mut blocks, episode_cid, |r| {
                    map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(boundary)]));
                });
                (
                    retarget_episode(&mut blocks, root_of(fixture), episode_cid, next),
                    boundary,
                )
            }
        };
        assert_eq!(identities(&blocks, root), Ok(()));
        assert_eq!(
            check_memory(&blocks, root),
            Err(MemoryConsistencyError::ProfileMismatch(owner).into())
        );
    }
}

#[test]
fn enabled_profile_requires_root_history_current_and_boundary_memory() {
    let fixture = &roles()[2];
    for location in [
        "root",
        "history",
        "current",
        "current_boundary",
        "history_boundary",
    ] {
        let (mut blocks, current_cid, current) = episode_for(fixture);
        let state = AgentState::decode(
            blocks.get(root_of(fixture)).unwrap(),
            &DecodeLimits::default(),
        )
        .unwrap();
        let history = state.history().next().unwrap();
        let (root, owner) = match location {
            "root" => {
                let root = patch(&mut blocks, root_of(fixture), |r| {
                    map_mut(r).insert("memory".into(), Cbor::Null);
                });
                (root, root)
            }
            "history" | "current" => {
                let old = if location == "history" {
                    history
                } else {
                    current_cid
                };
                let next = patch(&mut blocks, old, |r| {
                    map_mut(r).insert("entry_memory".into(), Cbor::Null);
                });
                (
                    retarget_episode(&mut blocks, root_of(fixture), old, next),
                    next,
                )
            }
            _ => {
                let (old, episode) = if location == "history_boundary" {
                    (
                        history,
                        Episode::decode(blocks.get(history).unwrap(), &DecodeLimits::default())
                            .unwrap(),
                    )
                } else {
                    (current_cid, current)
                };
                let boundary = patch(&mut blocks, episode.boundaries().next().unwrap(), |r| {
                    map_mut(r).insert("memory".into(), Cbor::Null);
                });
                let next = patch(&mut blocks, old, |r| {
                    map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(boundary)]));
                });
                (
                    retarget_episode(&mut blocks, root_of(fixture), old, next),
                    boundary,
                )
            }
        };
        assert_eq!(identities(&blocks, root), Ok(()));
        assert_eq!(
            check_memory(&blocks, root),
            Err(MemoryConsistencyError::ProfileMismatch(owner).into())
        );
    }
    for fixture in fixtures().as_array().unwrap() {
        let mut blocks = store(fixture);
        let state = AgentState::decode(
            blocks.get(root_of(fixture)).unwrap(),
            &DecodeLimits::default(),
        )
        .unwrap();
        let replacement = if state.memory_link().is_some() {
            Cbor::Null
        } else {
            let memory = new_memory(&mut blocks, "genesis-profile");
            link(memory)
        };
        let root = patch(&mut blocks, root_of(fixture), |r| {
            map_mut(r).insert("memory".into(), replacement);
        });
        assert_eq!(
            check_memory(&blocks, root),
            Err(MemoryConsistencyError::ProfileMismatch(root).into())
        );
    }
}

#[test]
fn current_entry_and_root_final_memory_bind_exact_cids_not_just_valid_memory_kinds() {
    let fixture = &roles()[2];
    let (mut blocks, cid, _) = episode_for(fixture);
    let other = new_memory(&mut blocks, "different-entry");
    let next = patch(&mut blocks, cid, |r| {
        map_mut(r).insert("entry_memory".into(), link(other));
    });
    let root = retarget_episode(&mut blocks, root_of(fixture), cid, next);
    assert_eq!(
        check_memory(&blocks, root),
        Err(MemoryConsistencyError::EntryMismatch(next).into())
    );
    let mut blocks = store(fixture);
    let other = new_memory(&mut blocks, "different-root");
    let root = patch(&mut blocks, root_of(fixture), |r| {
        map_mut(r).insert("memory".into(), link(other));
    });
    assert_eq!(
        check_memory(&blocks, root),
        Err(MemoryConsistencyError::RootMismatch(root).into())
    );
}

#[test]
fn changed_boundary_memory_is_allowed_and_reuse_has_no_duplicate_payload_or_genesis_invention() {
    let fixture = &roles()[2];
    let (mut blocks, cid, episode) = episode_for(fixture);
    let changed = new_memory(&mut blocks, "accepted-working-change");
    let boundary = patch(&mut blocks, episode.boundaries().next().unwrap(), |r| {
        map_mut(r).insert("memory".into(), link(changed));
    });
    let next = patch(&mut blocks, cid, |r| {
        map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(boundary)]));
    });
    let root = retarget_episode(&mut blocks, root_of(fixture), cid, next);
    let root = patch(&mut blocks, root, |r| {
        map_mut(r).insert("memory".into(), link(changed));
    });
    assert_eq!(check_memory(&blocks, root), Ok(())); // Entry and final memory may differ after working patches.
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let history = state.history().next().unwrap();
    let arbitrary = new_memory(&mut blocks, "earliest-retained-entry");
    let next = patch(&mut blocks, history, |r| {
        map_mut(r).insert("entry_memory".into(), link(arbitrary));
    });
    let root = retarget_episode(&mut blocks, root, history, next);
    let root = patch(&mut blocks, root, |r| {
        map_mut(r).insert("revision".into(), Cbor::Integer(100.into()));
    });
    let inspected = InspectedClosure::inspect(root, &blocks, &Limits::default()).unwrap();
    assert_eq!(inspected.check_memory_consistency(), Ok(())); // No guessed evicted prefix or empty genesis.
    let mut unique = BTreeMap::new();
    for (cid, bytes) in inspected.blocks() {
        assert!(unique.insert(cid, bytes).is_none());
    }
    assert_eq!(unique.len() as u64, inspected.block_count());
}

#[test]
fn completed_history_joins_and_latest_completed_root_are_checked_without_a_current_episode() {
    let fixture = &roles()[2];
    let (mut blocks, _, _) = episode_for(fixture);
    let state = AgentState::decode(
        blocks.get(root_of(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let history = state.history().next().unwrap();
    let first = Episode::decode(blocks.get(history).unwrap(), &DecodeLimits::default()).unwrap();
    let boundary = Boundary::decode(
        blocks.get(first.boundaries().next().unwrap()).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    let definition = Definition::decode(
        blocks.get(state.definition_link()).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let event = [90u8; 32];
    let id: crate::format::Id32 = Sha256::digest(encode(&Cbor::Array(vec![
        text("raiv/episode/1"),
        Cbor::Bytes(definition.execution_domain().to_vec()),
        Cbor::Bytes(state.agent_id().to_vec()),
        Cbor::Bytes(event.to_vec()),
    ])))
    .into();
    let input = patch(&mut blocks, boundary.input_link(), |r| {
        map_mut(r).insert("event_id".into(), Cbor::Bytes(event.to_vec()));
    });
    let final_memory = new_memory(&mut blocks, "second-completed-final");
    let next_boundary = patch(&mut blocks, first.boundaries().next().unwrap(), |r| {
        map_mut(r).insert("input".into(), link(input));
        map_mut(r).insert("memory".into(), link(final_memory));
    });
    let next = patch(&mut blocks, history, |r| {
        map_mut(r).insert("id".into(), Cbor::Bytes(id.to_vec()));
        map_mut(r).insert("trigger_event_id".into(), Cbor::Bytes(event.to_vec()));
        map_mut(r).insert(
            "entry_memory".into(),
            boundary.memory_link().map(link).unwrap(),
        );
        map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(next_boundary)]));
    });
    let root = patch(&mut blocks, root_of(fixture), |r| {
        map_mut(r).insert("current_episode".into(), Cbor::Null);
        map_mut(r).insert(
            "continuation".into(),
            Cbor::Map(vec![(text("tag"), text("await_actor"))]),
        );
        map_mut(r).insert(
            "history".into(),
            Cbor::Array(vec![link(history), link(next)]),
        );
        map_mut(r).insert("memory".into(), link(final_memory));
    });
    assert_eq!(check_memory(&blocks, root), Ok(()));
    let wrong = new_memory(&mut blocks, "wrong-second-entry");
    let bad = patch(&mut blocks, next, |r| {
        map_mut(r).insert("entry_memory".into(), link(wrong));
    });
    let bad_root = retarget_episode(&mut blocks, root, next, bad);
    assert_eq!(
        check_memory(&blocks, bad_root),
        Err(MemoryConsistencyError::EntryMismatch(bad).into())
    );
    let bad_root = patch(&mut blocks, root, |r| {
        map_mut(r).insert("memory".into(), link(wrong));
    });
    assert_eq!(
        check_memory(&blocks, bad_root),
        Err(MemoryConsistencyError::RootMismatch(bad_root).into())
    );
}
