use super::*;

#[path = "activation_tests.rs"]
mod activation;
#[path = "call_budget_tests.rs"]
mod call_budget;
#[path = "candidate_tests.rs"]
mod candidate;
#[path = "history_encoding_tests.rs"]
mod encoding;
#[path = "context_projection_tests.rs"]
mod full_context;
#[path = "inference_tests.rs"]
mod inference;
#[path = "lifecycle_tests.rs"]
mod lifecycle;
#[path = "patch_argument_tests.rs"]
mod patch_arguments;
#[path = "preparation_tests.rs"]
mod preparation;
#[path = "ram_tests.rs"]
mod ram;
#[path = "activation_registry_tests.rs"]
mod registry;
#[path = "reply_tests.rs"]
mod reply;
#[path = "unit_budget_tests.rs"]
mod unit_budget;
use crate::{
    graph::RestoredAgent,
    projection::{EpisodeProjection, HistoryProjection},
};

fn cbor(bytes: &[u8]) -> Cbor {
    ciborium::de::from_reader(bytes).unwrap()
}
fn field<'a>(value: &'a Cbor, name: &str) -> &'a Cbor {
    let Cbor::Map(fields) = value else {
        panic!("map")
    };
    &fields
        .iter()
        .find(|(key, _)| matches!(key,Cbor::Text(key) if key==name))
        .unwrap()
        .1
}
fn cid(value: &Cbor) -> Option<DaslCid> {
    if matches!(value, Cbor::Null) {
        return None;
    }
    let Cbor::Tag(42, bytes) = value else {
        panic!("Link")
    };
    let Cbor::Bytes(bytes) = bytes.as_ref() else {
        panic!("CID payload")
    };
    assert_eq!(bytes[0], 0);
    Some(DaslCid::from_bytes(&bytes[1..]).unwrap())
}
fn verify(projected: EpisodeProjection<'_>, episode: DaslCid, snapshot: &RestoredAgent) {
    let blocks = snapshot.blocks().collect::<BTreeMap<_, _>>();
    let episode = cbor(blocks[&episode]);
    assert_eq!(field(&episode, "id"), &Cbor::Bytes(projected.id().to_vec()));
    assert_eq!(field(&episode, "closed"), &Cbor::Bool(projected.closed()));
    let entry = cid(field(&episode, "entry_memory"));
    assert_eq!(projected.entry_memory().map(Memory::cid), entry);
    if let Some(memory) = projected.entry_memory() {
        assert_eq!(memory.as_bytes().as_ptr(), blocks[&memory.cid()].as_ptr());
    }
    let Cbor::Array(boundaries) = field(&episode, "boundaries") else {
        panic!("boundaries")
    };
    let mut expected = Vec::new();
    for (index, boundary) in boundaries.iter().enumerate() {
        let boundary_cid = cid(boundary).unwrap();
        let boundary_bytes = blocks[&boundary_cid];
        let boundary = cbor(boundary_bytes);
        let exit = field(&boundary, "exit");
        if field(exit, "tag") != &Cbor::Text("await_action".into()) {
            continue;
        }
        let command = encode(field(exit, "command"));
        let command_ptr = boundary_bytes
            .windows(command.len())
            .find(|bytes| *bytes == command)
            .unwrap()
            .as_ptr();
        let receipt = boundaries.get(index + 1).map(|next| {
            let next = cbor(blocks[&cid(next).unwrap()]);
            let input_bytes = blocks[&cid(field(&next, "input")).unwrap()];
            let input = cbor(input_bytes);
            let receipt = encode(field(&input, "receipt"));
            let pointer = input_bytes
                .windows(receipt.len())
                .find(|bytes| *bytes == receipt)
                .unwrap()
                .as_ptr();
            (receipt, pointer)
        });
        expected.push((command, command_ptr, receipt));
    }
    let actual = projected.actions().collect::<Vec<_>>();
    assert_eq!(actual.len(), expected.len());
    for (actual, (command, pointer, receipt)) in actual.into_iter().zip(expected) {
        assert_eq!(actual.command_bytes(), command);
        assert_eq!(actual.command_bytes().as_ptr(), pointer);
        let typed = actual.command();
        assert_eq!(typed.as_bytes().as_ptr(), pointer);
        let expected = cbor(&command);
        assert_eq!(
            field(&expected, "tool_id"),
            &Cbor::Text(typed.tool_id().into())
        );
        assert_eq!(
            field(&expected, "tool_version"),
            &Cbor::Integer(typed.tool_version().into())
        );
        assert_eq!(
            typed.arguments_bytes(),
            encode(field(&expected, "arguments"))
        );
        assert_eq!(
            field(&expected, "action_id"),
            &Cbor::Bytes(typed.action_id().to_vec())
        );
        assert_eq!(
            field(&expected, "authority_binding"),
            &Cbor::Bytes(typed.authority_binding().to_vec())
        );
        assert_eq!(
            actual.receipt_bytes(),
            receipt.as_ref().map(|(bytes, _)| bytes.as_slice())
        );
        if let Some((bytes, pointer)) = receipt {
            assert_eq!(actual.receipt_bytes().unwrap().as_ptr(), pointer);
            let typed = actual.receipt().unwrap();
            assert_eq!(typed.as_bytes().as_ptr(), pointer);
            let expected = cbor(&bytes);
            assert_eq!(
                field(&expected, "outcome"),
                &Cbor::Text(typed.outcome().as_str().into())
            );
            assert_eq!(typed.result_bytes(), encode(field(&expected, "result")));
            assert_eq!(typed.costs_bytes(), encode(field(&expected, "costs")));
            assert_eq!(
                field(&expected, "action_id"),
                &Cbor::Bytes(typed.action_id().to_vec())
            );
            assert_eq!(
                field(&expected, "command_digest"),
                &Cbor::Bytes(typed.command_digest().to_vec())
            );
        } else {
            assert!(actual.receipt().is_none());
        }
    }
    let last = cbor(blocks[&cid(boundaries.last().unwrap()).unwrap()]);
    assert_eq!(
        projected.latest_memory().map(Memory::cid),
        cid(field(&last, "memory"))
    );
}

