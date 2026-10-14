//! Local administrative ActorWake/ActionResult admission, not executor authentication.
//! No effect ledger, lost-unit policy or CAS; durable composition has separate owners.
use crate::{
    registry::RegisteredAgent,
    run::{ProgramBinding, Run, RunError},
};
use raiv::{
    budget::BudgetAdmission,
    format::Id32,
    input::{ActivationInput, Cause},
    preparation::{InputBinding, PreparedInput},
    state::Continuation,
};
use std::{fmt, sync::atomic::AtomicU64};
static NEXT_BOOK: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionStatus {
    Reserved,
    Attempted,
    Launched,
    Consumed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionRecord {
    pub binding: InputBinding,
    pub grant: BudgetAdmission,
    pub status: AdmissionStatus,
    pub launch_attempts: u64,
    pub accepted_root: Option<raiv::format::DaslCid>,
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum TokenState {
    Issued,
    #[cfg(target_os = "linux")]
    Recovered,
}
struct Entry {
    token_state: TokenState,
    registry_origin: u64,
    record: AdmissionRecord,
    program: ProgramBinding,
    run_origin: Option<u64>,
}
#[cfg(target_os = "linux")]
pub(crate) struct RecoveredAdmissions(Vec<Entry>);
/// Private fields prevent a provider/input payload from claiming admission or mutating
/// observation/grant after reservation. Dropping it does NOT release its reserved IDs.
pub struct AdmittedInput {
    book: u64,
    packet: PreparedInput,
}
/// Crate-private linear staging; only the originating mutably held book may publish.
pub(crate) struct StagedReservation {
    entry: Entry,
    token: AdmittedInput,
    expected_len: usize,
}
impl AdmittedInput {
    pub fn packet(&self) -> &PreparedInput {
        &self.packet
    }
}
#[derive(Debug)]
pub enum AdmissionError {
    InvalidCapacity,
    OriginExhausted,
    Capacity,
    Allocation,
    WrongPhase,
    GrantMismatch,
    ReceiptMismatch,
    Replay,
    Busy,
    ForeignToken,
    WrongRegistration,
    AlreadyAttempted,
    TokenAlreadyIssued,
    Input(raiv::input::InputError),
    Run(RunError),
}
impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCapacity => "admission capacity must be positive",
            Self::OriginExhausted => "process admission identity exhausted",
            Self::Capacity => "admission book capacity reached",
            Self::Allocation => "admission metadata allocation failure",
            Self::WrongPhase => "input cause does not match registered continuation",
            Self::ReceiptMismatch => {
                "input Receipt differs from native approval or pending Command"
            }
            Self::GrantMismatch => "input grant differs from explicit native approval",
            Self::Replay => "event/admission ID already reserved for this agent",
            Self::Busy => "agent has unresolved admission or candidate acceptance",
            Self::ForeignToken => "token belongs to another admission book",
            Self::WrongRegistration => "admission token does not match registered program/anchor",
            Self::AlreadyAttempted => {
                "admission launch already attempted; explicit recovery is required"
            }
            Self::TokenAlreadyIssued => "reservation already has a process-local input capability",
            Self::Input(e) => return e.fmt(f),
            Self::Run(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for AdmissionError {}
/// Possession of this mutable book is the local native Actor/admission administrative
/// port. Do not expose it to guest/provider/public payloads. All records are volatile.
pub struct AdmissionBook {
    origin: u64,
    capacity: usize,
    entries: Vec<Entry>,
}
impl AdmissionBook {
    pub fn new(capacity: usize) -> Result<Self, AdmissionError> {
        if capacity == 0 {
            return Err(AdmissionError::InvalidCapacity);
        }
        let origin =
            crate::atomic::next_u64(&NEXT_BOOK).map_err(|_| AdmissionError::OriginExhausted)?;
        Ok(Self {
            origin,
            capacity,
            entries: Vec::new(),
        })
    }
    pub(crate) fn records_for(
        &self,
        registered: &RegisteredAgent,
    ) -> Result<impl Iterator<Item = AdmissionRecord> + '_, AdmissionError> {
        let id = registered.anchor().agent_id;
        if self.entries.iter().any(|e| {
            e.record.binding.agent_id == id
                && (e.registry_origin != registered.registry_origin()
                    || e.program != registered.program_binding())
        }) {
            return Err(AdmissionError::WrongRegistration);
        }
        Ok(self
            .entries
            .iter()
            .filter(move |e| e.record.binding.agent_id == id)
            .map(|e| e.record))
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn stage_recovered(
        &mut self,
        registered: &RegisteredAgent,
        records: &[AdmissionRecord],
    ) -> Result<RecoveredAdmissions, AdmissionError> {
        let id = registered.anchor().agent_id;
        if self.entries.iter().any(|e| e.record.binding.agent_id == id) {
            return Err(AdmissionError::Busy);
        }
        if self
            .entries
            .len()
            .checked_add(records.len())
            .is_none_or(|n| n > self.capacity)
        {
            return Err(AdmissionError::Capacity);
        }
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(records.len())
            .map_err(|_| AdmissionError::Allocation)?;
        for record in records {
            if record.binding.agent_id != id
                || record.binding.execution_domain != registered.program_binding().execution_domain
            {
                return Err(AdmissionError::WrongRegistration);
            }
            staged.push(Entry {
                token_state: TokenState::Recovered,
                registry_origin: registered.registry_origin(),
                record: *record,
                program: registered.program_binding(),
                run_origin: None,
            });
        }
        self.entries
            .try_reserve(records.len())
            .map_err(|_| AdmissionError::Allocation)?;
        Ok(RecoveredAdmissions(staged))
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn publish_recovered(&mut self, staged: RecoveredAdmissions) {
        self.entries.extend(staged.0);
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn record(&self, agent: Id32, event: Id32) -> Option<AdmissionRecord> {
        self.entries
            .iter()
            .find(|e| {
                e.record.binding.agent_id == agent && e.record.binding.admitted_event_id == event
            })
            .map(|e| e.record)
    }
    /// Explicit native approval of the exact observation snapshot and grant. The grant
    /// comes from native policy, not from model/provider arguments. No external Actor
    /// principal verification, authentic pricing or lifetime funding is inferred here.
    pub fn admit_actor_wake(
        &mut self,
        registered: &RegisteredAgent,
        input: ActivationInput,
        approved_grant: BudgetAdmission,
    ) -> Result<AdmittedInput, AdmissionError> {
        let source = registered.snapshot();
        if source.state().continuation() != Continuation::AwaitActor
            || input.cause() != Cause::ActorWake
        {
            return Err(AdmissionError::WrongPhase);
        }
        Self::validate_input(registered, &input, &raiv::limits::Limits::default())?;
        self.reserve(registered, input, approved_grant)
    }
    /// Explicit LOCAL administrative approval of this exact terminal Receipt and grant.
    /// Never call from a guest/provider payload. Receipt construction/matching is data,
    /// not evidence of execution/finality; executor/authenticated-ledger policy is separate.
    pub fn admit_action_result(
        &mut self,
        registered: &RegisteredAgent,
        input: ActivationInput,
        approved_receipt: &raiv::receipt::Receipt,
        approved_grant: BudgetAdmission,
    ) -> Result<AdmittedInput, AdmissionError> {
        let staged =
            self.stage_action_result(registered, input, approved_receipt, approved_grant)?;
        Ok(self.publish_reservation(staged))
    }
    pub(crate) fn stage_action_result(
        &mut self,
        registered: &RegisteredAgent,
        input: ActivationInput,
        approved_receipt: &raiv::receipt::Receipt,
        approved_grant: BudgetAdmission,
    ) -> Result<StagedReservation, AdmissionError> {
        if !matches!(
            registered.snapshot().state().continuation(),
            Continuation::AwaitAction { .. }
        ) || input.cause() != Cause::ActionResult
        {
            return Err(AdmissionError::WrongPhase);
        }
        if input.receipt_bytes() != Some(approved_receipt.as_bytes()) {
            return Err(AdmissionError::ReceiptMismatch);
        }
        Self::validate_input(registered, &input, &raiv::limits::Limits::default())?;
        self.stage_reservation(registered, input, approved_grant)
    }
    pub(crate) fn validate_input(
        registered: &RegisteredAgent,
        input: &ActivationInput,
        caller: &raiv::limits::Limits,
    ) -> Result<(), AdmissionError> {
        raiv::preparation::check_input(registered.snapshot(), input, caller).map_err(|e| match e {
            raiv::preparation::InputCompatibilityError::Input(e) => AdmissionError::Input(e),
            raiv::preparation::InputCompatibilityError::WrongPhase => AdmissionError::WrongPhase,
            raiv::preparation::InputCompatibilityError::ReceiptMismatch => {
                AdmissionError::ReceiptMismatch
            }
        })
    }
    fn reserve(
        &mut self,
        registered: &RegisteredAgent,
        input: ActivationInput,
        approved_grant: BudgetAdmission,
    ) -> Result<AdmittedInput, AdmissionError> {
        let staged = self.stage_reservation(registered, input, approved_grant)?;
        Ok(self.publish_reservation(staged))
    }
    fn stage_reservation(
        &mut self,
        registered: &RegisteredAgent,
        input: ActivationInput,
        approved_grant: BudgetAdmission,
    ) -> Result<StagedReservation, AdmissionError> {
        let source = registered.snapshot();
        let anchor = registered.anchor();
        if input.budget_admission() != approved_grant {
            return Err(AdmissionError::GrantMismatch);
        }
        if self.entries.iter().any(|e| {
            e.record.binding.agent_id == anchor.agent_id
                && (e.record.binding.admitted_event_id == input.event_id()
                    || e.record.grant.admission_id == approved_grant.admission_id)
        }) || source.blocks().any(|(cid, _)| {
            source
                .input(cid)
                .is_some_and(|i| i.event_id() == input.event_id())
        }) {
            return Err(AdmissionError::Replay);
        }
        if self.entries.iter().any(|e| {
            e.record.binding.agent_id == anchor.agent_id
                && e.record.status != AdmissionStatus::Consumed
        }) {
            return Err(AdmissionError::Busy);
        }
        if self.entries.len() >= self.capacity {
            return Err(AdmissionError::Capacity);
        }
        let binding = InputBinding {
            agent_id: anchor.agent_id,
            execution_domain: registered.program_binding().execution_domain,
            expected_root: anchor.root,
            expected_revision: anchor.revision,
            admitted_event_id: input.event_id(),
            admitted_input_digest: input.cid().digest(),
        };
        let packet =
            PreparedInput::new(binding, input).map_err(|_| AdmissionError::WrongRegistration)?;
        self.entries
            .try_reserve(1)
            .map_err(|_| AdmissionError::Allocation)?;
        Ok(StagedReservation {
            expected_len: self.entries.len(),
            entry: Entry {
                token_state: TokenState::Issued,
                registry_origin: registered.registry_origin(),
                record: AdmissionRecord {
                    binding,
                    grant: approved_grant,
                    status: AdmissionStatus::Reserved,
                    launch_attempts: 0,
                    accepted_root: None,
                },
                program: registered.program_binding(),
                run_origin: None,
            },
            token: AdmittedInput {
                book: self.origin,
                packet,
            },
        })
    }
    /// No allocation/check left after a composition root's final fsync. The mutable
    /// book stays exclusively held between staging and publication; no callbacks run.
    pub(crate) fn publish_reservation(&mut self, staged: StagedReservation) -> AdmittedInput {
        assert_eq!(
            staged.token.book, self.origin,
            "staged reservation belongs to another book"
        );
        assert_eq!(
            staged.expected_len,
            self.entries.len(),
            "book changed during reservation staging"
        );
        self.entries.push(staged.entry);
        staged.token
    }
    /// A reservation permits only one launch attempt. Failure remains unresolved and
    /// cannot refund/free/relaunch by dropping the token. Counts are native launch
    /// attempts, NOT inferred model calls, actual expense or lost execution units.
    pub fn start_run<'a>(
        &mut self,
        wasm: &[u8],
        registered: &'a RegisteredAgent,
        token: &'a AdmittedInput,
    ) -> Result<Run<'a>, AdmissionError> {
        let index = self.reserved_entry(registered, token)?;
        registered
            .program_binding()
            .check_module(wasm, registered.snapshot())
            .map_err(AdmissionError::Run)?;
        self.mark_attempt(index);
        match registered.launch(wasm, token.packet()) {
            Ok(run) => {
                self.mark_launched(index, run.origin());
                Ok(run)
            }
            Err(e) => Err(AdmissionError::Run(e)),
        }
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn recoverable_reserved(
        &self,
        registered: &RegisteredAgent,
        event: Id32,
    ) -> Result<(usize, AdmissionRecord), AdmissionError> {
        let index = self
            .entries
            .iter()
            .position(|e| {
                e.record.binding.agent_id == registered.anchor().agent_id
                    && e.record.binding.admitted_event_id == event
            })
            .ok_or(AdmissionError::WrongRegistration)?;
        let e = &self.entries[index];
        let b = e.record.binding;
        let anchor = registered.anchor();
        if e.registry_origin != registered.registry_origin()
            || e.program != registered.program_binding()
            || b.expected_root != anchor.root
            || b.expected_revision != anchor.revision
        {
            return Err(AdmissionError::WrongRegistration);
        }
        if e.record.status != AdmissionStatus::Reserved
            || e.record.launch_attempts != 0
            || e.run_origin.is_some()
        {
            return Err(AdmissionError::AlreadyAttempted);
        }
        if e.token_state != TokenState::Recovered {
            return Err(AdmissionError::TokenAlreadyIssued);
        }
        Ok((index, e.record))
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn issue_recovered(&mut self, index: usize, packet: PreparedInput) -> AdmittedInput {
        self.entries[index].token_state = TokenState::Issued;
        AdmittedInput {
            book: self.origin,
            packet,
        }
    }
    pub(crate) fn reserved_entry(
        &self,
        registered: &RegisteredAgent,
        token: &AdmittedInput,
    ) -> Result<usize, AdmissionError> {
        let index = self.check_token(registered, token)?;
        if self.entries[index].record.status != AdmissionStatus::Reserved {
            return Err(AdmissionError::AlreadyAttempted);
        }
        Ok(index)
    }
    pub(crate) fn mark_attempt(&mut self, index: usize) {
        self.entries[index].record.status = AdmissionStatus::Attempted;
        self.entries[index].record.launch_attempts = 1;
    }
    pub(crate) fn mark_launched(&mut self, index: usize, origin: u64) {
        self.entries[index].run_origin = Some(origin);
        self.entries[index].record.status = AdmissionStatus::Launched;
    }
    /// Correlate only the run launched by this book, not a separately computed candidate.
    /// This is NOT event consumption or checkpoint/effect acceptance.
    pub fn matches_run(
        &self,
        registered: &RegisteredAgent,
        token: &AdmittedInput,
        run: &Run<'_>,
    ) -> bool {
        self.check_token(registered, token).is_ok_and(|index| {
            self.entries[index].run_origin == Some(run.origin())
                && run.input_binding() == token.packet.binding()
                && run.program_binding() == registered.program_binding()
        })
    }
    pub(crate) fn sealed_entry(
        &self,
        token: &AdmittedInput,
        sealed: &crate::run::SealedRun,
    ) -> Result<(usize, AdmissionRecord, u64), AdmissionError> {
        if token.book != self.origin {
            return Err(AdmissionError::ForeignToken);
        }
        let index = self
            .entries
            .iter()
            .position(|e| e.record.binding == token.packet.binding())
            .ok_or(AdmissionError::ForeignToken)?;
        let entry = &self.entries[index];
        if entry.run_origin != Some(sealed.origin)
            || entry.program != sealed.program
            || entry.record.binding != sealed.binding
        {
            return Err(AdmissionError::WrongRegistration);
        }
        Ok((index, entry.record, entry.registry_origin))
    }
    pub(crate) fn consume_ram(&mut self, index: usize, root: raiv::format::DaslCid) {
        self.entries[index].record.accepted_root = Some(root);
        self.entries[index].record.status = AdmissionStatus::Consumed;
    }
    fn check_token(
        &self,
        registered: &RegisteredAgent,
        token: &AdmittedInput,
    ) -> Result<usize, AdmissionError> {
        if token.book != self.origin {
            return Err(AdmissionError::ForeignToken);
        }
        let binding = token.packet.binding();
        let anchor = registered.anchor();
        let index = self
            .entries
            .iter()
            .position(|e| e.record.binding == binding)
            .ok_or(AdmissionError::ForeignToken)?;
        if self.entries[index].registry_origin != registered.registry_origin()
            || binding.agent_id != anchor.agent_id
            || binding.expected_root != anchor.root
            || binding.expected_revision != anchor.revision
            || binding.execution_domain != registered.program_binding().execution_domain
            || self.entries[index].program != registered.program_binding()
        {
            return Err(AdmissionError::WrongRegistration);
        }
        Ok(index)
    }
}
