//! Registry-wrapper lifetime is independent of retained Activation; metadata still borrows.
use super::*;
use crate::{
    activation::{prepare, Activation, InferenceError, RamProgress, RequestSequence},
    definition::ToolControl,
    format::oracle::json_bytes,
    preparation::PreparedInput,
    tools::{ToolImplementation, ToolRegistration, ToolSet},
};
use serde_json::json;

// This return was impossible when Activation borrowed the local ToolSet wrapper.
fn scoped_prepare<'registrations>(
    snapshot: RestoredAgent,
    input: PreparedInput,
    definition: &Definition,
    registrations: &'registrations [ToolRegistration<'registrations>],
) -> Activation<'registrations> {
    let registry = ToolSet::bind(definition, registrations, &Limits::default()).unwrap();
    prepare(
        snapshot,
        input,
        &registry,
        activation::policy(),
        RequestSequence::new([93; 32]),
    )
    .unwrap_or_else(|e| panic!("prepare: {}", e.reason))
}

#[test]
fn scoped_registry_wrapper_drops_then_moved_activation_preserves_binding_ram_and_exact_candidates()
{
    let mut enabled = 0;
    let mut disabled = 0;
    let mut finishes = 0;
    let mut yields = 0;
    for (blocks, root) in restored::valid_snapshots() {
        for finish in [true, false] {
            let snapshot = RestoredAgent::restore(root, &blocks, &Limits::default()).unwrap();
            let definition =
                Definition::decode(snapshot.definition().as_bytes(), &DecodeLimits::default())
                    .unwrap();
            let selected = definition.tools().find(|t| t.control == ToolControl::Yield);
            if !finish && selected.is_none() {
                continue;
            }
            let mut registrations = preparation::registrations(&definition);
            for registration in &mut registrations {
                registration.implementation = match registration.descriptor.control {
                    ToolControl::Continue if snapshot.memory().is_some() => {
                        ToolImplementation::MemoryPatch
                    }
                    ToolControl::Continue => ToolImplementation::Pure {
                        validate: candidate::validate,
                        plan: candidate::plan,
                    },
                    ToolControl::Yield => ToolImplementation::Yield {
                        validate: candidate::validate,
                        plan: candidate::plan,
                    },
                    ToolControl::Finish => panic!("Finish is intrinsic, not a registered tool"),
                };
            }
            let source = candidate::incoming(&snapshot, 182);
            let packet = preparation::packet(&snapshot, &source);
            let run = scoped_prepare(snapshot, packet, &definition, &registrations);
            let prefix = run.initial_prefix().to_vec();
            let prefix_ptr = run.initial_prefix().as_ptr();
            let context_ptr = run.request().unwrap().context_bytes().as_ptr();
            let pending = run.request().unwrap().binding();
            let block_ptrs = run
                .snapshot()
                .blocks()
                .map(|(cid, bytes)| (cid, bytes.as_ptr()))
                .collect::<BTreeMap<_, _>>();
            // Model the guest's owning live-Activation slot, not self-referential registry storage.
            let mut resident = alloc::boxed::Box::new(Some(run));
            let run = resident.as_mut().as_mut().unwrap();
            assert_eq!(run.initial_prefix().as_ptr(), prefix_ptr);
            assert_eq!(run.request().unwrap().context_bytes().as_ptr(), context_ptr);
            assert_eq!(run.request().unwrap().binding(), pending);
            assert_eq!(
                run.snapshot()
                    .blocks()
                    .map(|(cid, b)| (cid, b.as_ptr()))
                    .collect::<BTreeMap<_, _>>(),
                block_ptrs
            );
            assert_eq!(run.registered_tools().definition_cid(), definition.cid());
            for registered in &registrations {
                let actual = run
                    .registered_tools()
                    .lookup(registered.descriptor.id)
                    .unwrap()
                    .descriptor();
                assert_eq!(actual, registered.descriptor);
                assert_eq!(actual.id.as_ptr(), registered.descriptor.id.as_ptr());
                assert_eq!(
                    actual.arguments_schema.as_ptr(),
                    registered.descriptor.arguments_schema.as_ptr()
                );
            }
            assert!(run.registered_tools().lookup("unlisted").is_none());
            let before = (run.usage(), run.units().reserved_units());
            let wrong = crate::activation::RequestBinding {
                request_id: pending.request_id + 1,
                ..pending
            };
            assert_eq!(
                run.advance_ram(wrong, &[0xff]),
                Err(crate::activation::ActivationError::Inference(
                    InferenceError::WrongRequest
                ))
            );
            assert_eq!((run.usage(), run.units().reserved_units()), before);
            assert_eq!(run.request().unwrap().binding(), pending);
            if run.working_memory().is_some() {
                enabled += 1;
                if let Some(patch) = definition
                    .tools()
                    .find(|t| t.control == ToolControl::Continue)
                {
                    assert_eq!(
                        candidate::advance(
                            run,
                            patch.id,
                            cbor(&json_bytes(&json!({"intents":{"registry_lifetime":true}})))
                        ),
                        RamProgress::InferAgain
                    );
                    assert_eq!(run.initial_prefix(), prefix);
                    assert_eq!(run.initial_prefix().as_ptr(), prefix_ptr);
                    assert_eq!(run.request().unwrap().binding().request_id, 2);
                    assert_eq!(run.usage().local_tool_calls, 1);
                }
            } else {
                disabled += 1;
                assert!(run
                    .registered_tools()
                    .tools(&definition)
                    .unwrap()
                    .all(|t| !matches!(t.implementation(), ToolImplementation::MemoryPatch)));
            }
            assert_eq!(
                if finish {
                    candidate::finish(run)
                } else {
                    candidate::advance(run, selected.unwrap().id, Cbor::Map(vec![]))
                },
                RamProgress::Sealed
            );
            let checkpoint = run.candidate().unwrap();
            candidate::verify(run, &checkpoint, &source, finish);
            let sequence = run.release_sequence().unwrap();
            assert_eq!(sequence.instance_id(), [93; 32]);
            let root_after = checkpoint.new_root();
            // Neither candidate bytes nor a terminal sequence borrow the wrapper/live run.
            drop(resident);
            assert_eq!(checkpoint.snapshot().root(), root_after);
            assert_eq!(checkpoint.command_bytes().is_none(), finish);
            if finish {
                finishes += 1;
            } else {
                yields += 1;
            }
        }
    }
    assert!(enabled > 0 && disabled > 0 && finishes > 0 && yields > 0);
}
