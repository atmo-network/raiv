//! Typed local checkpoint MANIFEST DATA. No authority, token recreation or dispatch.

use crate::{
    admission::{AdmissionBook, AdmissionRecord, AdmissionStatus},
    registry::{Anchor, RegisteredAgent},
    run::ProgramBinding,
    VmLimits,
};
use raiv::{
    format::{validate_drisl, CodecError, DaslCid, DecodeLimits},
    graph::RestoredAgent,
};
use std::fmt;

#[cfg(test)]
mod tests;
mod wire;

#[derive(Clone, Copy, Debug)]
pub struct ManifestLimits {
    pub wire: DecodeLimits,
    pub max_blocks: u64,
    pub max_admissions: u64,
}

#[derive(Debug)]
pub enum CheckpointError {
    Codec(CodecError),
    Schema,
    Limit,
    Allocation,
    Binding,
    Admission(crate::admission::AdmissionError),
    Graph(raiv::graph::GraphError),
    #[cfg(target_os = "linux")]
    Disk(crate::disk::DiskError),
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Codec(e) => return e.fmt(f),
            Self::Schema => "invalid local checkpoint manifest schema",
            Self::Limit => "checkpoint manifest capacity exceeded",
            Self::Allocation => "checkpoint manifest allocation failure",
            Self::Binding => "checkpoint manifest/snapshot bindings disagree",
            Self::Admission(e) => return e.fmt(f),
            Self::Graph(e) => return e.fmt(f),
            #[cfg(target_os = "linux")]
            Self::Disk(e) => return e.fmt(f),
        })
    }
}

impl std::error::Error for CheckpointError {}

/// One local agent's exact metadata. The program/resource/policy/grant fields remain
/// DATA on decoding; importing a file cannot approve them or restore native capabilities.
/// No observations/context/replies/transcripts or core block bodies are duplicated here.
pub struct CheckpointManifest {
    anchor: Anchor,
    program: ProgramBinding,
    resources: VmLimits,
    required: Vec<DaslCid>,
    admissions: Vec<AdmissionRecord>,
}

