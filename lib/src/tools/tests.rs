use super::*;
use crate::definition::DefinitionSpec;
use crate::format::{
    oracle::{encode, fixture_value, map_mut},
    DecodeLimits,
};
use alloc::vec::Vec;
use ciborium::Value as Cbor;
use core::sync::atomic::{AtomicUsize, Ordering};
use serde_json::Value;

static CALLBACKS: AtomicUsize = AtomicUsize::new(0);

fn validate(_: &[u8]) -> Result<(), ToolFailure> {
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
    Err(ToolFailure::InvalidArguments)
}

fn plan(_: PlanningContext<'_>, _: &[u8]) -> Result<ModelValue, ToolFailure> {
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
    Err(ToolFailure::PlanningFailed)
}

fn definition(index: usize) -> Definition {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../fixtures/format/definition.json")).unwrap();
    Definition::decode(
        &encode(&fixture_value(&fixtures[index]["record"])),
        &DecodeLimits::default(),
    )
    .unwrap()
}

fn implementation(control: ToolControl) -> ToolImplementation {
    match control {
        ToolControl::Continue => ToolImplementation::MemoryPatch,
        ToolControl::Finish => panic!("Finish is intrinsic, not a registered tool"),
        ToolControl::Yield => ToolImplementation::Yield { validate, plan },
    }
}

fn registrations(definition: &Definition) -> Vec<ToolRegistration<'_>> {
    definition
        .tools()
        .map(|descriptor| ToolRegistration {
            implementation: implementation(descriptor.control),
            descriptor,
        })
        .collect()
}

fn pure_definition(profile: MemoryProfile) -> Definition {
    Definition::encode(
        &DefinitionSpec {
            execution_domain: [1; 32],
            policy: "fixture",
            mandate: "fixture",
            memory_profile: profile,
            tools: &[ToolDescriptorView {
                id: "alias",
                version: 1,
                control: ToolControl::Continue,
                description: "Pure or patch capability",
                arguments_schema: &[0xa0],
                authority_binding: None,
            }],
            inference_profile: &[0xa0],
            limits: Limits::default(),
        },
        &DecodeLimits::default(),
    )
    .unwrap()
}

