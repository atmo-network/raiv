//! Owns local live-VM routing, not registry approval, fresh admission or checkpoint acceptance.
use crate::{
    output::{Candidate, Output, Request},
    BridgeError, GuestVm, VmLimits, MAX_FRAME_BYTES,
};
use raiv::{
    activation::{ModelReply, ReplyError},
    format::{DaslCid, DaslCodec, Id32},
    graph::RestoredAgent,
    preparation::{InputBinding, PreparedInput},
};
use std::{fmt, sync::atomic::AtomicU64};
mod wire;
static NEXT_ORIGIN: AtomicU64 = AtomicU64::new(1);
/// Explicit integration-supplied program pin. Construction is NOT registry approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramBinding {
    pub module_digest: Id32,
    pub definition: DaslCid,
    pub execution_domain: Id32,
}
impl ProgramBinding {
    pub(crate) fn check_module(self, wasm: &[u8], source: &RestoredAgent) -> Result<(), RunError> {
        if wasm.len() > MAX_FRAME_BYTES {
            return Err(RunError::Frame);
        }
        if self.module_digest != DaslCid::for_block(DaslCodec::Raw, wasm).digest()
            || self.definition != source.state().definition_link()
            || self.execution_domain != source.definition().execution_domain()
        {
            return Err(RunError::ProgramBinding);
        }
        Ok(())
    }
}
/// Opaque safe-Rust correlation capability, never serialized/provider-controlled.
/// Uniqueness is process-local only; detached/restarted transports need their own protection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestTicket {
    origin: u64,
    request_id: u64,
}
impl RequestTicket {
    pub(crate) fn same_run(self, other: Self) -> bool {
        self.origin == other.origin
    }
    pub fn request_id(self) -> u64 {
        self.request_id
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Pending,
    Boundary,
    Failed,
}
#[derive(Debug)]
pub enum RunError {
    ProgramBinding,
    InputBinding,
    Correlation,
    Phase,
    Protocol,
    GuestFailure,
    OriginExhausted,
    Frame,
    Allocation,
    Reply(ReplyError),
    Bridge(BridgeError),
    Candidate(crate::candidate::VerificationError),
}
impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ProgramBinding => "module/Definition/domain does not match explicit program pin",
            Self::InputBinding => "input/source does not match captured binding",
            Self::Correlation => "response does not belong to this pending VM/run/request",
            Self::Phase => "run is terminal",
            Self::Protocol => "guest output contradicts live run protocol/binding",
            Self::GuestFailure => "guest reported activation failure",
            Self::OriginExhausted => "native process run identity exhausted",
            Self::Frame => "native input frame limit",
            Self::Allocation => "native input allocation failure",
            Self::Reply(e) => return e.fmt(f),
            Self::Bridge(e) => return e.fmt(f),
            Self::Candidate(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for RunError {}
/// The source/input and program pin remain immutable while the same VM owns all decisions.
/// Reference guest policy is currently scripted-only with default caller caps/one-unit quotes.
pub struct Run<'a> {
    vm: Option<GuestVm>,
    origin: u64,
    program: ProgramBinding,
    source: &'a RestoredAgent,
    packet: &'a PreparedInput,
    phase: Phase,
    pending: u64,
    output: Vec<u8>,
    candidate_snapshot: Option<RestoredAgent>,
}
/// Owning actual-VM output handoff. Private origin/binding and snapshot cannot be forged
/// by decoding candidate bytes. No borrowed source remains; no acceptance/dispatch grant.
pub struct SealedRun {
    pub(crate) origin: u64,
    pub(crate) program: ProgramBinding,
    pub(crate) binding: InputBinding,
    frame: Vec<u8>,
    pub(crate) snapshot: Option<RestoredAgent>,
}
impl SealedRun {
    pub fn candidate(&self) -> Candidate<'_> {
        let Output::Boundary(candidate) =
            Output::decode(&self.frame, &raiv::limits::Limits::default())
                .expect("sealed immutable frame")
        else {
            unreachable!("sealed boundary")
        };
        candidate
    }
    pub fn snapshot(&self) -> Option<&RestoredAgent> {
        self.snapshot.as_ref()
    }
}
impl<'a> Run<'a> {
    pub fn start(
        wasm: &[u8],
        program: ProgramBinding,
        source: &'a RestoredAgent,
        packet: &'a PreparedInput,
        limits: VmLimits,
    ) -> Result<Self, RunError> {
        check_binding(wasm, program, source, packet)?;
        let input = wire::activate(source, packet)?;
        let vm = GuestVm::new(wasm, limits).map_err(RunError::Bridge)?;
        let origin =
            crate::atomic::next_u64(&NEXT_ORIGIN).map_err(|_| RunError::OriginExhausted)?;
        let mut run = Self {
            vm: Some(vm),
            origin,
            program,
            source,
            packet,
            phase: Phase::Pending,
            pending: 0,
            output: Vec::new(),
            candidate_snapshot: None,
        };
        run.exchange(&input)?;
        Ok(run)
    }
    /// Move verified boundary data without keeping the source/registered anchor borrowed.
    pub fn into_sealed(self) -> Result<SealedRun, RunError> {
        if self.phase != Phase::Boundary || self.candidate_snapshot.is_none() {
            return Err(RunError::Phase);
        }
        Ok(SealedRun {
            origin: self.origin,
            program: self.program,
            binding: self.packet.binding(),
            frame: self.output,
            snapshot: self.candidate_snapshot,
        })
    }
    pub(crate) fn origin(&self) -> u64 {
        self.origin
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn program_binding(&self) -> ProgramBinding {
        self.program
    }
    pub fn input_binding(&self) -> InputBinding {
        self.packet.binding()
    }
    pub fn source(&self) -> &RestoredAgent {
        self.source
    }
    /// Borrow exact guest context/descriptors; never rebuild or select tools in the host.
    pub fn pending(&self) -> Option<(RequestTicket, Request<'_>)> {
        if self.phase != Phase::Pending {
            return None;
        }
        let Output::Infer {
            request_id,
            request,
        } = self.output().expect("retained pending frame")
        else {
            unreachable!("checked infer phase")
        };
        Some((
            RequestTicket {
                origin: self.origin,
                request_id,
            },
            request,
        ))
    }
    /// Sealed immutable candidate DATA may be retried by the later persistence owner.
    pub fn candidate(&self) -> Option<Candidate<'_>> {
        if self.phase != Phase::Boundary {
            return None;
        }
        let Output::Boundary(candidate) = self.output().expect("retained boundary frame") else {
            unreachable!("checked boundary phase")
        };
        Some(candidate)
    }
    /// Complete source-joined checkpoint DATA, still not accepted HEAD.
    pub fn candidate_snapshot(&self) -> Option<&RestoredAgent> {
        self.candidate_snapshot.as_ref()
    }
    pub fn output(&self) -> Option<Output<'_>> {
        if self.output.is_empty() {
            None
        } else {
            Some(
                Output::decode(&self.output, &raiv::limits::Limits::default())
                    .expect("retained validated output"),
            )
        }
    }
    pub fn reply(&mut self, ticket: RequestTicket, bytes: &[u8]) -> Result<(), RunError> {
        self.check_ticket(ticket)?;
        let mut caps = raiv::limits::Limits::default();
        let definition = self.source.definition().limits();
        caps.max_model_reply_bytes = caps
            .max_model_reply_bytes
            .min(definition.max_model_reply_bytes);
        caps.max_depth = caps.max_depth.min(definition.max_depth);
        caps.max_collection_entries = caps
            .max_collection_entries
            .min(definition.max_collection_entries);
        let reply = ModelReply::decode(bytes, &caps).map_err(RunError::Reply)?;
        if reply.request_id() != ticket.request_id {
            return Err(RunError::Correlation);
        }
        let input = wire::reply(ticket.request_id, bytes)?;
        self.exchange(&input)
    }
    pub fn inference_error(&mut self, ticket: RequestTicket, code: &str) -> Result<(), RunError> {
        self.check_ticket(ticket)?;
        if code.len() > 256 {
            return Err(RunError::Frame);
        }
        let input = wire::error_frame(ticket.request_id, code)?;
        self.exchange(&input)
    }
    fn check_ticket(&self, ticket: RequestTicket) -> Result<(), RunError> {
        if self.phase != Phase::Pending {
            return Err(RunError::Phase);
        }
        if ticket.origin != self.origin || ticket.request_id != self.pending {
            return Err(RunError::Correlation);
        }
        Ok(())
    }
    fn exchange(&mut self, input: &[u8]) -> Result<(), RunError> {
        let output = match self.vm.as_mut().expect("live VM").round_trip(input) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.fail();
                return Err(RunError::Bridge(error));
            }
        };
        let result = self.route(&output);
        match result {
            Ok(phase) => {
                self.phase = phase;
                self.output = output;
                Ok(())
            }
            Err(error) => {
                self.fail();
                self.output = output;
                Err(error)
            }
        }
    }
    fn fail(&mut self) {
        self.vm = None;
        self.phase = Phase::Failed;
        self.pending = 0;
        self.output.clear();
        self.candidate_snapshot = None;
    }
    fn route(&mut self, bytes: &[u8]) -> Result<Phase, RunError> {
        let output = Output::decode(bytes, &raiv::limits::Limits::default())
            .map_err(|_| RunError::Protocol)?;
        match output {
            Output::Infer {
                request_id,
                request,
            } => {
                // The reference guest increments exactly once on each inference yield.
                if Some(request_id) != self.pending.checked_add(1) {
                    return Err(RunError::Protocol);
                }
                let definition = self.source.definition();
                if request.model_parameters_bytes() != definition.inference_profile_bytes() {
                    return Err(RunError::ProgramBinding);
                }
                let mut registered = definition.tools();
                for enabled in request.tools() {
                    let expected = registered
                        .find(|t| t.id == enabled.id)
                        .ok_or(RunError::ProgramBinding)?;
                    if expected != enabled {
                        return Err(RunError::ProgramBinding);
                    }
                }
                self.pending = request_id;
                Ok(Phase::Pending)
            }
            Output::Boundary(candidate) => {
                if self.pending == 0 {
                    return Err(RunError::Protocol);
                }
                let snapshot = crate::candidate::restore(
                    candidate,
                    self.source,
                    self.packet,
                    &raiv::limits::Limits::default(),
                )
                .map_err(RunError::Candidate)?;
                self.candidate_snapshot = Some(snapshot);
                self.pending = 0;
                self.vm = None;
                Ok(Phase::Boundary)
            }
            Output::Error { .. } => Err(RunError::GuestFailure),
            Output::Initialized { .. } => Err(RunError::Protocol),
        }
    }
}
fn check_binding(
    wasm: &[u8],
    program: ProgramBinding,
    source: &RestoredAgent,
    packet: &PreparedInput,
) -> Result<(), RunError> {
    program.check_module(wasm, source)?;
    let b = packet.binding();
    if b.agent_id != source.state().agent_id()
        || b.execution_domain != program.execution_domain
        || b.expected_root != source.root()
        || b.expected_revision != source.state().revision()
    {
        return Err(RunError::InputBinding);
    }
    Ok(())
}