#[test]
fn completed_and_current_views_match_independent_ciborium_records_and_exact_borrowed_spans() {
    for (blocks, root) in restored::valid_snapshots() {
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let view = HistoryProjection::new(&snapshot);
        let history = view.completed().collect::<Vec<_>>();
        assert_eq!(history.len() as u64, snapshot.state().history_count());
        for (projected, cid) in history.into_iter().zip(snapshot.state().history()) {
            assert!(projected.closed());
            verify(projected, cid, &snapshot);
        }
        assert_eq!(
            view.current().is_some(),
            snapshot.state().current_episode_link().is_some()
        );
        if let Some(current) = view.current() {
            assert!(!current.closed());
            verify(
                current,
                snapshot.state().current_episode_link().unwrap(),
                &snapshot,
            );
        }
        if snapshot.state().revision() == 0 {
            assert_eq!(view.completed().count(), 0);
            assert!(view.current().is_none());
        }
    }
}

#[test]
fn final_noop_supplies_the_last_terminal_receipt_without_becoming_an_extra_action() {
    let fixtures = identity_graphs();
    for (index, fixture) in fixtures.as_array().unwrap().iter().enumerate() {
        let snapshot =
            RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
        let view = HistoryProjection::new(&snapshot);
        let episode = if index == 0 {
            view.current().unwrap()
        } else {
            view.completed().next().unwrap()
        };
        let actions = episode.actions().collect::<Vec<_>>();
        assert_eq!(actions.len(), 2);
        assert!(actions[0].receipt_bytes().is_some());
        assert_eq!(actions[1].receipt_bytes().is_some(), index == 1);
        assert_ne!(actions[0].command_bytes(), actions[1].command_bytes());
    }
    let fixture = &roles()[0];
    let snapshot =
        RestoredAgent::restore(root_of(fixture), &store(fixture), &Limits::default()).unwrap();
    assert_eq!(
        HistoryProjection::new(&snapshot)
            .completed()
            .next()
            .unwrap()
            .actions()
            .count(),
        0
    );
}

