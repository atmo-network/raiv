use super::*;
use crate::command::CommandSpec;
use crate::format::oracle::{encode as bytes, map_bytes};
use crate::limits::Limits;
use alloc::{collections::BTreeMap, vec, vec::Vec};
use ciborium::value::Value as Cbor;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn id(value: &Value) -> Id32 {
    hex(value.as_str().unwrap()).try_into().unwrap()
}

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/identity.json")).unwrap()
}

fn command_fixture(index: usize) -> Command {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../fixtures/format/command.json")).unwrap();
    Command::decode(
        &hex(fixtures[index]["hex"].as_str().unwrap()),
        &Limits::default(),
    )
    .unwrap()
}

fn oracle(fixture: &Value) -> Vec<u8> {
    let domain = Cbor::Bytes(id(&fixture["execution_domain"]).to_vec());
    let tree = match fixture["kind"].as_str().unwrap() {
        "episode" => Cbor::Array(vec![
            Cbor::Text("raiv/episode/1".into()),
            domain,
            Cbor::Bytes(id(&fixture["agent_id"]).to_vec()),
            Cbor::Bytes(id(&fixture["trigger_event_id"]).to_vec()),
        ]),
        "action" => Cbor::Array(vec![
            Cbor::Text("raiv/action/1".into()),
            domain,
            Cbor::Bytes(id(&fixture["agent_id"]).to_vec()),
            Cbor::Bytes(id(&fixture["episode_id"]).to_vec()),
            Cbor::Integer(fixture["action_index"].as_u64().unwrap().into()),
        ]),
        "command" => {
            let command = command_fixture(fixture["command_fixture"].as_u64().unwrap() as usize);
            let tree: Cbor = ciborium::de::from_reader(command.as_bytes()).unwrap();
            Cbor::Array(vec![Cbor::Text("raiv/command/1".into()), domain, tree])
        }
        _ => panic!("unknown identity fixture"),
    };
    bytes(&tree)
}

#[test]
fn all_identity_domains_match_pinned_independent_preimages_and_hashes() {
    let fixtures = fixtures();
    for fixture in fixtures.as_array().unwrap() {
        let preimage = hex(fixture["preimage"].as_str().unwrap());
        let expected = id(&fixture["digest"]);
        assert_eq!(oracle(fixture), preimage);
        assert_eq!(&Sha256::digest(&preimage)[..], expected);
        let domain = id(&fixture["execution_domain"]);
        let actual = match fixture["kind"].as_str().unwrap() {
            "episode" => episode_id(
                domain,
                id(&fixture["agent_id"]),
                id(&fixture["trigger_event_id"]),
            ),
            "action" => action_id(
                domain,
                id(&fixture["agent_id"]),
                id(&fixture["episode_id"]),
                fixture["action_index"].as_u64().unwrap(),
            ),
            "command" => command_digest(
                domain,
                &command_fixture(fixture["command_fixture"].as_u64().unwrap() as usize),
            ),
            _ => unreachable!(),
        };
        assert_eq!(actual, expected);
        if fixture["kind"] == "command" {
            let command = command_fixture(fixture["command_fixture"].as_u64().unwrap() as usize);
            let view = crate::command::CommandView::decode(command.as_bytes(), &Limits::default())
                .unwrap();
            assert_eq!(command_digest_view(domain, view), expected);
        }
    }
}

#[test]
fn episode_and_action_ids_bind_every_scalar_and_full_width_slot_index() {
    let base = episode_id([1; 32], [2; 32], [3; 32]);
    for changed in [
        episode_id([4; 32], [2; 32], [3; 32]),
        episode_id([1; 32], [4; 32], [3; 32]),
        episode_id([1; 32], [2; 32], [4; 32]),
    ] {
        assert_ne!(base, changed);
    }
    let base = action_id([1; 32], [2; 32], [3; 32], 0);
    for changed in [
        action_id([4; 32], [2; 32], [3; 32], 0),
        action_id([1; 32], [4; 32], [3; 32], 0),
        action_id([1; 32], [2; 32], [4; 32], 0),
        action_id([1; 32], [2; 32], [3; 32], 1),
        action_id([1; 32], [2; 32], [3; 32], u64::MAX),
    ] {
        assert_ne!(base, changed);
    }
    assert_ne!(episode_id([1; 32], [2; 32], [3; 32]), base);
    for index in [
        0,
        23,
        24,
        255,
        256,
        65_535,
        65_536,
        4_294_967_295,
        4_294_967_296,
        u64::MAX,
    ] {
        let fixture = serde_json::json!({"kind":"action","execution_domain":"01".repeat(32),"agent_id":"02".repeat(32),"episode_id":"03".repeat(32),"action_index":index});
        let expected: Id32 = Sha256::digest(oracle(&fixture)).into();
        assert_eq!(action_id([1; 32], [2; 32], [3; 32], index), expected);
    }
}

#[test]
fn command_digest_binds_exact_record_and_domain_but_preserves_the_logical_slot() {
    let original = command_fixture(0);
    let slot = original.action_id();
    let base = command_digest([1; 32], &original);
    assert_ne!(base, command_digest([2; 32], &original));
    let arguments = hex("a161616131");
    let mut spec = CommandSpec {
        action_id: slot,
        tool_id: original.tool_id(),
        tool_version: original.tool_version(),
        authority_binding: original.authority_binding(),
        arguments: original.arguments_bytes(),
    };
    for changed in 0..4 {
        let mut input = spec;
        match changed {
            0 => input.tool_id = "different",
            1 => input.tool_version = u64::MAX,
            2 => input.authority_binding = [3; 32],
            _ => input.arguments = &arguments,
        }
        let competing = Command::encode(&input, &Limits::default()).unwrap();
        assert_eq!(competing.action_id(), slot);
        assert_ne!(command_digest([1; 32], &competing), base);
    }
    spec.action_id = [4; 32];
    let another = Command::encode(&spec, &Limits::default()).unwrap();
    assert_ne!(command_digest([1; 32], &another), base);
    assert_eq!(command_digest([1; 32], &original), base);
}

#[test]
fn large_valid_command_tail_is_streamed_without_a_second_encoded_preimage() {
    let mut arguments = BTreeMap::new();
    arguments.insert("note".into(), Cbor::Text("a".repeat(4096)));
    let arguments = map_bytes(arguments);
    let limits = Limits {
        max_command_bytes: 8192,
        ..Limits::default()
    };
    let command = Command::encode(
        &CommandSpec {
            action_id: [1; 32],
            tool_id: "trade",
            tool_version: 1,
            authority_binding: [2; 32],
            arguments: &arguments,
        },
        &limits,
    )
    .unwrap();
    let tree: Cbor = ciborium::de::from_reader(command.as_bytes()).unwrap();
    let preimage = bytes(&Cbor::Array(vec![
        Cbor::Text("raiv/command/1".into()),
        Cbor::Bytes(vec![3; 32]),
        tree,
    ]));
    let expected: Id32 = Sha256::digest(preimage).into();
    assert_eq!(command_digest([3; 32], &command), expected);
}
