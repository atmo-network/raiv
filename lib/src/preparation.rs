//! Owns prepared-packet data binding and borrowed preparation checks, not host provenance.
//! No planner/inference, counters/units consumption, checkpoint or ready Activation here.

use crate::{
    boundary::{BoundaryError, Exit},
    command::Command,
    format::{validate_drisl, CodecError, DaslCid, Id32},
    graph::{GraphError, HistoryConsistencyError, RestoredAgent},
    input::{ActivationInput, Cause, InputError},
    limits::Limits,
    receipt::Receipt,
    state::Continuation,
    tools::ToolSet,
};
use core::fmt;

/// Host-supplied admission binding. Equality is checked, not cryptographic host approval.
/// The input digest is SHA-256 of the exact ActivationInput DRISL record (its CID digest).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputBinding {
    pub agent_id: Id32,
    pub execution_domain: Id32,
    pub expected_root: DaslCid,
    pub expected_revision: u64,
    pub admitted_event_id: Id32,
    pub admitted_input_digest: Id32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputCompatibilityError {
    Input(InputError),
    WrongPhase,
    ReceiptMismatch,
}

impl fmt::Display for InputCompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(f),
            Self::WrongPhase => f.write_str("input cause does not match continuation"),
            Self::ReceiptMismatch => f.write_str("Receipt does not match pending Command"),
        }
    }
}

impl core::error::Error for InputCompatibilityError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareError {
    InvalidRootCodec,
    EventBindingMismatch,
    InputDigestMismatch,
    WrongAgent,
    WrongDomain,
    WrongRoot,
    WrongRevision,
    WrongDefinition,
    KnownEventReplay,
    Graph(GraphError),
    Compatibility(InputCompatibilityError),
}

impl From<GraphError> for PrepareError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<InputCompatibilityError> for PrepareError {
    fn from(error: InputCompatibilityError) -> Self {
        Self::Compatibility(error)
    }
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Graph(error) => return error.fmt(f),
            Self::Compatibility(error) => return error.fmt(f),
            Self::InvalidRootCodec => "prepared binding requires a structural root CID",
            Self::EventBindingMismatch => "prepared event ID does not match input",
            Self::InputDigestMismatch => "prepared digest does not match exact input bytes",
            Self::WrongAgent => "prepared binding has another agent",
            Self::WrongDomain => "prepared binding has another execution domain",
            Self::WrongRoot => "prepared binding has another root",
            Self::WrongRevision => "prepared binding has another revision",
            Self::WrongDefinition => "ToolSet is bound to another Definition",
            Self::KnownEventReplay => "event already occurs in retained checkpoint data",
        })
    }
}

impl core::error::Error for PrepareError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::Compatibility(error) => Some(error),
            _ => None,
        }
    }
}

/// One exact host packet. Construction takes existing owned bytes without another copy.
/// Private fields prevent changing identity/digest binding after construction. Its type
/// alone proves no Actor/Receipt source, freshness beyond retained data, host admission,
/// authorized program/run, canonical HEAD or grant/lifetime/unit approval.
#[derive(Debug, Eq, PartialEq)]
pub struct PreparedInput {
    binding: InputBinding,
    input: ActivationInput,
}

impl PreparedInput {
    pub fn new(binding: InputBinding, input: ActivationInput) -> Result<Self, PrepareError> {
        binding
            .expected_root
            .require_state()
            .map_err(|_| PrepareError::InvalidRootCodec)?;
        if binding.admitted_event_id != input.event_id() {
            return Err(PrepareError::EventBindingMismatch);
        }
        if binding.admitted_input_digest != input.cid().digest() {
            return Err(PrepareError::InputDigestMismatch);
        }
        Ok(Self { binding, input })
    }

    pub fn binding(&self) -> InputBinding {
        self.binding
    }

    pub fn input(&self) -> &ActivationInput {
        &self.input
    }

    /// Check the packet against this immutable snapshot, bound ToolSet and raw caller caps.
    /// Caller history is materialization capacity, never a smaller Definition policy.
    /// Recheck source wire/embedded caps without another payload set or store reads.
    /// Events in retained data are rejected; pruned/uncommitted/global admission freshness
    /// still needs the host ledger. This does NOT invoke tools, authorize inference or
    /// enforce/consume episode/activation/unit/lifetime budgets.
    pub fn validate<'a>(
        &'a self,
        snapshot: &'a RestoredAgent,
        tools: &ToolSet<'_>,
        caller: &Limits,
    ) -> Result<ValidatedInput<'a>, PrepareError> {
        let state = snapshot.state();
        if self.binding.agent_id != state.agent_id() {
            return Err(PrepareError::WrongAgent);
        }
        if self.binding.execution_domain != snapshot.definition().execution_domain() {
            return Err(PrepareError::WrongDomain);
        }
        if self.binding.expected_root != snapshot.root() {
            return Err(PrepareError::WrongRoot);
        }
        if self.binding.expected_revision != state.revision() {
            return Err(PrepareError::WrongRevision);
        }
        if tools.definition_cid() != state.definition_link() {
            return Err(PrepareError::WrongDefinition);
        }
        check_snapshot_caps(snapshot, caller)?;
        check_input(snapshot, &self.input, caller)?;
        for (cid, _) in snapshot.blocks() {
            if snapshot
                .input(cid)
                .is_some_and(|input| input.event_id() == self.input.event_id())
            {
                return Err(PrepareError::KnownEventReplay);
            }
        }
        Ok(ValidatedInput {
            snapshot,
            packet: self,
            caller: *caller,
        })
    }
}

