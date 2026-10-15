//! Owns registered RAM reply advancement and staged local effects, not CommitCandidate
//! construction, provider JSON/error-message transport or host acceptance/dispatch.

use super::{
    Activation, ActivationError, InferenceError, ModelReply, RamDecision, RamProgress,
    RequestBinding,
};
use crate::{
    command::{Command, CommandSpec},
    definition::ToolControl,
    format::{validate_model_value, CodecError, DecodeLimits, ModelValue},
    identity::{action_id, episode_id},
    memory::{Memory, PatchArguments},
    tools::{BoundTool, PlanningContext, ToolImplementation},
    usage::Usage,
};

struct Work {
    memory: Option<Memory>,
    value: Option<ModelValue>,
    decision: Option<RamDecision>,
}

impl Work {
    fn result(&self) -> &[u8] {
        if let Some(value) = &self.value {
            value.as_bytes()
        } else if let Some(memory) = &self.memory {
            memory.value_bytes()
        } else {
            &[0xf6]
        }
    }
}

impl Activation<'_> {
    /// Core-owned call counts plus nullable exact reported cost sums. Reservations are
    /// deliberately not invented bills; unknown+known remains unknown, never zero.
    pub fn usage(&self) -> Usage {
        let counts = self.session.budget().used();
        Usage {
            model_calls: counts.model_calls,
            local_tool_calls: counts.local_tool_calls,
            ..self.usage_costs
        }
    }

    pub fn ram_decision(&self) -> Option<&RamDecision> {
        self.ram_decision.as_ref()
    }

    /// Complete matching replies may invoke at most one registered capability. This RAM
    /// milestone returns sealed planned Command/Finish data, NOT a CommitCandidate.
    /// Stale correlations preserve all RAM; matching malformed/fatal errors fail the run.
    pub fn advance_ram(
        &mut self,
        binding: RequestBinding,
        bytes: &[u8],
    ) -> Result<RamProgress, ActivationError> {
        self.session.check_response(binding)?;
        let reply = match self.session.accept_normalized_reply(
            binding,
            bytes,
            self.snapshot.definition(),
            &self.policy.caller,
        ) {
            Ok(reply) => reply,
            Err(error) => {
                if !matches!(
                    error,
                    super::ReplyError::Inference(InferenceError::WrongRequest)
                ) {
                    self.session.abort();
                }
                return Err(ActivationError::Reply(error));
            }
        };
        let result = self.advance_reply(&reply);
        if result.is_err() {
            self.session.abort();
        }
        result
    }

    fn advance_reply(&mut self, reply: &ModelReply<'_>) -> Result<RamProgress, ActivationError> {
        let reported = reply.usage();
        let costs = if self.reported_costs {
            Usage {
                input_tokens: add(self.usage_costs.input_tokens, reported.input_tokens)?,
                output_tokens: add(self.usage_costs.output_tokens, reported.output_tokens)?,
                billed_units: add(self.usage_costs.billed_units, reported.billed_units)?,
                ..self.usage_costs
            }
        } else {
            Usage {
                model_calls: 0,
                local_tool_calls: 0,
                input_tokens: reported.input_tokens,
                output_tokens: reported.output_tokens,
                billed_units: reported.billed_units,
            }
        };
        self.usage_costs = costs;
        self.reported_costs = true;
        let call = match reply.control_call() {
            Ok(Some(call)) => call,
            Ok(None) => return self.finish_reply(reply),
            Err(super::ReplyError::MultipleToolCalls) => {
                return self.correct(reply, None, "multiple_tool_calls", false)
            }
            Err(_) => return self.correct(reply, None, "invalid_arguments", false),
        };
        let Some(tool) = self.tools.lookup(call.meta().tool_id) else {
            return self.correct(reply, Some(call.arguments_bytes()), "unknown_tool", false);
        };
        if !self
            .request_tools()
            .any(|descriptor| descriptor.id == call.meta().tool_id)
        {
            return self.correct(reply, Some(call.arguments_bytes()), "disabled_tool", false);
        }
        let patch = if matches!(tool.implementation(), ToolImplementation::MemoryPatch) {
            match PatchArguments::decode(call.arguments_bytes(), &self.effective) {
                Ok(patch) => Some(patch),
                Err(error) => {
                    if matches!(
                        error,
                        crate::memory::PatchError::Codec(CodecError::AllocationFailure)
                    ) {
                        return Err(ActivationError::Patch(error));
                    }
                    return self.correct(
                        reply,
                        Some(call.arguments_bytes()),
                        error.feedback(),
                        false,
                    );
                }
            }
        } else {
            None
        };
        let control = tool.descriptor().control;
        self.session.begin_execution(
            control,
            &mut self.units,
            self.policy.quotes.control(control),
        )?;
        let work = match self.work(tool, call.arguments_bytes(), patch.as_ref()) {
            Ok(work) => work,
            Err(error) => {
                let code = match error {
                    ActivationError::Patch(crate::memory::PatchError::Codec(
                        CodecError::AllocationFailure,
                    ))
                    | ActivationError::Command(crate::command::CommandError::Codec(
                        CodecError::AllocationFailure,
                    ))
                    | ActivationError::Codec(CodecError::AllocationFailure) => return Err(error),
                    ActivationError::Patch(error) => error.feedback(),
                    ActivationError::Command(_) => "invalid_command",
                    _ => "tool_error",
                };
                return self.correct(reply, Some(call.arguments_bytes()), code, true);
            }
        };
        let trajectory = super::trajectory::append(
            &self.trajectory,
            reply,
            Some(call.arguments_bytes()),
            work.result(),
            None,
            &self.effective,
        )?;
        let context = self.prefix.context(&trajectory)?;
        self.session.complete_execution()?;
        if let Some(memory) = work.memory {
            if self.memory.as_ref() != Some(&memory) {
                self.memory = Some(memory);
            }
        }
        self.trajectory = trajectory;
        self.context = context;
        if let Some(decision) = work.decision {
            self.ram_decision = Some(decision);
            return Ok(RamProgress::Sealed);
        }
        self.next_request()?;
        Ok(RamProgress::InferAgain)
    }

    /// A complete normalized no-call reply is an intrinsic control decision, not a
    /// fabricated tool invocation. Stage all RAM output before sealing; errors remain fatal.
    fn finish_reply(&mut self, reply: &ModelReply<'_>) -> Result<RamProgress, ActivationError> {
        self.session.begin_execution(
            ToolControl::Finish,
            &mut self.units,
            self.policy.quotes.finish,
        )?;
        let trajectory = super::trajectory::append(
            &self.trajectory,
            reply,
            None,
            &[0xf6],
            None,
            &self.effective,
        )?;
        let context = self.prefix.context(&trajectory)?;
        self.session.complete_execution()?;
        self.trajectory = trajectory;
        self.context = context;
        self.ram_decision = Some(RamDecision::Finish);
        Ok(RamProgress::Sealed)
    }

    fn request_tools(&self) -> impl Iterator<Item = crate::definition::ToolDescriptorView<'_>> {
        super::runtime::enabled_controls(
            self.snapshot.definition(),
            self.session.budget().remaining(),
            self.units
                .remaining_activation_units()
                .min(self.units.remaining_episode_units()),
            self.policy.quotes,
        )
    }

    fn work(
        &self,
        tool: BoundTool<'_>,
        arguments: &[u8],
        patch: Option<&PatchArguments<'_>>,
    ) -> Result<Work, ActivationError> {
        let mut work = Work {
            memory: None,
            value: None,
            decision: None,
        };
        match tool.implementation() {
            ToolImplementation::MemoryPatch => {
                let patch = patch.expect("decoded intrinsic patch");
                let before = self
                    .memory
                    .as_ref()
                    .expect("bound memory-enabled capability");
                let accepted = patch.patched(before).map_err(ActivationError::Patch)?;
                work.value = Some(
                    patch
                        .report(before, &accepted)
                        .map_err(ActivationError::Patch)?,
                );
                work.memory = Some(accepted);
            }
            ToolImplementation::Pure { validate, plan }
            | ToolImplementation::Yield { validate, plan } => {
                validate(arguments).map_err(ActivationError::Tool)?;
                let value = plan(
                    PlanningContext {
                        definition: self.snapshot.definition(),
                        input: self.input.input(),
                        memory: self.memory.as_ref(),
                        limits: &self.effective,
                    },
                    arguments,
                )
                .map_err(ActivationError::Tool)?;
                validate_model_value(
                    value.as_bytes(),
                    &DecodeLimits {
                        max_block_bytes: self.effective.max_context_bytes,
                        max_depth: self.effective.max_depth,
                        max_collection_entries: self.effective.max_collection_entries,
                    },
                )?;
                if tool.descriptor().control == ToolControl::Yield {
                    let definition = self.snapshot.definition();
                    let agent = self.snapshot.state().agent_id();
                    let episode = self
                        .snapshot
                        .current_episode()
                        .map(|episode| episode.id())
                        .unwrap_or_else(|| {
                            episode_id(
                                definition.execution_domain(),
                                agent,
                                self.input.input().event_id(),
                            )
                        });
                    let command = Command::encode(
                        &CommandSpec {
                            action_id: action_id(
                                definition.execution_domain(),
                                agent,
                                episode,
                                self.session.budget().accepted_episode_counts().actions,
                            ),
                            tool_id: tool.descriptor().id,
                            tool_version: tool.descriptor().version,
                            authority_binding: tool
                                .descriptor()
                                .authority_binding
                                .expect("declared Yield authority"),
                            arguments: value.as_bytes(),
                        },
                        &self.effective,
                    )
                    .map_err(ActivationError::Command)?;
                    work.decision = Some(RamDecision::Yield(command));
                } else {
                    work.value = Some(value);
                }
            }
        }
        Ok(work)
    }

    fn correct(
        &mut self,
        reply: &ModelReply<'_>,
        arguments: Option<&[u8]>,
        error: &str,
        admitted: bool,
    ) -> Result<RamProgress, ActivationError> {
        let trajectory = super::trajectory::append(
            &self.trajectory,
            reply,
            arguments,
            &[0xf6],
            Some(error),
            &self.effective,
        )?;
        let context = self.prefix.context(&trajectory)?;
        if admitted {
            self.session.failed_execution()?;
        } else {
            self.session.corrective_turn()?;
        }
        self.trajectory = trajectory;
        self.context = context;
        self.next_request()?;
        Ok(RamProgress::InferAgain)
    }

    fn next_request(&mut self) -> Result<(), ActivationError> {
        self.units
            .check(self.session.budget().binding(), self.policy.quotes.model)
            .map_err(|error| ActivationError::Inference(InferenceError::Units(error)))?;
        let available = self
            .units
            .remaining_activation_units()
            .min(self.units.remaining_episode_units())
            - self.policy.quotes.model;
        if available == 0
            || (self.policy.quotes.finish > available
                && !self
                    .request_tools()
                    .any(|tool| self.policy.quotes.control(tool.control) <= available))
        {
            return Err(ActivationError::NoEnabledControl);
        }
        self.session
            .issue_with_units(&mut self.units, self.policy.quotes.model)?;
        Ok(())
    }
}

fn add(old: Option<u64>, new: Option<u64>) -> Result<Option<u64>, ActivationError> {
    match (old, new) {
        (Some(a), Some(b)) => Ok(Some(
            a.checked_add(b).ok_or(ActivationError::UsageOverflow)?,
        )),
        _ => Ok(None),
    }
}
