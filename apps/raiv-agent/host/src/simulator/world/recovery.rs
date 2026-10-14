//! Explicit paired main/primary-world approval. No auto-submit, old capabilities,
//! pin selection/retention, Core-store-loss reconciliation or external world provenance.
use super::*;
use raiv::format::{DaslCid, DaslCodec};
/// Independently selected native metadata, NOT cryptographic approval or currentness.
#[derive(Clone, Copy, Debug)]
pub struct WorldRecoveryPin {
    pub source: LocalRecoveryPin,
    pub world_digest: Id32,
}
fn digest(bytes: &[u8]) -> Id32 {
    DaslCid::for_block(DaslCodec::Raw, bytes).digest()
}
impl<'a> ScriptedWorld<'a> {
    /// Export only already approved RAM/session and exact file/source agreement.
    /// This acknowledges DATA; it cannot choose independently approved current pins.
    pub fn current_pin(
        &mut self,
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        limits: &LocalExecutorLimits,
    ) -> Result<WorldRecoveryPin, WorldError> {
        self.check_scope(registered)?;
        let captured =
            ExecutorCheckpoint::capture(registered, admissions, &self.book, &limits.executor)
                .map_err(WorldError::Checkpoint)?;
        if captured.as_bytes() != self.state.ledger.as_bytes() {
            return Err(WorldError::Conflict);
        }
        self.state
            .ledger
            .preflight_restore(
                registered,
                admissions,
                &limits.caller,
                &limits.executor,
                self.book.capacity,
            )
            .map_err(WorldError::Checkpoint)?;
        let bytes = self.state.encode(&limits.executor)?;
        if WorldState::read(self.store, &limits.executor)?.encode(&limits.executor)? != bytes {
            return Err(WorldError::Conflict);
        }
        let source = current_pin(
            registered,
            admissions,
            agent_store,
            &limits.manifest,
            &limits.caller,
        )
        .map_err(|e| WorldError::Simulator(SimulatorError::Policy(e)))?;
        ManifestFile::world(self.store, limits.executor.wire)
            .replace(Some(&bytes), &bytes)
            .map_err(WorldError::Manifest)?;
        Ok(WorldRecoveryPin {
            source,
            world_digest: digest(&bytes),
        })
    }
    /// Construct a NEW native session from independently approved exact paired pins.
    /// Main Registry/AdmissionBook must already be independently restored/approved.
    /// No merge, balance reset, capability deserialization, ticket or auto-submit. All
    /// borrowed source/cap checks, owned staging and both acknowledgements precede return.
    pub fn adopt(
        registered: &RegisteredAgent,
        admissions: &AdmissionBook,
        agent_store: &mut DiskBlockStore,
        world_store: &'a mut DiskBlockStore,
        capacity: usize,
        pin: &WorldRecoveryPin,
        limits: &LocalExecutorLimits,
    ) -> Result<Self, WorldError> {
        if !pin.source.matches_registered(registered) {
            return Err(WorldError::Binding);
        }
        let state = WorldState::read(world_store, &limits.executor)?;
        let bytes = state.encode(&limits.executor)?;
        if digest(&bytes) != pin.world_digest {
            return Err(WorldError::Binding);
        }
        let mut book = ExecutorBook::new(capacity).map_err(WorldError::Simulator)?;
        let staged = state
            .ledger
            .stage_restore(
                registered,
                admissions,
                &limits.caller,
                &limits.executor,
                capacity,
            )
            .map_err(WorldError::Checkpoint)?;
        let source = current_pin(
            registered,
            admissions,
            agent_store,
            &limits.manifest,
            &limits.caller,
        )
        .map_err(|e| WorldError::Simulator(SimulatorError::Policy(e)))?;
        if source.manifest_digest != pin.source.manifest_digest {
            return Err(WorldError::Binding);
        }
        ManifestFile::world(world_store, limits.executor.wire)
            .replace(Some(&bytes), &bytes)
            .map_err(WorldError::Manifest)?;
        book.entries = staged;
        Ok(Self {
            store: world_store,
            state,
            book,
        })
    }
}
