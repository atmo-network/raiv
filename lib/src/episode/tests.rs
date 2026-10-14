use super::*;
use crate::format::oracle::{encode as bytes, fixture_value as node, link, map_mut as map};
use alloc::{string::ToString, vec};
use ciborium::value::Value as Cbor;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/format/episode.json")).unwrap()
}
fn record(index: usize) -> Cbor {
    node(&fixtures()[index]["record"])
}
fn encode(episode: &Episode, bounds: &DecodeLimits) -> Result<Episode, EpisodeError> {
    let links = episode.boundaries().collect::<Vec<_>>();
    Episode::encode(
        &EpisodeSpec {
            id: episode.id(),
            trigger_event_id: episode.trigger_event_id(),
            entry_memory: episode.entry_memory_link(),
            boundaries: &links,
            closed: episode.closed(),
        },
        bounds,
    )
}

#[test]
fn complete_episode_vectors_match_independent_codec_hash_cid_and_owned_encoding() {
    for fixture in fixtures().as_array().unwrap() {
        let expected = hex(fixture["hex"].as_str().unwrap());
        assert_eq!(bytes(&node(&fixture["record"])), expected);
        assert_eq!(
            &Sha256::digest(&expected)[..],
            hex(fixture["sha256"].as_str().unwrap())
        );
        let episode = Episode::decode(&expected, &DecodeLimits::default()).unwrap();
        assert_eq!(episode.as_bytes(), expected);
        assert_eq!(episode.cid().to_string(), fixture["cid"].as_str().unwrap());
        assert_eq!(
            episode.id().to_vec(),
            hex(fixture["record"]["id"]["$bytes"].as_str().unwrap())
        );
        assert_eq!(
            episode.trigger_event_id().to_vec(),
            hex(fixture["record"]["trigger_event_id"]["$bytes"]
                .as_str()
                .unwrap())
        );
        assert_eq!(
            episode
                .entry_memory_link()
                .map(|v| v.to_string())
                .as_deref(),
            fixture["record"]["entry_memory"]["$link"].as_str()
        );
        assert_eq!(
            episode.closed(),
            fixture["record"]["closed"].as_bool().unwrap()
        );
        let links = fixture["record"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["$link"].as_str().unwrap().parse::<DaslCid>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(episode.boundary_count(), links.len() as u64);
        assert_eq!(episode.boundaries().collect::<Vec<_>>(), links);
        let owned = encode(&episode, &DecodeLimits::default()).unwrap();
        assert_eq!(owned, episode);
        drop(episode);
        assert_eq!(owned.as_bytes(), expected);
        assert_eq!(owned.boundaries().collect::<Vec<_>>(), links);
    }
}

#[test]
fn root_membership_kind_and_version_are_exact_without_defaults() {
    let mut source = record(0);
    let keys = map(&mut source).keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let mut changed = source.clone();
        map(&mut changed).remove(&key);
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
        map(&mut changed).insert("unknown".into(), Cbor::Null);
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
    }
    for (key, value, error) in [
        (
            "kind",
            Cbor::Text("boundary".into()),
            EpisodeError::WrongKind,
        ),
        ("kind", Cbor::Null, EpisodeError::InvalidSchema),
        (
            "version",
            Cbor::Integer(0.into()),
            EpisodeError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(2.into()),
            EpisodeError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer(u64::MAX.into()),
            EpisodeError::UnsupportedVersion,
        ),
        (
            "version",
            Cbor::Integer((-1).into()),
            EpisodeError::InvalidSchema,
        ),
    ] {
        let mut changed = source.clone();
        map(&mut changed).insert(key.into(), value);
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(error)
        );
    }
    for value in [Cbor::Null, Cbor::Array(vec![]), Cbor::Integer(0.into())] {
        assert_eq!(
            Episode::decode(&bytes(&value), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
    }
}

#[test]
fn episode_and_trigger_ids_are_raw_id32_without_link_or_text_coercion() {
    let link = link(
        "bafyreigbtj4x7ip5legnfznufuopl4sg4knzc2cof6duas4b3q2fy6swua"
            .parse()
            .unwrap(),
    );
    for key in ["id", "trigger_event_id"] {
        for value in [
            Cbor::Null,
            Cbor::Bytes(vec![0; 31]),
            Cbor::Bytes(vec![0; 33]),
            Cbor::Text("id".into()),
            link.clone(),
        ] {
            let mut changed = record(0);
            map(&mut changed).insert(key.into(), value);
            assert_eq!(
                Episode::decode(&bytes(&changed), &DecodeLimits::default()),
                Err(EpisodeError::InvalidSchema)
            );
        }
    }
}

#[test]
fn nullable_entry_memory_and_boundary_elements_require_structural_tag42_links() {
    let cid = DaslCid::from_digest(DaslCodec::Drisl, [1; 32]);
    let raw = DaslCid::from_digest(DaslCodec::Raw, [2; 32]);
    for value in [
        Cbor::Bytes(cid.as_bytes().to_vec()),
        Cbor::Bytes(vec![0; 32]),
        Cbor::Text(cid.to_string()),
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("entry_memory".into(), value.clone());
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
        map(&mut changed).insert("entry_memory".into(), Cbor::Null);
        map(&mut changed).insert("boundaries".into(), Cbor::Array(vec![value]));
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
    }
    for value in [
        Cbor::Null,
        Cbor::Integer(0.into()),
        Cbor::Text("cid".into()),
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("boundaries".into(), value.clone());
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
        map(&mut changed).insert("boundaries".into(), Cbor::Array(vec![value]));
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
    }
    let raw_link = link(raw);
    for key in ["entry_memory", "boundaries"] {
        let mut changed = record(0);
        map(&mut changed).insert(
            key.into(),
            if key == "boundaries" {
                Cbor::Array(vec![raw_link.clone()])
            } else {
                raw_link.clone()
            },
        );
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::Codec(CodecError::InvalidLink))
        );
    }
    let source = Episode::decode(&bytes(&record(0)), &DecodeLimits::default()).unwrap();
    let base = EpisodeSpec {
        id: source.id(),
        trigger_event_id: source.trigger_event_id(),
        entry_memory: None,
        boundaries: &[],
        closed: false,
    };
    assert_eq!(
        Episode::encode(
            &EpisodeSpec {
                entry_memory: Some(raw),
                ..base
            },
            &DecodeLimits::default()
        ),
        Err(EpisodeError::Codec(CodecError::InvalidLink))
    );
    assert_eq!(
        Episode::encode(
            &EpisodeSpec {
                boundaries: &[raw],
                ..base
            },
            &DecodeLimits::default()
        ),
        Err(EpisodeError::Codec(CodecError::InvalidLink))
    );
}

