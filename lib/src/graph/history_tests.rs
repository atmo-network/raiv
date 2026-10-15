use super::*;

fn history(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_history_consistency()
}

fn revision(blocks: &mut Store, root: DaslCid, revision: u64) -> DaslCid {
    patch(blocks, root, |r| {
        map_mut(r).insert("revision".into(), Cbor::Integer(revision.into()));
    })
}

fn window(blocks: &mut Store, root: DaslCid, window: u64) -> DaslCid {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let definition = patch(blocks, state.definition_link(), |r| {
        map_mut(map_mut(r).get_mut("limits").unwrap())
            .insert("history_episodes".into(), Cbor::Integer(window.into()));
    });
    patch(blocks, root, |r| {
        map_mut(r).insert("definition".into(), link(definition));
    })
}

pub(super) fn completed_pair() -> (Store, DaslCid) {
    let fixture = &roles()[0];
    let (mut blocks, episode_cid, episode) = episode_for(fixture);
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
    let boundary = Boundary::decode(
        blocks.get(episode.boundaries().next().unwrap()).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    let event = [91u8; 32];
    let id: crate::format::Id32 = Sha256::digest(encode(&Cbor::Array(vec![
        Cbor::Text("raiv/episode/1".into()),
        Cbor::Bytes(definition.execution_domain().to_vec()),
        Cbor::Bytes(state.agent_id().to_vec()),
        Cbor::Bytes(event.to_vec()),
    ])))
    .into();
    let input = patch(&mut blocks, boundary.input_link(), |r| {
        map_mut(r).insert("event_id".into(), Cbor::Bytes(event.to_vec()));
    });
    let next_boundary = patch(&mut blocks, episode.boundaries().next().unwrap(), |r| {
        map_mut(r).insert("input".into(), link(input));
    });
    let next = patch(&mut blocks, episode_cid, |r| {
        map_mut(r).insert("id".into(), Cbor::Bytes(id.to_vec()));
        map_mut(r).insert("trigger_event_id".into(), Cbor::Bytes(event.to_vec()));
        map_mut(r).insert("boundaries".into(), Cbor::Array(vec![link(next_boundary)]));
    });
    let root = patch(&mut blocks, root_of(fixture), |r| {
        map_mut(r).insert(
            "history".into(),
            Cbor::Array(vec![link(episode_cid), link(next)]),
        );
        map_mut(r).insert("revision".into(), Cbor::Integer(2.into()));
    });
    (blocks, root)
}

#[test]
fn all_existing_role_identity_and_genesis_graphs_have_consistent_totals_without_io_or_rewrites() {
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
        let before = inspected
            .blocks()
            .map(|(cid, b)| (cid, b.to_vec()))
            .collect::<BTreeMap<_, _>>();
        let reads = blocks.1.get();
        assert_eq!(
            inspected.check_history_consistency(),
            Ok(()),
            "{}",
            fixture["name"]
        );
        assert_eq!(blocks.1.get(), reads);
        assert_eq!(
            before,
            inspected
                .blocks()
                .map(|(cid, b)| (cid, b.to_vec()))
                .collect()
        );
    }
}

#[test]
fn revision_cannot_undercount_retained_boundaries_across_current_and_completed_episodes() {
    for (fixture, low) in [
        (identity_graphs()[0].clone(), 1),
        (identity_graphs()[1].clone(), 2),
    ] {
        let mut blocks = store(&fixture);
        let root = revision(&mut blocks, root_of(&fixture), low);
        assert_eq!(
            InspectedClosure::inspect(root, &blocks, &Limits::default())
                .unwrap()
                .check_memory_consistency(),
            Ok(())
        );
        assert_eq!(
            history(&blocks, root),
            Err(HistoryConsistencyError::RevisionTooSmall(root).into())
        );
    }
    let (mut blocks, root) = completed_pair();
    let root = revision(&mut blocks, root, 1);
    assert_eq!(
        history(&blocks, root),
        Err(HistoryConsistencyError::RevisionTooSmall(root).into())
    );
}

