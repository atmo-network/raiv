use super::*;
use crate::input::ActivationInput;

fn check_grants(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_admission_grants()
}

fn grant_change(
    blocks: &mut Store,
    root: DaslCid,
    cid: DaslCid,
    index: usize,
    fields: &[(&str, u64)],
) -> (DaslCid, DaslCid) {
    let episode = Episode::decode(blocks.get(cid).unwrap(), &DecodeLimits::default()).unwrap();
    let mut boundaries = episode.boundaries().collect::<Vec<_>>();
    let boundary =
        Boundary::decode(blocks.get(boundaries[index]).unwrap(), &Limits::default()).unwrap();
    let input = patch(blocks, boundary.input_link(), |r| {
        let budget = map_mut(r).get_mut("budget_admission").unwrap();
        for (name, value) in fields {
            map_mut(budget).insert((*name).into(), Cbor::Integer((*value).into()));
        }
    });
    let boundary = patch(blocks, boundaries[index], |r| {
        map_mut(r).insert("input".into(), link(input));
    });
    boundaries[index] = boundary;
    let next = patch(blocks, cid, |r| {
        map_mut(r).insert(
            "boundaries".into(),
            Cbor::Array(boundaries.into_iter().map(link).collect()),
        );
    });
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let history = state
        .history()
        .map(|old| if old == cid { next } else { old })
        .collect::<Vec<_>>();
    let current = state
        .current_episode_link()
        .map(|old| if old == cid { next } else { old });
    let root = patch(blocks, root, |r| {
        map_mut(r).insert(
            "history".into(),
            Cbor::Array(history.into_iter().map(link).collect()),
        );
        map_mut(r).insert(
            "current_episode".into(),
            current.map(link).unwrap_or(Cbor::Null),
        );
    });
    (root, boundary)
}

