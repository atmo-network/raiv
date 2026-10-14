use super::*;

#[path = "admission_tests.rs"]
mod admissions;

fn check_usage(blocks: &Store, root: DaslCid) -> Result<(), GraphError> {
    InspectedClosure::inspect(root, blocks, &Limits::default())
        .unwrap()
        .check_usage_limits()
}
fn limits_change(blocks: &mut Store, root: DaslCid, values: &[(&str, u64)]) -> DaslCid {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let definition = patch(blocks, state.definition_link(), |r| {
        let limits = map_mut(r).get_mut("limits").unwrap();
        for (name, value) in values {
            map_mut(limits).insert((*name).into(), Cbor::Integer((*value).into()));
        }
    });
    patch(blocks, root, |r| {
        map_mut(r).insert("definition".into(), link(definition));
    })
}
fn selected_episode(blocks: &Store, root: DaslCid) -> DaslCid {
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    state
        .current_episode_link()
        .unwrap_or_else(|| state.history().last().unwrap())
}
fn usage_change(
    blocks: &mut Store,
    root: DaslCid,
    cid: DaslCid,
    index: usize,
    edit: impl FnOnce(&mut Cbor),
) -> (DaslCid, DaslCid) {
    let episode = Episode::decode(blocks.get(cid).unwrap(), &DecodeLimits::default()).unwrap();
    let mut boundaries = episode.boundaries().collect::<Vec<_>>();
    let boundary = patch(blocks, boundaries[index], |r| {
        edit(map_mut(r).get_mut("usage").unwrap());
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
fn normalized(fixture: &Value, model: u64, local: u64) -> (Store, DaslCid) {
    let mut blocks = store(fixture);
    let mut root = root_of(fixture);
    let count = Episode::decode(
        blocks.get(selected_episode(&blocks, root)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap()
    .boundary_count();
    for index in 0..count {
        let cid = selected_episode(&blocks, root);
        (root, _) = usage_change(&mut blocks, root, cid, index as usize, |r| {
            map_mut(r).insert("model_calls".into(), Cbor::Integer(model.into()));
            map_mut(r).insert("local_tool_calls".into(), Cbor::Integer(local.into()));
        });
    }
    (blocks, root)
}

#[test]
fn all_existing_graphs_and_genesis_profiles_keep_exact_bytes_and_need_no_more_reads() {
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
        assert_eq!(inspected.check_usage_limits(), Ok(()));
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
fn model_and_local_tool_activation_caps_accept_exact_limits_and_reject_the_next_counter() {
    for (field, limit_field, model_error) in [
        ("model_calls", "max_model_calls_per_activation", true),
        ("local_tool_calls", "max_local_tools_per_activation", false),
    ] {
        let (mut blocks, root) = normalized(&identity_graphs()[0], 1, 1);
        let root = limits_change(&mut blocks, root, &[(limit_field, 1)]);
        assert_eq!(check_usage(&blocks, root), Ok(()));
        let cid = selected_episode(&blocks, root);
        let (root, boundary) = usage_change(&mut blocks, root, cid, 0, |r| {
            map_mut(r).insert(field.into(), Cbor::Integer(2.into()));
        });
        assert_eq!(
            InspectedClosure::inspect(root, &blocks, &Limits::default())
                .unwrap()
                .check_history_consistency(),
            Ok(())
        );
        let error = if model_error {
            UsageLimitError::ActivationModelCalls(boundary)
        } else {
            UsageLimitError::ActivationLocalTools(boundary)
        };
        assert_eq!(check_usage(&blocks, root), Err(error.into()));
    }
}

#[test]
fn action_result_does_not_reset_episode_models_or_actions_and_finish_still_counts_models() {
    let (mut blocks, root) = normalized(&identity_graphs()[0], 1, 1);
    let episode = Episode::decode(
        blocks.get(selected_episode(&blocks, root)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let second = episode.boundaries().nth(1).unwrap();
    let exact = limits_change(
        &mut blocks,
        root,
        &[
            ("max_model_calls_per_episode", 2),
            ("max_actions_per_episode", 2),
        ],
    );
    assert_eq!(check_usage(&blocks, exact), Ok(()));
    let bad = limits_change(&mut blocks, root, &[("max_model_calls_per_episode", 1)]);
    assert_eq!(
        check_usage(&blocks, bad),
        Err(UsageLimitError::EpisodeModelCalls(second).into())
    );
    let bad = limits_change(&mut blocks, root, &[("max_actions_per_episode", 1)]);
    assert_eq!(
        check_usage(&blocks, bad),
        Err(UsageLimitError::EpisodeActions(second).into())
    );
    let (mut blocks, root) = normalized(&identity_graphs()[1], 1, 1);
    let episode = Episode::decode(
        blocks.get(selected_episode(&blocks, root)).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let final_actor = episode.boundaries().last().unwrap();
    let exact = limits_change(
        &mut blocks,
        root,
        &[
            ("max_model_calls_per_episode", 3),
            ("max_actions_per_episode", 2),
        ],
    );
    assert_eq!(check_usage(&blocks, exact), Ok(())); // Final AwaitActor is NOT a third external action.
    let bad = limits_change(&mut blocks, root, &[("max_model_calls_per_episode", 2)]);
    assert_eq!(
        check_usage(&blocks, bad),
        Err(UsageLimitError::EpisodeModelCalls(final_actor).into())
    );
}

#[test]
fn capacity_resets_at_episode_boundaries_not_at_each_activation_and_history_also_obeys_caps() {
    let fixture = &roles()[2];
    let (mut blocks, mut root) = normalized(fixture, 1, 1);
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let history = state.history().next().unwrap();
    (root, _) = usage_change(&mut blocks, root, history, 0, |r| {
        map_mut(r).insert("model_calls".into(), Cbor::Integer(1.into()));
        map_mut(r).insert("local_tool_calls".into(), Cbor::Integer(1.into()));
    });
    let root = limits_change(
        &mut blocks,
        root,
        &[
            ("max_model_calls_per_activation", 1),
            ("max_model_calls_per_episode", 1),
            ("max_local_tools_per_activation", 1),
            ("max_actions_per_episode", 1),
        ],
    );
    assert_eq!(check_usage(&blocks, root), Ok(())); // Two Episodes each get one model-call ceiling, NOT a lifetime pool.
    let state = AgentState::decode(blocks.get(root).unwrap(), &DecodeLimits::default()).unwrap();
    let history = state.history().next().unwrap();
    let (root, boundary) = usage_change(&mut blocks, root, history, 0, |r| {
        map_mut(r).insert("local_tool_calls".into(), Cbor::Integer(2.into()));
    });
    assert_eq!(
        check_usage(&blocks, root),
        Err(UsageLimitError::ActivationLocalTools(boundary).into())
    );
}

#[test]
fn zero_ceilings_allow_zero_usage_no_action_finish_but_never_a_yield_and_do_not_infer_costs() {
    let (mut blocks, root) = normalized(&roles()[0], 0, 0);
    let root = limits_change(
        &mut blocks,
        root,
        &[
            ("max_model_calls_per_activation", 0),
            ("max_model_calls_per_episode", 0),
            ("max_local_tools_per_activation", 0),
            ("max_actions_per_episode", 0),
        ],
    );
    let cid = selected_episode(&blocks, root);
    let (root, boundary) = usage_change(&mut blocks, root, cid, 0, |r| {
        map_mut(r).insert("input_tokens".into(), Cbor::Null);
        map_mut(r).insert("output_tokens".into(), Cbor::Integer(0.into()));
        map_mut(r).insert("billed_units".into(), Cbor::Integer(u64::MAX.into()));
    });
    assert_eq!(check_usage(&blocks, root), Ok(()));
    let usage = Boundary::decode(blocks.get(boundary).unwrap(), &Limits::default())
        .unwrap()
        .usage();
    assert_eq!(usage.input_tokens, None);
    assert_eq!(usage.output_tokens, Some(0));
    assert_eq!(usage.billed_units, Some(u64::MAX));
    let (mut blocks, root) = normalized(&roles()[1], 0, 0);
    let cid = selected_episode(&blocks, root);
    let first = Episode::decode(blocks.get(cid).unwrap(), &DecodeLimits::default())
        .unwrap()
        .boundaries()
        .next()
        .unwrap();
    let root = limits_change(&mut blocks, root, &[("max_actions_per_episode", 0)]);
    assert_eq!(
        check_usage(&blocks, root),
        Err(UsageLimitError::EpisodeActions(first).into())
    );
}

#[test]
fn full_u64_capacity_rejects_max_plus_one_without_wrapping_or_narrowing_counters() {
    let (mut blocks, root) = normalized(&identity_graphs()[0], 0, 0);
    let root = limits_change(
        &mut blocks,
        root,
        &[
            ("max_model_calls_per_activation", u64::MAX),
            ("max_model_calls_per_episode", u64::MAX),
            ("max_local_tools_per_activation", u64::MAX),
        ],
    );
    let cid = selected_episode(&blocks, root);
    let (root, _) = usage_change(&mut blocks, root, cid, 0, |r| {
        map_mut(r).insert("model_calls".into(), Cbor::Integer(u64::MAX.into()));
        map_mut(r).insert("local_tool_calls".into(), Cbor::Integer(u64::MAX.into()));
    });
    assert_eq!(check_usage(&blocks, root), Ok(()));
    let cid = selected_episode(&blocks, root);
    let (root, boundary) = usage_change(&mut blocks, root, cid, 1, |r| {
        map_mut(r).insert("model_calls".into(), Cbor::Integer(1.into()));
    });
    assert_eq!(
        check_usage(&blocks, root),
        Err(UsageLimitError::EpisodeModelCalls(boundary).into())
    );
}