/// Borrowed checked data, not an execution permit. Captured caller limits are unchanged;
/// budgets/units/fresh host authorization and ready ModelRequest assembly remain later.
pub struct ValidatedInput<'a> {
    snapshot: &'a RestoredAgent,
    packet: &'a PreparedInput,
    caller: Limits,
}

impl<'a> ValidatedInput<'a> {
    pub fn snapshot(&self) -> &'a RestoredAgent {
        self.snapshot
    }

    pub fn input(&self) -> &'a ActivationInput {
        self.packet.input()
    }

    pub fn binding(&self) -> InputBinding {
        self.packet.binding()
    }

    pub fn caller_limits(&self) -> &Limits {
        &self.caller
    }
}

fn wire_limits(definition: &Limits, caller: &Limits) -> Limits {
    Limits {
        max_block_bytes: caller.max_block_bytes.min(definition.max_block_bytes),
        max_memory_bytes: caller.max_memory_bytes.min(definition.max_memory_bytes),
        max_command_bytes: caller.max_command_bytes.min(definition.max_command_bytes),
        max_observation_bytes: caller
            .max_observation_bytes
            .min(definition.max_observation_bytes),
        max_receipt_bytes: caller.max_receipt_bytes.min(definition.max_receipt_bytes),
        max_depth: caller.max_depth.min(definition.max_depth),
        max_collection_entries: caller
            .max_collection_entries
            .min(definition.max_collection_entries),
        ..*definition
    }
}

fn check_snapshot_caps(snapshot: &RestoredAgent, caller: &Limits) -> Result<(), PrepareError> {
    if snapshot.block_count() > caller.max_blocks {
        return Err(GraphError::BlockCountLimit.into());
    }
    if snapshot.encoded_bytes() > caller.max_materialized_bytes {
        return Err(GraphError::MaterializedByteLimit.into());
    }
    if snapshot.state().history_count() > caller.history_episodes {
        return Err(
            GraphError::HistoryConsistency(HistoryConsistencyError::CallerWindowLimit(
                snapshot.root(),
            ))
            .into(),
        );
    }
    let wire = wire_limits(snapshot.definition().limits(), caller);
    for (cid, bytes) in snapshot.blocks() {
        validate_drisl(bytes, &wire.decode_limits()).map_err(GraphError::from)?;
        if snapshot.memory_block(cid).is_some() && bytes.len() as u64 > wire.max_memory_bytes {
            return Err(GraphError::Codec(CodecError::ByteLimit).into());
        }
        if let Some(input) = snapshot.input(cid) {
            ActivationInput::validate_encoded(input.as_bytes(), &wire)
                .map_err(GraphError::Input)?;
        }
        if let Some(boundary) = snapshot.boundary(cid) {
            if let Exit::AwaitAction(command) = boundary.exit() {
                Command::validate_encoded(command, &wire)
                    .map_err(|error| GraphError::Boundary(BoundaryError::Command(error)))?;
            }
        }
    }
    Ok(())
}

/// Shared input-wire/phase pairing predicate for data projection and preparation.
/// It deliberately does not infer event admission or inspect pruned history, authenticate
/// a Receipt source, approve a grant, or consume counters. Rechecks caller/Definition
/// wire bounds and exact domain-bound pending Command identity without payload copying.
pub fn check_input(
    snapshot: &RestoredAgent,
    input: &ActivationInput,
    caller: &Limits,
) -> Result<(), InputCompatibilityError> {
    let definition = snapshot.definition();
    let wire = wire_limits(definition.limits(), caller);
    ActivationInput::validate_encoded(input.as_bytes(), &wire)
        .map_err(InputCompatibilityError::Input)?;
    match (snapshot.state().continuation(), input.cause()) {
        (Continuation::AwaitActor, Cause::ActorWake) => Ok(()),
        (Continuation::AwaitAction { .. }, Cause::ActionResult) => {
            let episode = snapshot
                .current_episode()
                .expect("restored current Episode");
            let boundary = snapshot
                .boundary(
                    episode
                        .boundaries()
                        .last()
                        .expect("restored nonempty Episode"),
                )
                .expect("restored Boundary");
            let Exit::AwaitAction(command) = boundary.exit() else {
                unreachable!("restored pending Command")
            };
            if Receipt::matches_in_validated(
                input.receipt_bytes().expect("ActionResult Receipt"),
                definition.execution_domain(),
                command,
            ) {
                Ok(())
            } else {
                Err(InputCompatibilityError::ReceiptMismatch)
            }
        }
        _ => Err(InputCompatibilityError::WrongPhase),
    }
}
