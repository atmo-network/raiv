use super::*;

#[path = "identity_tests.rs"]
mod identities;
use crate::episode::Episode;
use crate::format::oracle::{encode, link, map_mut};
use ciborium::value::Value as Cbor;
use core::cell::Cell;

fn roles() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/roles.json")).unwrap()
}
fn patch(blocks: &mut Store, cid: DaslCid, edit: impl FnOnce(&mut Cbor)) -> DaslCid {
    let mut record: Cbor = ciborium::de::from_reader(blocks.get(cid).unwrap()).unwrap();
    edit(&mut record);
    let bytes = encode(&record);
    let next = DaslCid::for_block(DaslCodec::Drisl, &bytes);
    blocks.0.insert(next, bytes);
    next
}
fn episode_change(fixture: &Value, edit: impl FnOnce(&mut Cbor)) -> (Store, DaslCid, DaslCid) {
    let mut blocks = store(fixture);
    let state = AgentState::decode(
        blocks.get(super::root(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let current = state.current_episode_link().is_some();
    let cid = state
        .current_episode_link()
        .unwrap_or_else(|| state.history().next().unwrap());
    let next = patch(&mut blocks, cid, edit);
    let next_root = patch(&mut blocks, super::root(fixture), |record| {
        if current {
            map_mut(record).insert("current_episode".into(), link(next));
        } else {
            map_mut(record).insert("history".into(), Cbor::Array(vec![link(next)]));
        }
    });
    (blocks, next_root, next)
}
fn check(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_episode_roles()
}
fn episode_for(fixture: &Value) -> (Store, DaslCid, Episode) {
    let blocks = store(fixture);
    let state = AgentState::decode(
        blocks.get(super::root(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let cid = state
        .current_episode_link()
        .unwrap_or_else(|| state.history().next().unwrap());
    let episode = Episode::decode(blocks.get(cid).unwrap(), &DecodeLimits::default()).unwrap();
    (blocks, cid, episode)
}

#[test]
fn independent_role_graphs_and_both_genesis_profiles_pass_without_more_store_reads() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    let role_fixtures = roles();
    let genesis = fixtures();
    for fixture in role_fixtures
        .as_array()
        .unwrap()
        .iter()
        .chain(genesis.as_array().unwrap())
    {
        let blocks = Counting(store(fixture), Cell::new(0));
        for bytes in blocks.0 .0.values() {
            let independent: Cbor = ciborium::de::from_reader(bytes.as_slice()).unwrap();
            assert_eq!(encode(&independent), *bytes);
        }
        let inspected =
            InspectedClosure::inspect(super::root(fixture), &blocks, &Limits::default()).unwrap();
        let before = inspected
            .blocks()
            .map(|(cid, bytes)| (cid, bytes.to_vec()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            inspected.encoded_bytes(),
            fixture["closure_bytes"].as_u64().unwrap()
        );
        assert_eq!(
            inspected.block_count(),
            fixture["blocks"].as_array().unwrap().len() as u64
        );
        let calls = blocks.1.get();
        assert_eq!(inspected.check_episode_roles(), Ok(()));
        assert_eq!(blocks.1.get(), calls);
        assert_eq!(
            before,
            inspected
                .blocks()
                .map(|(cid, bytes)| (cid, bytes.to_vec()))
                .collect()
        );
    }
}

#[test]
fn current_history_flags_and_empty_episodes_are_rejected_in_their_own_roles() {
    let data = roles();
    for (index, closed, expected) in [(0, false, false), (1, true, true)] {
        let (blocks, root, cid) = episode_change(&data[index], |record| {
            map_mut(record).insert("closed".into(), Cbor::Bool(closed));
        });
        let error = if expected {
            EpisodeRoleError::CurrentEpisodeClosed(cid)
        } else {
            EpisodeRoleError::HistoryEpisodeOpen(cid)
        };
        assert_eq!(check(&blocks, root), Err(error.into()));
    }
    for index in [0, 1] {
        let (blocks, root, cid) = episode_change(&data[index], |record| {
            map_mut(record).insert("boundaries".into(), Cbor::Array(vec![]));
        });
        assert_eq!(
            check(&blocks, root),
            Err(EpisodeRoleError::EmptyEpisode(cid).into())
        );
    }
}

#[test]
fn duplicate_episode_cids_and_different_manifest_versions_of_one_id_are_rejected() {
    let fixture = &roles()[0];
    let (mut blocks, cid, _) = episode_for(fixture);
    let duplicate_root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("history".into(), Cbor::Array(vec![link(cid), link(cid)]));
    });
    let inspected = InspectedClosure::inspect(duplicate_root, &blocks, &Limits::default()).unwrap();
    assert_eq!(inspected.block_count(), 5); // Physical reuse is allowed; semantic repetition is not.
    assert_eq!(
        inspected.check_episode_roles(),
        Err(EpisodeRoleError::RepeatedEpisode(cid).into())
    );
    let next = patch(&mut blocks, cid, |record| {
        map_mut(record).insert("trigger_event_id".into(), Cbor::Bytes(vec![42; 32]));
    });
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("history".into(), Cbor::Array(vec![link(cid), link(next)]));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::RepeatedEpisode(next).into())
    );
    let fixture = &roles()[2];
    let mut blocks = store(fixture);
    let state = AgentState::decode(
        blocks.get(super::root(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let history = state.history().next().unwrap();
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("current_episode".into(), link(history));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::RepeatedEpisode(history).into())
    );
}

#[test]
fn boundary_reuse_and_distinct_cids_at_one_episode_ordinal_are_rejected() {
    let fixture = &roles()[1];
    let (_, _, episode) = episode_for(fixture);
    let boundary = episode.boundaries().next().unwrap();
    let (blocks, root, _) = episode_change(fixture, |record| {
        map_mut(record).insert(
            "boundaries".into(),
            Cbor::Array(vec![link(boundary), link(boundary)]),
        );
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::RepeatedBoundary(boundary).into())
    );
    let (mut blocks, cid, _) = episode_for(fixture);
    let next = patch(&mut blocks, boundary, |record| {
        map_mut(map_mut(record).get_mut("usage").unwrap())
            .insert("model_calls".into(), Cbor::Integer(2.into()));
    });
    let next_episode = patch(&mut blocks, cid, |record| {
        map_mut(record).insert(
            "boundaries".into(),
            Cbor::Array(vec![link(boundary), link(next)]),
        );
    });
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("current_episode".into(), link(next_episode));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::RepeatedBoundary(next).into())
    );
    let fixture = &roles()[0];
    let (mut blocks, cid, episode) = episode_for(fixture);
    let next = patch(&mut blocks, cid, |record| {
        map_mut(record).insert("id".into(), Cbor::Bytes(vec![43; 32]));
    });
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("history".into(), Cbor::Array(vec![link(cid), link(next)]));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::RepeatedBoundary(episode.boundaries().next().unwrap()).into())
    );
}