#[test]
fn closed_is_exact_boolean_and_never_nullable_or_integer_truthiness() {
    for value in [
        Cbor::Null,
        Cbor::Integer(0.into()),
        Cbor::Integer(1.into()),
        Cbor::Text("true".into()),
        Cbor::Bytes(vec![1]),
        Cbor::Array(vec![]),
    ] {
        let mut changed = record(0);
        map(&mut changed).insert("closed".into(), value);
        assert_eq!(
            Episode::decode(&bytes(&changed), &DecodeLimits::default()),
            Err(EpisodeError::InvalidSchema)
        );
    }
    for closed in [false, true] {
        let mut changed = record(0);
        map(&mut changed).insert("closed".into(), Cbor::Bool(closed));
        let expected = bytes(&changed);
        let episode = Episode::decode(&expected, &DecodeLimits::default()).unwrap();
        assert_eq!(episode.closed(), closed);
        assert_eq!(
            encode(&episode, &DecodeLimits::default())
                .unwrap()
                .as_bytes(),
            expected
        );
    }
}

#[test]
fn list_order_and_duplicates_are_retained_without_a_second_owned_cid_list() {
    let links = [
        DaslCid::from_digest(DaslCodec::Drisl, [3; 32]),
        DaslCid::from_digest(DaslCodec::Drisl, [1; 32]),
        DaslCid::from_digest(DaslCodec::Drisl, [3; 32]),
    ];
    let base = EpisodeSpec {
        id: [1; 32],
        trigger_event_id: [2; 32],
        entry_memory: None,
        boundaries: &links,
        closed: false,
    };
    let episode = Episode::encode(&base, &DecodeLimits::default()).unwrap();
    assert_eq!(episode.boundaries().collect::<Vec<_>>(), links);
    assert_eq!(episode.boundary_count(), 3);
    let different = [links[1], links[0], links[2]];
    let reordered = Episode::encode(
        &EpisodeSpec {
            boundaries: &different,
            ..base
        },
        &DecodeLimits::default(),
    )
    .unwrap();
    assert_ne!(reordered.cid(), episode.cid());
    let list = &episode.as_bytes()[episode.boundaries.clone()];
    let expected = [
        vec![0x83],
        links
            .iter()
            .flat_map(|cid| [vec![0xd8, 0x2a, 0x58, 0x25, 0], cid.as_bytes().to_vec()].concat())
            .collect(),
    ]
    .concat();
    assert_eq!(list, expected);
    assert_eq!(
        Episode::decode(episode.as_bytes(), &DecodeLimits::default()).unwrap(),
        episode
    );
}