impl CheckpointManifest {
    pub fn capture(
        registered: &RegisteredAgent,
        book: &AdmissionBook,
        limits: &ManifestLimits,
    ) -> Result<Self, CheckpointError> {
        let required = Self::index(registered.snapshot(), limits)?;
        let mut admissions = Vec::new();
        for record in book
            .records_for(registered)
            .map_err(CheckpointError::Admission)?
        {
            if admissions.len() as u64 >= limits.max_admissions {
                return Err(CheckpointError::Limit);
            }
            admissions
                .try_reserve(1)
                .map_err(|_| CheckpointError::Allocation)?;
            admissions.push(record);
        }
        admissions.sort_unstable_by_key(|r| r.binding.admitted_event_id);
        let data = Self {
            anchor: registered.anchor(),
            program: registered.program_binding(),
            resources: registered.resources(),
            required,
            admissions,
        };
        data.check(limits)?;
        data.check_snapshot(registered.snapshot())?;
        Ok(data)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn next(
        &self,
        snapshot: &RestoredAgent,
        binding: raiv::preparation::InputBinding,
        limits: &ManifestLimits,
    ) -> Result<Self, CheckpointError> {
        if snapshot.state().agent_id() != self.anchor.agent_id
            || snapshot.state().revision()
                != self
                    .anchor
                    .revision
                    .checked_add(1)
                    .ok_or(CheckpointError::Binding)?
        {
            return Err(CheckpointError::Binding);
        }
        let mut admissions = Vec::new();
        admissions
            .try_reserve_exact(self.admissions.len())
            .map_err(|_| CheckpointError::Allocation)?;
        admissions.extend_from_slice(&self.admissions);
        let index = admissions
            .binary_search_by_key(&binding.admitted_event_id, |r| r.binding.admitted_event_id)
            .map_err(|_| CheckpointError::Binding)?;
        if admissions[index].binding != binding
            || admissions[index].status != AdmissionStatus::Launched
        {
            return Err(CheckpointError::Binding);
        }
        admissions[index].status = AdmissionStatus::Consumed;
        admissions[index].accepted_root = Some(snapshot.root());
        let required = Self::index(snapshot, limits)?;
        let data = Self {
            anchor: Anchor {
                agent_id: self.anchor.agent_id,
                revision: snapshot.state().revision(),
                root: snapshot.root(),
            },
            program: self.program,
            resources: self.resources,
            required,
            admissions,
        };
        data.check(limits)?;
        data.check_snapshot(snapshot)?;
        Ok(data)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn admission_stage(
        &self,
        binding: raiv::preparation::InputBinding,
        target: Option<AdmissionStatus>,
        limits: &ManifestLimits,
    ) -> Result<Self, CheckpointError> {
        let mut required = Vec::new();
        required
            .try_reserve_exact(self.required.len())
            .map_err(|_| CheckpointError::Allocation)?;
        required.extend_from_slice(&self.required);
        let mut admissions = Vec::new();
        admissions
            .try_reserve_exact(self.admissions.len())
            .map_err(|_| CheckpointError::Allocation)?;
        admissions.extend_from_slice(&self.admissions);
        let index = admissions
            .binary_search_by_key(&binding.admitted_event_id, |r| r.binding.admitted_event_id)
            .map_err(|_| CheckpointError::Binding)?;
        let record = &mut admissions[index];
        if record.binding != binding {
            return Err(CheckpointError::Binding);
        }
        match (record.status, target) {
            (AdmissionStatus::Reserved, None) => {
                admissions.remove(index);
            }
            (AdmissionStatus::Reserved, Some(AdmissionStatus::Attempted)) => {
                record.status = AdmissionStatus::Attempted;
                record.launch_attempts = 1;
            }
            (AdmissionStatus::Attempted, Some(AdmissionStatus::Launched)) => {
                record.status = AdmissionStatus::Launched;
            }
            _ => return Err(CheckpointError::Binding),
        }
        let data = Self {
            anchor: self.anchor,
            program: self.program,
            resources: self.resources,
            required,
            admissions,
        };
        data.check(limits)?;
        Ok(data)
    }

    fn index(
        snapshot: &RestoredAgent,
        limits: &ManifestLimits,
    ) -> Result<Vec<DaslCid>, CheckpointError> {
        let mut required = Vec::new();
        for (cid, _) in snapshot.blocks() {
            if required.len() as u64 >= limits.max_blocks {
                return Err(CheckpointError::Limit);
            }
            required
                .try_reserve(1)
                .map_err(|_| CheckpointError::Allocation)?;
            required.push(cid);
        }
        required.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Ok(required)
    }

    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    pub fn program_binding(&self) -> ProgramBinding {
        self.program
    }

    pub fn resources(&self) -> VmLimits {
        self.resources
    }

    pub fn required_cids(&self) -> &[DaslCid] {
        &self.required
    }

    pub fn admissions(&self) -> &[AdmissionRecord] {
        &self.admissions
    }

    pub fn encode(&self, limits: &ManifestLimits) -> Result<Vec<u8>, CheckpointError> {
        self.check(limits)?;
        wire::encode(self, limits)
    }

    pub fn decode(bytes: &[u8], limits: &ManifestLimits) -> Result<Self, CheckpointError> {
        validate_drisl(bytes, &limits.wire).map_err(CheckpointError::Codec)?;
        let data = wire::decode(bytes, limits)?;
        data.check(limits)?;
        Ok(data)
    }

    fn check(&self, limits: &ManifestLimits) -> Result<(), CheckpointError> {
        if self.required.len() as u64 > limits.max_blocks
            || self.admissions.len() as u64 > limits.max_admissions
        {
            return Err(CheckpointError::Limit);
        }
        self.anchor
            .root
            .require_state()
            .map_err(|_| CheckpointError::Schema)?;
        self.program
            .definition
            .require_state()
            .map_err(|_| CheckpointError::Schema)?;
        self.resources
            .validate()
            .map_err(|_| CheckpointError::Schema)?;
        if self.required.is_empty()
            || !self.required.contains(&self.anchor.root)
            || self
                .required
                .windows(2)
                .any(|w| w[0].as_bytes() >= w[1].as_bytes())
        {
            return Err(CheckpointError::Schema);
        }
        if self
            .admissions
            .windows(2)
            .any(|w| w[0].binding.admitted_event_id >= w[1].binding.admitted_event_id)
        {
            return Err(CheckpointError::Schema);
        }
        let mut unresolved = 0;
        let mut latest = 0;
        for (i, r) in self.admissions.iter().enumerate() {
            r.binding
                .expected_root
                .require_state()
                .map_err(|_| CheckpointError::Schema)?;
            if r.binding.agent_id != self.anchor.agent_id
                || r.binding.execution_domain != self.program.execution_domain
                || self.admissions[..i]
                    .iter()
                    .any(|old| old.grant.admission_id == r.grant.admission_id)
            {
                return Err(CheckpointError::Binding);
            }
            if r.launch_attempts
                != if r.status == AdmissionStatus::Reserved {
                    0
                } else {
                    1
                }
            {
                return Err(CheckpointError::Schema);
            }
            if r.status == AdmissionStatus::Consumed {
                let root = r.accepted_root.ok_or(CheckpointError::Schema)?;
                root.require_state().map_err(|_| CheckpointError::Schema)?;
                let revision = r
                    .binding
                    .expected_revision
                    .checked_add(1)
                    .ok_or(CheckpointError::Schema)?;
                if revision > self.anchor.revision
                    || (revision == self.anchor.revision && root != self.anchor.root)
                    || self.admissions[..i].iter().any(|old| {
                        old.status == AdmissionStatus::Consumed
                            && old.binding.expected_revision == r.binding.expected_revision
                    })
                {
                    return Err(CheckpointError::Binding);
                }
                if revision == self.anchor.revision {
                    latest += 1;
                }
            } else {
                unresolved += 1;
                if unresolved > 1
                    || r.accepted_root.is_some()
                    || r.binding.expected_root != self.anchor.root
                    || r.binding.expected_revision != self.anchor.revision
                {
                    return Err(CheckpointError::Binding);
                }
            }
        }
        if self.anchor.revision > 0 && latest != 1 {
            return Err(CheckpointError::Binding);
        }
        Ok(())
    }

    fn check_snapshot(&self, snapshot: &RestoredAgent) -> Result<(), CheckpointError> {
        if snapshot.root() != self.anchor.root
            || snapshot.state().agent_id() != self.anchor.agent_id
            || snapshot.state().revision() != self.anchor.revision
            || snapshot.state().definition_link() != self.program.definition
            || snapshot.definition().execution_domain() != self.program.execution_domain
            || snapshot.block_count() != self.required.len() as u64
            || snapshot.blocks().any(|(cid, _)| {
                self.required
                    .binary_search_by(|c| c.as_bytes().cmp(cid.as_bytes()))
                    .is_err()
            })
        {
            return Err(CheckpointError::Binding);
        }
        if self.anchor.revision > 0 {
            let episode = snapshot
                .current_episode()
                .or_else(|| snapshot.history().last())
                .ok_or(CheckpointError::Binding)?;
            let last = episode
                .boundaries()
                .last()
                .and_then(|cid| snapshot.boundary(cid))
                .ok_or(CheckpointError::Binding)?;
            let input = snapshot
                .input(last.input_link())
                .ok_or(CheckpointError::Binding)?;
            let record = self
                .admissions
                .iter()
                .find(|r| {
                    r.status == AdmissionStatus::Consumed
                        && r.binding.expected_revision.checked_add(1) == Some(self.anchor.revision)
                })
                .ok_or(CheckpointError::Binding)?;
            if record.binding.admitted_event_id != input.event_id()
                || record.binding.admitted_input_digest != input.cid().digest()
                || record.grant != input.budget_admission()
            {
                return Err(CheckpointError::Binding);
            }
        }
        for input in snapshot.blocks().filter_map(|(cid, _)| snapshot.input(cid)) {
            let index = self
                .admissions
                .binary_search_by_key(&input.event_id(), |r| r.binding.admitted_event_id)
                .map_err(|_| CheckpointError::Binding)?;
            let r = &self.admissions[index];
            if r.status != AdmissionStatus::Consumed
                || input.cid().digest() != r.binding.admitted_input_digest
                || input.budget_admission() != r.grant
            {
                return Err(CheckpointError::Binding);
            }
        }
        Ok(())
    }

    /// Explicit indexed I/O followed by pure Core restoration. No approval/HEAD install.
    #[cfg(target_os = "linux")]
    pub fn restore(
        &self,
        disk: &crate::disk::DiskBlockStore,
        caller: &raiv::limits::Limits,
    ) -> Result<RestoredAgent, CheckpointError> {
        if self.required.len() as u64 > caller.max_blocks {
            return Err(CheckpointError::Limit);
        }
        let mut ram = crate::store::MemoryBlockStore::new(crate::store::StoreLimits {
            max_blocks: caller.max_blocks,
            max_bytes: caller.max_materialized_bytes,
            wire: caller.decode_limits(),
        });
        for cid in &self.required {
            let body = disk
                .load_capped(*cid, &caller.decode_limits())
                .map_err(CheckpointError::Disk)?;
            ram.put(*cid, &body)
                .map_err(|e| CheckpointError::Disk(crate::disk::DiskError::Data(e)))?;
        }
        let snapshot = RestoredAgent::restore(self.anchor.root, &ram, caller)
            .map_err(CheckpointError::Graph)?;
        self.check_snapshot(&snapshot)?;
        Ok(snapshot)
    }
}