#[test]
fn current_actor_exits_and_completed_action_tails_and_nonfinal_actor_exits_reject() {
    let fixture = &roles()[1];
    let (mut blocks, cid, episode) = episode_for(fixture);
    let boundary = episode.boundaries().next().unwrap();
    let actor = patch(&mut blocks, boundary, |record| {
        map_mut(record).insert(
            "exit".into(),
            Cbor::Map(vec![(
                Cbor::Text("tag".into()),
                Cbor::Text("await_actor".into()),
            )]),
        );
    });
    let next = patch(&mut blocks, cid, |record| {
        map_mut(record).insert("boundaries".into(), Cbor::Array(vec![link(actor)]));
    });
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("current_episode".into(), link(next));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::UnexpectedExit(actor).into())
    );
    let (mut blocks, _, episode) = episode_for(fixture);
    let next = patch(&mut blocks, cid, |record| {
        map_mut(record).insert("closed".into(), Cbor::Bool(true));
    });
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("current_episode".into(), Cbor::Null);
        map_mut(record).insert(
            "continuation".into(),
            Cbor::Map(vec![(
                Cbor::Text("tag".into()),
                Cbor::Text("await_actor".into()),
            )]),
        );
        map_mut(record).insert("history".into(), Cbor::Array(vec![link(next)]));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::UnexpectedExit(episode.boundaries().next().unwrap()).into())
    );
    let fixture = &roles()[0];
    let (mut blocks, cid, episode) = episode_for(fixture);
    let actor = episode.boundaries().next().unwrap();
    let next = patch(&mut blocks, actor, |record| {
        map_mut(record).insert("ordinal".into(), Cbor::Integer(1.into()));
    });
    let ep = patch(&mut blocks, cid, |record| {
        map_mut(record).insert(
            "boundaries".into(),
            Cbor::Array(vec![link(actor), link(next)]),
        );
    });
    let root = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("history".into(), Cbor::Array(vec![link(ep)]));
    });
    assert_eq!(
        check(&blocks, root),
        Err(EpisodeRoleError::UnexpectedExit(actor).into())
    );
}

#[test]
fn revision_zero_cannot_have_episodes_and_nonzero_actor_requires_history() {
    let fixture = &roles()[0];
    let mut blocks = store(fixture);
    let zero = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("revision".into(), Cbor::Integer(0.into()));
    });
    assert_eq!(
        check(&blocks, zero),
        Err(EpisodeRoleError::RevisionZeroWithEpisodes.into())
    );
    let empty = patch(&mut blocks, super::root(fixture), |record| {
        map_mut(record).insert("history".into(), Cbor::Array(vec![]));
    });
    assert_eq!(
        check(&blocks, empty),
        Err(EpisodeRoleError::MissingCompletedHistory.into())
    );
}