#[test]
fn byte_and_root_list_entry_caps_are_caller_owned() {
    let episode = Episode::decode(&bytes(&record(2)), &DecodeLimits::default()).unwrap();
    let exact = DecodeLimits {
        max_block_bytes: episode.as_bytes().len() as u64,
        max_depth: 4,
        max_collection_entries: 7,
    };
    assert_eq!(encode(&episode, &exact).unwrap(), episode);
    for (bounds, error) in [
        (
            DecodeLimits {
                max_block_bytes: exact.max_block_bytes - 1,
                ..exact
            },
            CodecError::ByteLimit,
        ),
        (
            DecodeLimits {
                max_collection_entries: 6,
                ..exact
            },
            CodecError::CollectionLimit,
        ),
        (
            DecodeLimits {
                max_depth: 3,
                ..exact
            },
            CodecError::DepthLimit,
        ),
    ] {
        assert_eq!(
            Episode::decode(episode.as_bytes(), &bounds),
            Err(EpisodeError::Codec(error))
        );
        assert_eq!(encode(&episode, &bounds), Err(EpisodeError::Codec(error)));
    }
    let links = vec![DaslCid::from_digest(DaslCodec::Drisl, [1; 32]); 8];
    let spec = EpisodeSpec {
        id: [1; 32],
        trigger_event_id: [2; 32],
        entry_memory: None,
        boundaries: &links,
        closed: false,
    };
    let bounds = DecodeLimits {
        max_collection_entries: 8,
        ..DecodeLimits::default()
    };
    let eight = Episode::encode(&spec, &bounds).unwrap();
    let short = DecodeLimits {
        max_collection_entries: 7,
        ..bounds
    };
    assert_eq!(
        Episode::encode(&spec, &short),
        Err(EpisodeError::Codec(CodecError::CollectionLimit))
    );
    assert_eq!(
        Episode::decode(eight.as_bytes(), &short),
        Err(EpisodeError::Codec(CodecError::CollectionLimit))
    );
}

#[test]
fn minimum_depth_distinguishes_empty_list_memory_tag_and_boundary_tag_payload() {
    let source = Episode::decode(&bytes(&record(0)), &DecodeLimits::default()).unwrap();
    let base = EpisodeSpec {
        id: source.id(),
        trigger_event_id: source.trigger_event_id(),
        entry_memory: None,
        boundaries: &[],
        closed: false,
    };
    let link = DaslCid::from_digest(DaslCodec::Drisl, [3; 32]);
    for (spec, depth) in [
        (base, 2),
        (
            EpisodeSpec {
                entry_memory: Some(link),
                ..base
            },
            3,
        ),
        (
            EpisodeSpec {
                boundaries: &[link],
                ..base
            },
            4,
        ),
    ] {
        let bounds = DecodeLimits {
            max_depth: depth,
            ..DecodeLimits::default()
        };
        let episode = Episode::encode(&spec, &bounds).unwrap();
        assert_eq!(
            Episode::decode(episode.as_bytes(), &bounds).unwrap(),
            episode
        );
        let shallow = DecodeLimits {
            max_depth: depth - 1,
            ..bounds
        };
        assert_eq!(
            Episode::encode(&spec, &shallow),
            Err(EpisodeError::Codec(CodecError::DepthLimit))
        );
        assert_eq!(
            Episode::decode(episode.as_bytes(), &shallow),
            Err(EpisodeError::Codec(CodecError::DepthLimit))
        );
    }
}

#[test]
fn malformed_truncated_duplicate_nonminimal_and_trailing_episodes_reject() {
    let encoded = bytes(&record(0));
    for end in 0..encoded.len() {
        assert!(Episode::decode(&encoded[..end], &DecodeLimits::default()).is_err());
    }
    for (data, error) in [
        (hex("a26269644062696440"), CodecError::DuplicateKey),
        (
            [vec![0xb8, 7], encoded[1..].to_vec()].concat(),
            CodecError::NonCanonical,
        ),
        ([encoded, vec![0]].concat(), CodecError::TrailingBytes),
    ] {
        assert_eq!(
            Episode::decode(&data, &DecodeLimits::default()),
            Err(EpisodeError::Codec(error))
        );
    }
}