#[test]
fn existing_closures_and_genesis_bind_grants_without_new_reads_or_byte_rewrites() {
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
        assert_eq!(inspected.check_admission_grants(), Ok(()));
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
fn each_input_call_grant_must_cover_exact_boundary_usage_not_an_earlier_or_later_grant() {
    for index in [0, 1] {
        let (mut blocks, root) = normalized(&identity_graphs()[0], 1, 0);
        let cid = selected_episode(&blocks, root);
        let (exact, _) = grant_change(
            &mut blocks,
            root,
            cid,
            index,
            &[("remaining_model_calls", 1)],
        );
        assert_eq!(check_grants(&blocks, exact), Ok(()));
        let cid = selected_episode(&blocks, exact);
        let (bad, boundary) = grant_change(
            &mut blocks,
            exact,
            cid,
            index,
            &[("remaining_model_calls", 0)],
        );
        assert_eq!(check_usage(&blocks, bad), Ok(()));
        assert_eq!(
            check_grants(&blocks, bad),
            Err(AdmissionGrantError::ModelCalls(boundary).into())
        );
    }
}

#[test]
fn both_yield_slots_need_an_action_grant_and_final_actor_does_not_spend_one() {
    for index in [0, 1] {
        let fixture = &identity_graphs()[0];
        let mut blocks = store(fixture);
        let cid = selected_episode(&blocks, root_of(fixture));
        let (exact, _) = grant_change(
            &mut blocks,
            root_of(fixture),
            cid,
            index,
            &[("remaining_actions", 1)],
        );
        assert_eq!(check_grants(&blocks, exact), Ok(()));
        let cid = selected_episode(&blocks, exact);
        let (bad, boundary) =
            grant_change(&mut blocks, exact, cid, index, &[("remaining_actions", 0)]);
        assert_eq!(
            check_grants(&blocks, bad),
            Err(AdmissionGrantError::Action(boundary).into())
        );
    }
    let (mut blocks, root) = normalized(&identity_graphs()[1], 1, 0);
    let cid = selected_episode(&blocks, root);
    let (root, _) = grant_change(
        &mut blocks,
        root,
        cid,
        2,
        &[("remaining_actions", 0), ("remaining_model_calls", 1)],
    );
    assert_eq!(check_grants(&blocks, root), Ok(()));
    let cid = selected_episode(&blocks, root);
    let (bad, boundary) = grant_change(&mut blocks, root, cid, 2, &[("remaining_model_calls", 0)]);
    assert_eq!(
        check_grants(&blocks, bad),
        Err(AdmissionGrantError::ModelCalls(boundary).into())
    );
}

#[test]
fn zero_call_grant_covers_only_zero_reported_calls_and_history_has_its_own_inputs() {
    let (mut blocks, root) = normalized(&roles()[0], 0, 0);
    let cid = selected_episode(&blocks, root);
    let (root, _) = grant_change(
        &mut blocks,
        root,
        cid,
        0,
        &[("remaining_model_calls", 0), ("remaining_actions", 0)],
    );
    assert_eq!(check_grants(&blocks, root), Ok(()));
    let fixture = &roles()[2];
    let mut blocks = store(fixture);
    let state = AgentState::decode(
        blocks.get(root_of(fixture)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let history = state.history().next().unwrap();
    let (root, _) = usage_change(&mut blocks, root_of(fixture), history, 0, |r| {
        map_mut(r).insert("model_calls".into(), Cbor::Integer(1.into()));
    });
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let history = state.history().next().unwrap();
    let (root, boundary) = grant_change(
        &mut blocks,
        root,
        history,
        0,
        &[("remaining_model_calls", 0)],
    );
    assert_eq!(
        check_grants(&blocks, root),
        Err(AdmissionGrantError::ModelCalls(boundary).into())
    );
}

#[test]
fn a_generous_host_grant_never_overrides_definition_limits() {
    let (mut blocks, root) = normalized(&identity_graphs()[0], 1, 0);
    let cid = selected_episode(&blocks, root);
    let (root, boundary) = grant_change(
        &mut blocks,
        root,
        cid,
        0,
        &[
            ("remaining_model_calls", u64::MAX),
            ("remaining_actions", u64::MAX),
        ],
    );
    assert_eq!(check_grants(&blocks, root), Ok(())); // A host grant may exceed hard policy; it does not replace it.
    let bad = limits_change(&mut blocks, root, &[("max_model_calls_per_activation", 0)]);
    assert_eq!(
        check_grants(&blocks, bad),
        Err(UsageLimitError::ActivationModelCalls(boundary).into())
    );
    let bad = limits_change(&mut blocks, root, &[("max_actions_per_episode", 0)]);
    assert_eq!(
        check_grants(&blocks, bad),
        Err(UsageLimitError::EpisodeActions(boundary).into())
    );
}

#[test]
fn full_u64_grants_are_not_narrowed_or_converted_to_costs_or_used_to_invent_topup_policy() {
    let (mut blocks, mut root) = normalized(&identity_graphs()[0], 0, 0);
    root = limits_change(
        &mut blocks,
        root,
        &[
            ("max_model_calls_per_activation", u64::MAX),
            ("max_model_calls_per_episode", u64::MAX),
        ],
    );
    let cid = selected_episode(&blocks, root);
    (root, _) = usage_change(&mut blocks, root, cid, 0, |r| {
        map_mut(r).insert("model_calls".into(), Cbor::Integer(u64::MAX.into()));
        map_mut(r).insert("input_tokens".into(), Cbor::Null);
        map_mut(r).insert("billed_units".into(), Cbor::Null);
    });
    let cid = selected_episode(&blocks, root);
    (root, _) = grant_change(
        &mut blocks,
        root,
        cid,
        0,
        &[
            ("remaining_model_calls", u64::MAX),
            ("remaining_actions", u64::MAX),
        ],
    );
    let cid = selected_episode(&blocks, root);
    (root, _) = grant_change(
        &mut blocks,
        root,
        cid,
        1,
        &[("remaining_model_calls", 0), ("remaining_actions", 1)],
    );
    assert_eq!(check_grants(&blocks, root), Ok(()));
    let cid = selected_episode(&blocks, root);
    let (bad, boundary) = grant_change(
        &mut blocks,
        root,
        cid,
        0,
        &[("remaining_model_calls", u64::MAX - 1)],
    );
    assert_eq!(
        check_grants(&blocks, bad),
        Err(AdmissionGrantError::ModelCalls(boundary).into())
    );
    // Grants may change across inputs; checking this subset does not authorize a top-up.
    let (mut blocks, root) = normalized(&identity_graphs()[0], 0, 0);
    let cid = selected_episode(&blocks, root);
    let (root, _) = grant_change(
        &mut blocks,
        root,
        cid,
        0,
        &[("remaining_model_calls", 0), ("remaining_actions", 1)],
    );
    let cid = selected_episode(&blocks, root);
    let (root, boundary) = grant_change(
        &mut blocks,
        root,
        cid,
        1,
        &[
            ("remaining_model_calls", u64::MAX),
            ("remaining_actions", u64::MAX),
        ],
    );
    assert_eq!(check_grants(&blocks, root), Ok(()));
    let boundary = Boundary::decode(blocks.get(boundary).unwrap(), &Limits::default()).unwrap();
    let input = ActivationInput::decode(
        blocks.get(boundary.input_link()).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(input.budget_admission().remaining_model_calls, u64::MAX);
}