#[test]
fn exact_bijection_preserves_definition_order_and_borrowed_descriptor_bytes_without_callbacks() {
    for index in 0..2 {
        let definition = definition(index);
        let mut registrations = registrations(&definition);
        registrations.reverse();
        let before = definition.as_bytes().to_vec();
        let cid = definition.cid();
        let tools = ToolSet::bind(&definition, &registrations, &Limits::default()).unwrap();
        assert_eq!(tools.definition_cid(), cid);
        for (actual, expected) in tools.tools(&definition).unwrap().zip(definition.tools()) {
            assert_eq!(actual.descriptor(), expected);
            assert_eq!(
                actual.descriptor().arguments_schema.as_ptr(),
                expected.arguments_schema.as_ptr()
            );
            assert_eq!(actual.descriptor().id.as_ptr(), expected.id.as_ptr());
            assert_eq!(actual.implementation().control(), expected.control);
            assert_eq!(tools.lookup(expected.id).unwrap().descriptor(), expected);
        }
        assert!(tools.lookup("outside-whitelist").is_none());
        assert_eq!(definition.as_bytes(), before);
    }
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn extra_missing_duplicate_and_unknown_registrations_reject_without_partial_binding() {
    let definition = definition(0);
    let registrations = registrations(&definition);
    assert_eq!(
        ToolSet::bind(
            &definition,
            &registrations[..registrations.len() - 1],
            &Limits::default()
        )
        .err(),
        Some(ToolSetError::MissingRegistration)
    );
    let mut duplicate = registrations.clone();
    duplicate.push(duplicate[0]);
    assert_eq!(
        ToolSet::bind(&definition, &duplicate, &Limits::default()).err(),
        Some(ToolSetError::DuplicateRegistration)
    );
    let mut extra = registrations.clone();
    let mut unknown = extra[0];
    unknown.descriptor.id = "unknown";
    extra.push(unknown);
    assert_eq!(
        ToolSet::bind(&definition, &extra, &Limits::default()).err(),
        Some(ToolSetError::UnlistedRegistration)
    );
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn every_pinned_metadata_component_is_checked_independently_of_implementation_identity() {
    let definition = definition(0);
    let baseline = registrations(&definition);
    let cases: [(ToolSetError, ToolDescriptorView<'_>); 5] = [
        (
            ToolSetError::VersionMismatch,
            ToolDescriptorView {
                version: u64::MAX,
                ..baseline[0].descriptor
            },
        ),
        (
            ToolSetError::ControlMismatch,
            ToolDescriptorView {
                control: ToolControl::Continue,
                ..baseline[0].descriptor
            },
        ),
        (
            ToolSetError::DescriptionMismatch,
            ToolDescriptorView {
                description: "changed instruction",
                ..baseline[0].descriptor
            },
        ),
        (
            ToolSetError::SchemaMismatch,
            ToolDescriptorView {
                arguments_schema: &[0xa0],
                ..baseline[0].descriptor
            },
        ),
        (
            ToolSetError::AuthorityMismatch,
            ToolDescriptorView {
                authority_binding: Some([99; 32]),
                ..baseline[0].descriptor
            },
        ),
    ];
    for (error, descriptor) in cases {
        let mut actual = baseline.clone();
        actual[0].descriptor = descriptor;
        assert_eq!(
            ToolSet::bind(&definition, &actual, &Limits::default()).err(),
            Some(error)
        );
    }
    for malformed in [
        &[][..],
        &[0xa1][..],
        &[0xf6][..],
        &[0xa1, 0x61, b'x', 0x18, 0][..],
    ] {
        let mut actual = baseline.clone();
        actual[0].descriptor.arguments_schema = malformed;
        assert_eq!(
            ToolSet::bind(&definition, &actual, &Limits::default()).err(),
            Some(ToolSetError::SchemaMismatch)
        );
    }
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn implementation_control_and_memory_profile_are_capability_checks_not_name_heuristics() {
    let definition = definition(0);
    let mut actual = registrations(&definition);
    actual[0].implementation = ToolImplementation::MemoryPatch;
    assert_eq!(
        ToolSet::bind(&definition, &actual, &Limits::default()).err(),
        Some(ToolSetError::ImplementationControlMismatch)
    );
    let none = pure_definition(MemoryProfile::None);
    let mut actual = registrations(&none);
    assert_eq!(actual[0].descriptor.id, "alias");
    assert_eq!(
        ToolSet::bind(&none, &actual, &Limits::default()).err(),
        Some(ToolSetError::MemoryDisabled)
    );
    actual[0].implementation = ToolImplementation::Pure { validate, plan };
    assert!(ToolSet::bind(&none, &actual, &Limits::default()).is_ok());
    let memory = pure_definition(MemoryProfile::IntentsStrategyTactics);
    let actual = registrations(&memory);
    assert!(ToolSet::bind(&memory, &actual, &Limits::default()).is_ok());
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn bound_set_does_not_retain_the_source_definition_and_rejects_a_different_definition_cid() {
    let actual = [ToolRegistration {
        descriptor: ToolDescriptorView {
            id: "alias",
            version: 1,
            control: ToolControl::Continue,
            description: "Pure or patch capability",
            arguments_schema: &[0xa0],
            authority_binding: None,
        },
        implementation: ToolImplementation::Pure { validate, plan },
    }];
    let (bound, cid) = {
        let source = pure_definition(MemoryProfile::None);
        let cid = source.cid();
        (
            ToolSet::bind(&source, &actual, &Limits::default()).unwrap(),
            cid,
        )
    }; // The owned Definition is dropped; registered metadata remains independent.
    assert_eq!(bound.definition_cid(), cid);
    assert!(bound.lookup("alias").is_some());
    let source = pure_definition(MemoryProfile::None);
    assert_eq!(bound.tools(&source).unwrap().count(), 1);
    let mut changed: Cbor = ciborium::de::from_reader(source.as_bytes()).unwrap();
    map_mut(&mut changed).insert("policy".into(), Cbor::Text("other policy".into()));
    let changed = Definition::decode(&encode(&changed), &DecodeLimits::default()).unwrap();
    assert_eq!(
        bound.tools(&changed).err(),
        Some(ToolSetError::DefinitionMismatch)
    );
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 0);
}

#[test]
fn full_u64_versions_zero_and_definition_caller_count_caps_are_not_narrowed_or_overridden() {
    let definition = definition(0);
    let mut value: Cbor = ciborium::de::from_reader(definition.as_bytes()).unwrap();
    let Cbor::Array(tools) = map_mut(&mut value).get_mut("tools").unwrap() else {
        panic!("tools")
    };
    map_mut(&mut tools[0]).insert("version".into(), Cbor::Integer(u64::MAX.into()));
    let definition = Definition::decode(&encode(&value), &DecodeLimits::default()).unwrap();
    let actual = registrations(&definition);
    let bound = ToolSet::bind(
        &definition,
        &actual,
        &Limits {
            max_collection_entries: u64::MAX,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(
        bound
            .tools(&definition)
            .unwrap()
            .next()
            .unwrap()
            .descriptor()
            .version,
        u64::MAX
    );
    assert_eq!(
        ToolSet::bind(
            &definition,
            &actual,
            &Limits {
                max_collection_entries: actual.len() as u64 - 1,
                ..Limits::default()
            }
        )
        .err(),
        Some(ToolSetError::CountLimit)
    );
    assert_eq!(
        ToolSet::bind(
            &definition,
            &actual,
            &Limits {
                max_collection_entries: 0,
                ..Limits::default()
            }
        )
        .err(),
        Some(ToolSetError::CountLimit)
    );
    let empty = Definition::encode(
        &DefinitionSpec {
            execution_domain: [1; 32],
            policy: "fixture",
            mandate: "fixture",
            memory_profile: MemoryProfile::None,
            tools: &[],
            inference_profile: &[0xa0],
            limits: Limits::default(),
        },
        &DecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(
        ToolSet::bind(
            &empty,
            &[],
            &Limits {
                max_collection_entries: 0,
                ..Limits::default()
            }
        )
        .unwrap()
        .tools(&empty)
        .unwrap()
        .count(),
        0
    );
    let count = actual.len() as u64;
    map_mut(map_mut(&mut value).get_mut("limits").unwrap()).insert(
        "max_collection_entries".into(),
        Cbor::Integer((count - 1).into()),
    );
    let narrow = Definition::decode(&encode(&value), &DecodeLimits::default()).unwrap();
    assert_eq!(
        ToolSet::bind(
            &narrow,
            &actual,
            &Limits {
                max_collection_entries: u64::MAX,
                ..Limits::default()
            }
        )
        .err(),
        Some(ToolSetError::CountLimit)
    );
}