#[test]
fn zero_history_policy_hides_completed_projection_not_retention_current_chain_or_memory() {
    for window in [0, 1, 2, u64::MAX] {
        let (mut blocks, root) = restored::memory_profile_snapshot();
        let root = definition_change(&mut blocks, root, |r| {
            map_mut(map_mut(r).get_mut("limits").unwrap())
                .insert("history_episodes".into(), Cbor::Integer(window.into()));
        });
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let before = snapshot
            .blocks()
            .map(|(cid, b)| (cid, b.to_vec()))
            .collect::<BTreeMap<_, _>>();
        let view = HistoryProjection::new(&snapshot);
        assert_eq!(view.completed().count(), usize::from(window != 0));
        assert_eq!(snapshot.state().history_count(), 1);
        let current = view.current().unwrap();
        assert_eq!(current.actions().count(), 1);
        assert!(current.entry_memory().is_some());
        assert!(current.latest_memory().is_some());
        assert_eq!(
            snapshot
                .blocks()
                .map(|(cid, b)| (cid, b.to_vec()))
                .collect::<BTreeMap<_, _>>(),
            before
        );
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
}

#[test]
fn completed_episodes_remain_chronological_and_disabled_memory_never_creates_empty_planes() {
    let (blocks, root) = super::super::history::completed_pair();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let ids = snapshot.history().map(Episode::id).collect::<Vec<_>>();
    let projected = HistoryProjection::new(&snapshot)
        .completed()
        .collect::<Vec<_>>();
    assert_eq!(
        projected
            .iter()
            .map(|episode| episode.id())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(projected.len(), 2);
    assert_ne!(ids[0], ids[1]);
    for episode in projected {
        assert!(episode.entry_memory().is_none());
        assert!(episode.latest_memory().is_none());
        assert_eq!(episode.actions().count(), 0);
    }
}

#[test]
fn fresh_projection_requires_no_transcript_and_performs_no_store_reads_or_buffer_rewrites() {
    struct Counting(Store, Cell<u64>);
    impl MaterializedBlocks for Counting {
        fn get(&self, cid: DaslCid) -> Option<&[u8]> {
            self.1.set(self.1.get() + 1);
            self.0.get(cid)
        }
    }
    for (blocks, root) in restored::valid_snapshots() {
        let blocks = Counting(blocks, Cell::new(0));
        let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        let reads = blocks.1.get();
        let pointers = snapshot
            .blocks()
            .map(|(cid, b)| (cid, b.as_ptr()))
            .collect::<BTreeMap<_, _>>();
        for _ in 0..2 {
            let view = HistoryProjection::new(&snapshot);
            for episode in view.completed().chain(view.current()) {
                let _ = episode.entry_memory();
                let _ = episode.latest_memory();
                for action in episode.actions() {
                    assert!(!action.command_bytes().is_empty());
                }
            }
        }
        assert_eq!(blocks.1.get(), reads);
        assert_eq!(
            snapshot
                .blocks()
                .map(|(cid, b)| (cid, b.as_ptr()))
                .collect::<BTreeMap<_, _>>(),
            pointers
        );
    }
}

#[test]
fn working_memory_patches_cannot_rewrite_the_initial_checkpoint_projection() {
    let (blocks, root) = restored::memory_profile_snapshot();
    let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
    let view = HistoryProjection::new(&snapshot).current().unwrap();
    let initial = view.latest_memory().unwrap();
    let before = initial.as_bytes().to_vec();
    let cid = initial.cid();
    let mut working = Memory::decode(&before, &DecodeLimits::default()).unwrap();
    working
        .apply_patch(
            &crate::format::oracle::json_bytes(
                &serde_json::json!({"intents":{"new_note":"accepted"}}),
            ),
            snapshot.definition().limits(),
        )
        .unwrap();
    assert_ne!(working.cid(), cid);
    assert_eq!(view.latest_memory().unwrap().as_bytes(), before);
    assert_eq!(view.latest_memory().unwrap().cid(), cid);
}
