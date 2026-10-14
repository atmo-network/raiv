//! Native process-local administrative registry. No external auth, durability or dispatch.
use crate::{
    run::{ProgramBinding, Run, RunError},
    BridgeError, GuestVm, VmLimits,
};
use raiv::{
    format::{DaslCid, Id32},
    graph::RestoredAgent,
    preparation::PreparedInput,
    state::Continuation,
};
use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_REGISTRY: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Anchor {
    pub agent_id: Id32,
    pub revision: u64,
    pub root: DaslCid,
}
#[derive(Debug)]
pub enum RegistryError {
    InvalidCapacity,
    OriginExhausted,
    Capacity,
    Allocation,
    AlreadyRegistered,
    NotGenesis,
    Program(RunError),
    Bridge(BridgeError),
}
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCapacity => "registry capacity must be positive",
            Self::OriginExhausted => "process registry identity exhausted",
            Self::Capacity => "registry capacity reached",
            Self::Allocation => "registry metadata allocation failure",
            Self::AlreadyRegistered => "agent already has a registered anchor",
            Self::NotGenesis => "registration requires complete revision-zero genesis data",
            Self::Program(e) => return e.fmt(f),
            Self::Bridge(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for RegistryError {}
/// Immutable module/Definition/domain approval and transaction-owned RAM anchor.
/// External callers must never receive the administrative registration port unguarded.
/// Actor/Receipt/grant freshness and acceptance are not supplied by this object.
pub struct RegisteredAgent {
    registry_origin: u64,
    program: ProgramBinding,
    snapshot: RestoredAgent,
    resources: VmLimits,
}
impl RegisteredAgent {
    pub(crate) fn registry_origin(&self) -> u64 {
        self.registry_origin
    }
    pub(crate) fn resources(&self) -> VmLimits {
        self.resources
    }
    pub fn anchor(&self) -> Anchor {
        Anchor {
            agent_id: self.snapshot.state().agent_id(),
            revision: self.snapshot.state().revision(),
            root: self.snapshot.root(),
        }
    }
    pub fn program_binding(&self) -> ProgramBinding {
        self.program
    }
    pub(crate) fn install_ram(&mut self, snapshot: RestoredAgent) {
        self.snapshot = snapshot;
    }
    pub fn snapshot(&self) -> &RestoredAgent {
        &self.snapshot
    }
    /// The registered program and source cannot be supplied/replaced by an input sender.
    /// This authenticates the program against local administrative policy ONLY; the packet
    /// still needs trusted admission/provenance before a live provider is attached.
    pub fn start_run<'a>(
        &'a self,
        wasm: &[u8],
        packet: &'a PreparedInput,
    ) -> Result<Run<'a>, RegistryError> {
        self.launch(wasm, packet).map_err(RegistryError::Program)
    }
    pub(crate) fn launch<'a>(
        &'a self,
        wasm: &[u8],
        packet: &'a PreparedInput,
    ) -> Result<Run<'a>, RunError> {
        Run::start(wasm, self.program, &self.snapshot, packet, self.resources)
    }
}
/// RAM-only native local-trusted administration. Possession of &mut Registry is the
/// administrative port, not a payload flag or a claim about an external principal.
/// Process loss loses the registry; do not use this as a crash-recovery manifest.
pub struct Registry {
    origin: u64,
    agents: Vec<RegisteredAgent>,
    capacity: usize,
}
impl Registry {
    pub fn new(capacity: usize) -> Result<Self, RegistryError> {
        if capacity == 0 {
            return Err(RegistryError::InvalidCapacity);
        }
        let origin = NEXT_REGISTRY
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| RegistryError::OriginExhausted)?;
        Ok(Self {
            origin,
            agents: Vec::new(),
            capacity,
        })
    }
    pub fn len(&self) -> usize {
        self.agents.len()
    }
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
    pub fn agent(&self, id: Id32) -> Option<&RegisteredAgent> {
        self.agents
            .iter()
            .find(|entry| entry.anchor().agent_id == id)
    }
    pub(crate) fn agent_mut(&mut self, id: Id32) -> Option<&mut RegisteredAgent> {
        self.agents
            .iter_mut()
            .find(|entry| entry.anchor().agent_id == id)
    }
    /// Explicit administrative approval. Never called by guest output parsing/import.
    /// Snapshot validity is established by Core restoration, not fabricated/default state.
    /// All checks/allocation precede assignment; existing records cannot be overwritten.
    pub fn register_genesis(
        &mut self,
        wasm: &[u8],
        program: ProgramBinding,
        snapshot: RestoredAgent,
        resources: VmLimits,
    ) -> Result<(), RegistryError> {
        if self.agent(snapshot.state().agent_id()).is_some() {
            return Err(RegistryError::AlreadyRegistered);
        }
        if self.agents.len() >= self.capacity {
            return Err(RegistryError::Capacity);
        }
        let state = snapshot.state();
        if state.revision() != 0
            || state.continuation() != Continuation::AwaitActor
            || state.current_episode_link().is_some()
            || state.history_count() != 0
        {
            return Err(RegistryError::NotGenesis);
        }
        let entry = self.stage_approved(wasm, program, snapshot, resources)?;
        self.publish_approved(entry);
        Ok(())
    }
    pub(crate) fn stage_approved(
        &mut self,
        wasm: &[u8],
        program: ProgramBinding,
        snapshot: RestoredAgent,
        resources: VmLimits,
    ) -> Result<RegisteredAgent, RegistryError> {
        if self.agent(snapshot.state().agent_id()).is_some() {
            return Err(RegistryError::AlreadyRegistered);
        }
        if self.agents.len() >= self.capacity {
            return Err(RegistryError::Capacity);
        }
        program
            .check_module(wasm, &snapshot)
            .map_err(RegistryError::Program)?;
        // Approval must at least admit the locked import-free Wasmi/typed-export/resource
        // contract. Drop this bootstrap instance; every run still gets its own fresh VM.
        GuestVm::new(wasm, resources).map_err(RegistryError::Bridge)?;
        self.agents
            .try_reserve(1)
            .map_err(|_| RegistryError::Allocation)?;
        Ok(RegisteredAgent {
            registry_origin: self.origin,
            program,
            snapshot,
            resources,
        })
    }
    pub(crate) fn publish_approved(&mut self, entry: RegisteredAgent) {
        self.agents.push(entry);
    }
}
