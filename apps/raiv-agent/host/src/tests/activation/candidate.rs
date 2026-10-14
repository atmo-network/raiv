//! Full closure plus source joins: valid standalone roots do not establish next transitions.
use super::*;
use crate::{
    candidate::{restore, VerificationError},
    output::Output,
};
use raiv::{
    format::{DaslCid, DaslCodec},
    graph::{MaterializedBlocks, RestoredAgent},
    input::ActivationInput,
    limits::Limits,
    preparation::{InputBinding, PreparedInput},
};
fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
pub(super) fn source() -> RestoredAgent {
    let bytes = encode(&genesis(1));
    let Output::Initialized { root, blocks } = Output::decode(&bytes, &Limits::default()).unwrap()
    else {
        panic!()
    };
    RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap()
}
pub(super) fn prepared(source: &RestoredAgent, input: &Value) -> PreparedInput {
    let input = ActivationInput::decode(&encode(input), &Limits::default()).unwrap();
    PreparedInput::new(
        InputBinding {
            agent_id: source.state().agent_id(),
            execution_domain: source.definition().execution_domain(),
            expected_root: source.root(),
            expected_revision: source.state().revision(),
            admitted_event_id: input.event_id(),
            admitted_input_digest: input.cid().digest(),
        },
        input,
    )
    .unwrap()
}
fn activation(source: &RestoredAgent, packet: &PreparedInput) -> Vec<u8> {
    envelope(
        "activate",
        map(vec![
            ("root", Value::Bytes(source.root().as_bytes().to_vec())),
            ("input", Value::Bytes(packet.input().as_bytes().to_vec())),
            (
                "blocks",
                Value::Array(
                    source
                        .blocks()
                        .map(|(cid, body)| {
                            map(vec![
                                ("cid", Value::Bytes(cid.as_bytes().to_vec())),
                                ("body", Value::Bytes(body.to_vec())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    )
}
pub(super) fn boundary(
    source: &RestoredAgent,
    packet: &PreparedInput,
    patch: Option<Value>,
    tool: &str,
    args: Value,
) -> Vec<u8> {
    let mut guest = guest::Guest::new();
    guest.step(&activation(source, packet)).unwrap();
    let mut id = 1;
    if let Some(patch) = patch {
        guest
            .step(&progression::reply(
                1,
                1,
                vec![progression::call("patch_memory", patch)],
            ))
            .unwrap();
        id = 2;
    }
    guest
        .step(&progression::reply(
            id,
            id,
            if tool.is_empty() {
                vec![]
            } else {
                vec![progression::call(tool, args)]
            },
        ))
        .unwrap()
}
pub(super) fn check(
    bytes: &[u8],
    source: &RestoredAgent,
    packet: &PreparedInput,
    caps: &Limits,
) -> Result<RestoredAgent, VerificationError> {
    let Output::Boundary(candidate) = Output::decode(bytes, &Limits::default()).unwrap() else {
        panic!()
    };
    restore(candidate, source, packet, caps)
}
fn change(frame: &[u8], key: &str, value: Value) -> Vec<u8> {
    let mut frame = decode(frame);
    let mut payload = field(&frame, "payload").clone();
    let mut candidate = field(&payload, "candidate").clone();
    set(&mut candidate, key, value);
    set(&mut payload, "candidate", candidate);
    set(&mut frame, "payload", payload);
    encode(&frame)
}
fn delta(frame: &[u8]) -> Vec<Value> {
    let frame = decode(frame);
    let Value::Array(blocks) = field(field(field(&frame, "payload"), "candidate"), "new_blocks")
    else {
        panic!()
    };
    blocks.clone()
}
fn sorted(mut blocks: Vec<Value>) -> Value {
    blocks.sort_by_key(|b| encode(field(b, "cid")));
    Value::Array(blocks)
}
struct Overlay<'a> {
    source: &'a RestoredAgent,
    blocks: crate::output::Blocks<'a>,
}
impl MaterializedBlocks for Overlay<'_> {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.blocks.get(cid).or_else(|| {
            self.source
                .blocks()
                .find(|(key, _)| *key == cid)
                .map(|(_, bytes)| bytes)
        })
    }
}
fn standalone(frame: &[u8], source: &RestoredAgent) -> RestoredAgent {
    let Output::Boundary(candidate) = Output::decode(frame, &Limits::default()).unwrap() else {
        panic!()
    };
    RestoredAgent::restore(
        candidate.new_root(),
        &Overlay {
            source,
            blocks: candidate.new_blocks(),
        },
        &Limits::default(),
    )
    .unwrap()
}
fn state_change(frame: &[u8], edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut blocks = delta(frame);
    let index = blocks
        .iter()
        .position(|b| {
            let Value::Bytes(body) = field(b, "body") else {
                panic!()
            };
            field(&decode(body), "kind") == &Value::Text("agent_state".into())
        })
        .unwrap();
    let Value::Bytes(body) = field(&blocks[index], "body") else {
        panic!()
    };
    let mut state = decode(body);
    edit(&mut state);
    let body = encode(&state);
    let cid = DaslCid::for_block(DaslCodec::Drisl, &body);
    blocks[index] = map(vec![
        ("cid", Value::Bytes(cid.as_bytes().to_vec())),
        ("body", Value::Bytes(body)),
    ]);
    let frame = change(frame, "new_root", Value::Bytes(cid.as_bytes().to_vec()));
    change(&frame, "new_blocks", sorted(blocks))
}
#[test]
fn six_independent_lifecycle_witnesses_restore_and_join_exact_source_episode_history_and_input() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../fixtures/format/lifecycle.json"
    ))
    .unwrap();
    let mut source = source();
    for row in fixture["steps"].as_array().unwrap() {
        let input = decode(&hex(row["input"]["hex"].as_str().unwrap()));
        let packet = prepared(&source, &input);
        let frame = boundary(
            &source,
            &packet,
            Some(super::fixture(&row["patch"])),
            row["tool_id"].as_str().unwrap_or(""),
            super::fixture(&row["arguments"]),
        );
        let next = check(&frame, &source, &packet, &Limits::default()).unwrap();
        assert_eq!(
            next.state().as_bytes(),
            hex(row["state"]["hex"].as_str().unwrap())
        );
        assert_eq!(
            next.root().to_string(),
            row["state"]["cid"].as_str().unwrap()
        );
        assert_eq!(next.state().revision(), source.state().revision() + 1);
        assert_eq!(
            check(&frame, &source, &packet, &Limits::default()).unwrap(),
            next
        );
        source = next;
    }
}
#[test]
fn binding_disposition_caps_and_every_missing_delta_reject_without_source_mutation() {
    let source = source();
    let packet = prepared(&source, &packet());
    let frame = boundary(&source, &packet, None, "trade", map(vec![]));
    let before: Vec<_> = source.blocks().map(|(c, b)| (c, b.to_vec())).collect();
    for (key, value) in [
        ("agent_id", Value::Bytes(vec![0; 32])),
        (
            "expected_root",
            Value::Bytes(source.state().definition_link().as_bytes().to_vec()),
        ),
        ("expected_revision", Value::Integer(u64::MAX.into())),
        ("admitted_event_id", Value::Bytes(vec![0; 32])),
        ("admitted_input_digest", Value::Bytes(vec![0; 32])),
        (
            "disposition",
            map(vec![("tag", Value::Text("await_actor".into()))]),
        ),
    ] {
        assert!(check(
            &change(&frame, key, value),
            &source,
            &packet,
            &Limits::default()
        )
        .is_err());
    }
    for caps in [
        Limits {
            max_blocks: 0,
            ..Limits::default()
        },
        Limits {
            max_materialized_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_block_bytes: 1,
            ..Limits::default()
        },
    ] {
        assert!(check(&frame, &source, &packet, &caps).is_err());
    }
    let blocks = delta(&frame);
    for index in 0..blocks.len() {
        let mut missing = blocks.clone();
        missing.remove(index);
        let bytes = change(&frame, "new_blocks", sorted(missing));
        match Output::decode(&bytes, &Limits::default()) {
            Ok(Output::Boundary(c)) => {
                assert!(restore(c, &source, &packet, &Limits::default()).is_err())
            }
            Err(_) => {}
            _ => panic!(),
        }
    }
    assert_eq!(
        before,
        source
            .blocks()
            .map(|(c, b)| (c, b.to_vec()))
            .collect::<Vec<_>>()
    );
    check(&frame, &source, &packet, &Limits::default()).unwrap();
}
#[test]
fn unrelated_hash_valid_staging_blocks_and_repeated_source_blocks_are_not_candidate_delta() {
    let source = source();
    let packet = prepared(&source, &packet());
    let frame = boundary(&source, &packet, None, "", map(vec![]));
    let mut extra = delta(&frame);
    let cid = DaslCid::for_block(DaslCodec::Drisl, &[0xa0]);
    extra.push(map(vec![
        ("cid", Value::Bytes(cid.as_bytes().to_vec())),
        ("body", Value::Bytes(vec![0xa0])),
    ]));
    let bytes = change(&frame, "new_blocks", sorted(extra));
    standalone(&bytes, &source);
    assert_eq!(
        check(&bytes, &source, &packet, &Limits::default()).unwrap_err(),
        VerificationError::Delta
    );
    let (cid, body) = source.blocks().next().unwrap();
    let mut repeated = delta(&frame);
    repeated.push(map(vec![
        ("cid", Value::Bytes(cid.as_bytes().to_vec())),
        ("body", Value::Bytes(body.to_vec())),
    ]));
    let bytes = change(&frame, "new_blocks", sorted(repeated));
    assert_eq!(
        check(&bytes, &source, &packet, &Limits::default()).unwrap_err(),
        VerificationError::Delta
    );
}
#[test]
fn hash_valid_alternate_input_cannot_be_relabelled_as_the_admitted_packet() {
    let source = source();
    let original = packet();
    let packet = prepared(&source, &original);
    let mut different = original;
    set(
        &mut different,
        "observation",
        map(vec![("forged", Value::Bool(true))]),
    );
    let other = prepared(&source, &different);
    let frame = boundary(&source, &other, None, "", map(vec![]));
    let frame = change(
        &frame,
        "admitted_input_digest",
        Value::Bytes(packet.binding().admitted_input_digest.to_vec()),
    );
    standalone(&frame, &source);
    assert_eq!(
        check(&frame, &source, &packet, &Limits::default()).unwrap_err(),
        VerificationError::Transition
    );
}
#[test]
fn a_valid_alternate_pending_episode_prefix_cannot_replace_the_source_lineage() {
    let rows: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../fixtures/format/lifecycle.json"
    ))
    .unwrap();
    let genesis = source();
    let input = decode(&hex(rows["steps"][0]["input"]["hex"].as_str().unwrap()));
    let first = prepared(&genesis, &input);
    let args = super::fixture(&rows["steps"][0]["arguments"]);
    let patch = super::fixture(&rows["steps"][0]["patch"]);
    let frame = boundary(&genesis, &first, Some(patch.clone()), "trade", args.clone());
    let a = check(&frame, &genesis, &first, &Limits::default()).unwrap();
    let mut other_patch = patch;
    set(
        &mut other_patch,
        "strategy",
        map(vec![("alternate", Value::Bool(true))]),
    );
    let frame = boundary(&genesis, &first, Some(other_patch), "trade", args);
    let b = check(&frame, &genesis, &first, &Limits::default()).unwrap();
    assert_ne!(a.root(), b.root());
    let input = decode(&hex(rows["steps"][1]["input"]["hex"].as_str().unwrap()));
    let admitted = prepared(&a, &input);
    let other = prepared(&b, &input);
    let frame = boundary(
        &b,
        &other,
        Some(super::fixture(&rows["steps"][1]["patch"])),
        "trade",
        super::fixture(&rows["steps"][1]["arguments"]),
    );
    let next_b = check(&frame, &b, &other, &Limits::default()).unwrap();
    let full_delta = next_b
        .blocks()
        .filter(|(cid, _)| !a.blocks().any(|(old, _)| old == *cid))
        .map(|(cid, body)| {
            map(vec![
                ("cid", Value::Bytes(cid.as_bytes().to_vec())),
                ("body", Value::Bytes(body.to_vec())),
            ])
        })
        .collect();
    let frame = change(
        &frame,
        "expected_root",
        Value::Bytes(a.root().as_bytes().to_vec()),
    );
    let frame = change(&frame, "new_blocks", sorted(full_delta));
    standalone(&frame, &a);
    assert_eq!(
        check(&frame, &a, &admitted, &Limits::default()).unwrap_err(),
        VerificationError::Transition
    );
}
#[test]
fn physical_history_zero_one_two_and_disabled_memory_remain_source_joined() {
    for (profile, window) in [(1, 0), (1, 1), (1, 2), (0, 0)] {
        let mut initialization = decode(&frame(profile));
        let mut payload = field(&initialization, "payload").clone();
        let Value::Bytes(bytes) = field(&payload, "definition") else {
            panic!()
        };
        let mut definition = decode(bytes);
        let mut limits = field(&definition, "limits").clone();
        set(
            &mut limits,
            "history_episodes",
            Value::Integer(window.into()),
        );
        set(&mut definition, "limits", limits);
        set(
            &mut payload,
            "definition",
            Value::Bytes(encode(&definition)),
        );
        set(&mut initialization, "payload", payload);
        let mut guest = guest::Guest::new();
        let data = guest.step(&encode(&initialization)).unwrap();
        let Output::Initialized { root, blocks } =
            Output::decode(&data, &Limits::default()).unwrap()
        else {
            panic!()
        };
        let mut source = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
        for event in [0x70, 0x71, 0x72] {
            let mut input = packet();
            set(&mut input, "event_id", Value::Bytes(vec![event; 32]));
            let packet = prepared(&source, &input);
            let frame = boundary(&source, &packet, None, "", map(vec![]));
            let next = check(&frame, &source, &packet, &Limits::default()).unwrap();
            assert_eq!(
                next.state().history_count(),
                next.state().revision().min(window.max(1))
            );
            assert_eq!(next.memory().is_some(), profile == 1);
            source = next;
        }
    }
}
#[test]
fn standalone_valid_history_rewrite_and_revision_skip_are_not_next_source_transition() {
    let mut source = source();
    for event in [0x60, 0x61] {
        let mut input = packet();
        set(&mut input, "event_id", Value::Bytes(vec![event; 32]));
        let packet = prepared(&source, &input);
        let frame = boundary(&source, &packet, None, "", map(vec![]));
        source = check(&frame, &source, &packet, &Limits::default()).unwrap();
    }
    let mut input = packet();
    set(&mut input, "event_id", Value::Bytes(vec![0x62; 32]));
    let packet = prepared(&source, &input);
    let frame = boundary(&source, &packet, None, "", map(vec![]));
    let old_episode = source.state().history().next().unwrap();
    let replacement = Value::Tag(
        42,
        Box::new(Value::Bytes([&[0u8][..], old_episode.as_bytes()].concat())),
    );
    let rewritten = state_change(&frame, |state| {
        let mut history = field(state, "history").clone();
        let Value::Array(ref mut entries) = history else {
            panic!()
        };
        entries[0] = replacement;
        set(state, "history", history);
    });
    standalone(&rewritten, &source);
    assert_eq!(
        check(&rewritten, &source, &packet, &Limits::default()).unwrap_err(),
        VerificationError::Transition
    );
    let skipped = state_change(&frame, |state| {
        set(state, "revision", Value::Integer(u64::MAX.into()))
    });
    standalone(&skipped, &source);
    assert_eq!(
        check(&skipped, &source, &packet, &Limits::default()).unwrap_err(),
        VerificationError::Transition
    );
}
