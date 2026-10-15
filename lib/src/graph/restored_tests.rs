use super::*;
use crate::graph::RestoredAgent;

pub(super) fn memory_profile_snapshot() -> (Store, DaslCid) {
    let fixture = &roles()[2];
    let blocks = store(fixture);
    let (_, command) = command_at(&blocks, root_of(fixture));
    let (id, _, _) = Command::descriptor_in_validated(&command);
    let state = AgentState::decode(
        blocks.get(root_of(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let definition = Definition::decode(
        blocks.get(state.definition_link()).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let authority = definition
        .tools()
        .find(|tool| tool.id == id)
        .unwrap()
        .authority_binding
        .unwrap();
    let (blocks, root, _) = boundary_change(fixture, 0, |r| {
        let command = map_mut(map_mut(r).get_mut("exit").unwrap())
            .get_mut("command")
            .unwrap();
        map_mut(command).insert("authority_binding".into(), Cbor::Bytes(authority.to_vec()));
    });
    (blocks, root)
}

pub(super) fn valid_snapshots() -> Vec<(Store, DaslCid)> {
    let base = roles();
    let identity = identity_graphs();
    let genesis = fixtures();
    let mut snapshots = base
        .as_array()
        .unwrap()
        .iter()
        .take(1)
        .chain(identity.as_array().unwrap())
        .chain(genesis.as_array().unwrap())
        .map(|fixture| (store(fixture), root_of(fixture)))
        .collect::<Vec<_>>();
    snapshots.push(memory_profile_snapshot());
    snapshots
}

#[test]
fn complete_genesis_and_post_genesis_snapshots_expose_exact_borrowed_domain_views() {
    for (blocks, root) in valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        assert_eq!(snapshot.root(), root);
        assert_eq!(snapshot.state().cid(), root);
        assert_eq!(
            snapshot.definition().cid(),
            snapshot.state().definition_link()
        );
        assert_eq!(
            snapshot.encoded_bytes(),
            snapshot.blocks().map(|(_, b)| b.len() as u64).sum::<u64>()
        );
        assert_eq!(snapshot.block_count(), snapshot.blocks().count() as u64);
        assert_eq!(
            snapshot.current_episode().is_some(),
            snapshot.state().current_episode_link().is_some()
        );
        assert_eq!(
            snapshot.history().count() as u64,
            snapshot.state().history_count()
        );
        assert_eq!(
            snapshot.memory().map(Memory::cid),
            snapshot.state().memory_link()
        );
        let exact = snapshot.blocks().collect::<BTreeMap<_, _>>();
        assert_eq!(snapshot.state().as_bytes().as_ptr(), exact[&root].as_ptr());
        for episode in snapshot.history().chain(snapshot.current_episode()) {
            assert_eq!(episode.as_bytes().as_ptr(), exact[&episode.cid()].as_ptr());
            if let Some(cid) = episode.entry_memory_link() {
                assert_eq!(
                    snapshot.memory_block(cid).unwrap().as_bytes().as_ptr(),
                    exact[&cid].as_ptr()
                );
            }
            for cid in episode.boundaries() {
                let boundary = snapshot.boundary(cid).unwrap();
                assert_eq!(boundary.as_bytes().as_ptr(), exact[&cid].as_ptr());
                let input = snapshot.input(boundary.input_link()).unwrap();
                assert_eq!(input.as_bytes().as_ptr(), exact[&input.cid()].as_ptr());
                if let Some(cid) = boundary.memory_link() {
                    assert_eq!(snapshot.memory_block(cid).unwrap().as_bytes(), exact[&cid]);
                }
            }
        }
        if snapshot.state().revision() == 0 {
            let genesis = Genesis::restore(root, &blocks, &Limits::default()).unwrap();
            assert_eq!(snapshot.state().as_bytes(), genesis.state().as_bytes());
            assert_eq!(
                snapshot
                    .blocks()
                    .map(|(cid, b)| (cid, b.to_vec()))
                    .collect::<BTreeMap<_, _>>(),
                genesis.blocks().map(|(cid, b)| (cid, b.to_vec())).collect()
            );
        } else {
            assert_eq!(
                Genesis::restore(root, &blocks, &Limits::default()),
                Err(GraphError::NotGenesis)
            );
        }
    }
}

#[test]
fn promotion_moves_inspected_buffers_without_new_store_reads_or_payload_allocations() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    for (blocks, root) in valid_snapshots() {
        let blocks = Counting(blocks, Cell::new(0));
        let inspected = InspectedClosure::inspect(root, &blocks, &Limits::default()).unwrap();
        let pointers = inspected
            .blocks()
            .map(|(cid, b)| (cid, (b.as_ptr() as usize, b.len())))
            .collect::<BTreeMap<_, _>>();
        let calls = blocks.1.get();
        let snapshot = RestoredAgent::from_inspected(inspected).unwrap();
        assert_eq!(blocks.1.get(), calls);
        assert_eq!(
            pointers,
            snapshot
                .blocks()
                .map(|(cid, b)| (cid, (b.as_ptr() as usize, b.len())))
                .collect()
        );
        drop(blocks);
        assert_eq!(snapshot.root(), root); // Owned original buffers survive the input store.
    }
}

#[test]
fn every_required_missing_or_corrupt_block_fails_restoration_without_genesis_or_memory_fallback() {
    for (blocks, root) in valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        for (cid, _) in snapshot.blocks() {
            let mut missing = Store(blocks.0.clone());
            missing.0.remove(&cid);
            assert_eq!(
                RestoredAgent::restore(root, &missing, &Limits::default()),
                Err(GraphError::InputUnavailable(cid))
            );
            let mut corrupt = Store(blocks.0.clone());
            corrupt.0.get_mut(&cid).unwrap()[0] ^= 1;
            assert_eq!(
                RestoredAgent::restore(root, &corrupt, &Limits::default()),
                Err(GraphError::IntegrityFailure(cid))
            );
        }
    }
}

#[test]
fn partial_role_graphs_and_bad_roles_ids_ordinals_or_grants_cannot_promote() {
    for (index, fixture) in roles().as_array().unwrap().iter().enumerate().skip(1) {
        let blocks = store(fixture);
        let (boundary, _) = command_at(&blocks, root_of(fixture));
        let error = if index == 1 {
            ToolBindingError::UnknownTool(boundary)
        } else {
            ToolBindingError::AuthorityMismatch(boundary)
        };
        let inspected =
            InspectedClosure::inspect(root_of(fixture), &blocks, &Limits::default()).unwrap();
        assert_eq!(RestoredAgent::from_inspected(inspected), Err(error.into()));
    }
    let fixture = &identity_graphs()[0];
    let (blocks, root, cid) = episode_change(fixture, |r| {
        map_mut(r).insert("closed".into(), Cbor::Bool(true));
    });
    assert_eq!(
        RestoredAgent::restore(root, &blocks, &Limits::default()),
        Err(EpisodeRoleError::CurrentEpisodeClosed(cid).into())
    );
    let (blocks, root, cid) = episode_change(fixture, |r| {
        map_mut(r).insert("id".into(), Cbor::Bytes(vec![99; 32]));
    });
    assert_eq!(
        RestoredAgent::restore(root, &blocks, &Limits::default()),
        Err(IdentityError::EpisodeIdMismatch(cid).into())
    );
    let (blocks, root, cid) = boundary_change(fixture, 1, |r| {
        map_mut(r).insert("ordinal".into(), Cbor::Integer(9.into()));
    });
    assert_eq!(
        RestoredAgent::restore(root, &blocks, &Limits::default()),
        Err(TransitionError::OrdinalMismatch(cid).into())
    );
    let (blocks, root, cid) = super::super::transitions::input_change(fixture, 1, |r| {
        map_mut(map_mut(r).get_mut("budget_admission").unwrap())
            .insert("remaining_actions".into(), Cbor::Integer(0.into()));
    });
    assert_eq!(
        RestoredAgent::restore(root, &blocks, &Limits::default()),
        Err(AdmissionGrantError::Action(cid).into())
    );
}

#[test]
fn caller_wire_nested_and_aggregate_caps_reject_without_changing_definition_or_history() {
    struct NoRead(Cell<u64>);
    impl MaterializedBlocks for NoRead {
        fn get(&self, _: DaslCid) -> Option<&[u8]> {
            self.0.set(self.0.get() + 1);
            None
        }
    }
    let (_, root) = memory_profile_snapshot();
    let no_read = NoRead(Cell::new(0));
    assert_eq!(
        RestoredAgent::restore(
            root,
            &no_read,
            &Limits {
                max_blocks: 0,
                ..Limits::default()
            }
        ),
        Err(GraphError::BlockCountLimit)
    );
    assert_eq!(no_read.0.get(), 0);
    for (blocks, root) in valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        assert_eq!(
            RestoredAgent::restore(
                root,
                &blocks,
                &Limits {
                    max_materialized_bytes: snapshot.encoded_bytes() - 1,
                    ..Limits::default()
                }
            ),
            Err(GraphError::MaterializedByteLimit)
        );
        assert_eq!(
            RestoredAgent::restore(
                root,
                &blocks,
                &Limits {
                    max_blocks: snapshot.block_count() - 1,
                    ..Limits::default()
                }
            ),
            Err(GraphError::BlockCountLimit)
        );
        assert!(RestoredAgent::restore(
            root,
            &blocks,
            &Limits {
                max_depth: 1,
                ..Limits::default()
            }
        )
        .is_err());
        if snapshot.state().history_count() > 0 {
            assert_eq!(
                RestoredAgent::restore(
                    root,
                    &blocks,
                    &Limits {
                        history_episodes: 0,
                        ..Limits::default()
                    }
                ),
                Err(HistoryConsistencyError::CallerWindowLimit(root).into())
            );
        }
        if snapshot.current_episode().is_some() {
            assert!(RestoredAgent::restore(
                root,
                &blocks,
                &Limits {
                    max_command_bytes: 1,
                    ..Limits::default()
                }
            )
            .is_err());
        }
    }
}

#[test]
fn typed_queries_do_not_hydrate_unrelated_or_wrong_kind_cids_and_restoration_does_not_authorize_head(
) {
    let (mut blocks, root) = memory_profile_snapshot();
    let foreign = memories::new_memory(&mut blocks, "unrelated");
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    assert!(snapshot.memory_block(foreign).is_none());
    assert!(snapshot.boundary(root).is_none());
    assert!(snapshot.input(root).is_none());
    assert!(snapshot.memory_block(root).is_none());
    assert!(!snapshot.blocks().any(|(cid, _)| cid == foreign));
    let raw = DaslCid::for_block(DaslCodec::Raw, b"not state");
    assert_eq!(
        RestoredAgent::restore(raw, &blocks, &Limits::default()),
        Err(GraphError::Codec(crate::format::CodecError::InvalidLink))
    );
    // Old and new valid roots may both restore as data; this API does not choose a canonical HEAD.
    let roots = identity_graphs();
    for fixture in roots.as_array().unwrap() {
        assert!(
            RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).is_ok()
        );
    }
}