#[test]
fn an_evicted_prefix_requires_the_full_definition_window_but_initial_partial_history_is_valid() {
    for fixture in [
        roles()[0].clone(),
        roles()[2].clone(),
        identity_graphs()[0].clone(),
    ] {
        let mut blocks = store(&fixture);
        assert_eq!(history(&blocks, root_of(&fixture)), Ok(()));
        let root = revision(&mut blocks, root_of(&fixture), 100);
        assert_eq!(
            history(&blocks, root),
            Err(HistoryConsistencyError::IncompleteWindow(root).into())
        );
    }
    let (mut blocks, root) = completed_pair();
    let root = revision(&mut blocks, root, u64::MAX);
    assert_eq!(history(&blocks, root), Ok(())); // Full retained window, unknown evicted prefix, not authorized revision evidence.
}

#[test]
fn definition_window_is_not_widened_by_caller_capacity_and_zero_has_a_retention_floor() {
    for value in [0, 1] {
        let (mut blocks, root) = completed_pair();
        let root = window(&mut blocks, root, value);
        assert_eq!(
            InspectedClosure::inspect(
                root,
                &blocks,
                &Limits {
                    history_episodes: u64::MAX,
                    ..Limits::default()
                }
            )
            .unwrap()
            .check_history_consistency(),
            Err(HistoryConsistencyError::DefinitionWindowLimit(root).into())
        );
    }
    for value in [0, 1] {
        let fixture = &roles()[0];
        let mut blocks = store(fixture);
        let root = window(&mut blocks, root_of(fixture), value);
        let root = revision(&mut blocks, root, u64::MAX);
        assert_eq!(history(&blocks, root), Ok(()));
    }
}

#[test]
fn narrower_caller_rejects_before_episode_reads_without_truncating_definition_history() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    for cap in [0, 1] {
        let (blocks, root) = completed_pair();
        let blocks = Counting(blocks, Cell::new(0));
        let limits = Limits {
            history_episodes: cap,
            ..Limits::default()
        };
        assert_eq!(
            InspectedClosure::inspect(root, &blocks, &limits),
            Err(HistoryConsistencyError::CallerWindowLimit(root).into())
        );
        assert_eq!(blocks.1.get(), 2); // Root/Definition only; never fetch/truncate Episodes.
        let fixture = &roles()[0];
        let blocks = store(fixture);
        if cap == 0 {
            assert_eq!(
                InspectedClosure::inspect(root_of(fixture), &blocks, &limits),
                Err(HistoryConsistencyError::CallerWindowLimit(root_of(fixture)).into())
            ); // Caller zero is a true capacity, NOT the Definition's retention floor.
            let fixture = &roles()[1];
            assert_eq!(
                InspectedClosure::inspect(root_of(fixture), &store(fixture), &limits)
                    .unwrap()
                    .check_history_consistency(),
                Ok(())
            ); // First unfinished Episode has no completed history.
        } else {
            assert_eq!(
                InspectedClosure::inspect(root_of(fixture), &blocks, &limits)
                    .unwrap()
                    .check_history_consistency(),
                Ok(())
            ); // Only one completion yet, no policy rewrite.
        }
    }
}

#[test]
fn full_u64_windows_and_genesis_counts_remain_data_valid_without_casting_or_inventing_history() {
    let (mut blocks, root) = completed_pair();
    let root = window(&mut blocks, root, u64::MAX);
    assert_eq!(history(&blocks, root), Ok(())); // No eviction yet; available history need not fill capacity.
    let root = revision(&mut blocks, root, u64::MAX);
    assert_eq!(
        history(&blocks, root),
        Err(HistoryConsistencyError::IncompleteWindow(root).into())
    );
    for fixture in fixtures().as_array().unwrap() {
        let mut blocks = store(fixture);
        for value in [0, 1, u64::MAX] {
            let root = window(&mut blocks, root_of(fixture), value);
            assert_eq!(history(&blocks, root), Ok(()));
        }
    }
}
