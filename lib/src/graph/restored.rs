//! Owns an immutable protocol-data snapshot, not runnable activation or trusted HEAD.

use super::{
    AgentState, Definition, GraphError, InspectedClosure, Limits, MaterializedBlocks, Memory,
};
use crate::{boundary::Boundary, episode::Episode, format::DaslCid, input::ActivationInput};

/// Complete bounded typed checkpoint data after all graph consistency checks.
/// Restoring data does NOT authorize Definition/code, facts/receipts/admissions or dispatch.
/// Application argument validation, units/lifetime/caller execution policy and fresh
/// prepared inputs remain runtime/host duties. No Activation or transcript is reconstructed.
#[derive(Debug, Eq, PartialEq)]
pub struct RestoredAgent {
    closure: InspectedClosure,
}

impl RestoredAgent {
    /// Restore exact materialized bytes with caller/Definition caps, never hydrate/fallback.
    /// Includes genesis and post-genesis data; no partial successful snapshot on any failure.
    ///
    /// ```
    /// use raiv::graph::{Genesis, MaterializedBlocks, RestoredAgent};
    /// use raiv::definition::{Definition, DefinitionSpec, MemoryProfile};
    /// use raiv::format::{DaslCid, DecodeLimits};
    /// use raiv::limits::Limits;
    /// struct Store(Genesis);
    /// impl MaterializedBlocks for Store {
    ///     fn get(&self, cid: DaslCid) -> Option<&[u8]> {
    ///         self.0.blocks().find(|(key, _)| *key == cid).map(|(_, bytes)| bytes)
    ///     }
    /// }
    /// let limits = Limits::default();
    /// let definition = Definition::encode(&DefinitionSpec {
    ///     execution_domain: [1; 32], policy: "fixture", mandate: "fixture",
    ///     memory_profile: MemoryProfile::None, tools: &[], inference_profile: &[0xa0],
    ///     limits,
    /// }, &DecodeLimits::default())?;
    /// let store = Store(Genesis::build([2; 32], definition, None, &limits)?);
    /// let snapshot = RestoredAgent::restore(store.0.root(), &store, &limits)?;
    /// assert_eq!(snapshot.state().revision(), 0);
    /// assert!(snapshot.current_episode().is_none());
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn restore(
        root: DaslCid,
        blocks: &impl MaterializedBlocks,
        limits: &Limits,
    ) -> Result<Self, GraphError> {
        Self::from_inspected(InspectedClosure::inspect(root, blocks, limits)?)
    }

    /// Move a previously inspected closure after validating all semantic data checks.
    /// No new store reads, re-encoding, or another owned payload set.
    pub fn from_inspected(closure: InspectedClosure) -> Result<Self, GraphError> {
        closure.check_tool_bindings()?;
        Ok(Self { closure })
    }

    pub fn root(&self) -> DaslCid {
        self.closure.root()
    }

    pub fn state(&self) -> &AgentState {
        self.closure.state()
    }

    pub fn definition(&self) -> &Definition {
        self.closure.definition()
    }

    pub fn memory(&self) -> Option<&Memory> {
        self.state()
            .memory_link()
            .map(|cid| self.closure.find_memory(cid).expect("restored root Memory"))
    }

    pub fn current_episode(&self) -> Option<&Episode> {
        self.state()
            .current_episode_link()
            .map(|cid| self.closure.episode(cid))
    }

    /// Completed Episodes in declared chronological order, borrowed from the retained closure.
    pub fn history(&self) -> impl Iterator<Item = &Episode> {
        self.state().history().map(|cid| self.closure.episode(cid))
    }

    /// Typed lookup in this exact required closure only; no missing-block hydration.
    pub fn boundary(&self, cid: DaslCid) -> Option<&Boundary> {
        self.closure.find_boundary(cid)
    }

    pub fn input(&self, cid: DaslCid) -> Option<&ActivationInput> {
        self.closure.find_input(cid)
    }

    pub fn memory_block(&self, cid: DaslCid) -> Option<&Memory> {
        self.closure.find_memory(cid)
    }

    pub fn block_count(&self) -> u64 {
        self.closure.block_count()
    }

    pub fn encoded_bytes(&self) -> u64 {
        self.closure.encoded_bytes()
    }

    /// Exact unique bytes; discovery order is not CAR export order.
    pub fn blocks(&self) -> impl Iterator<Item = (DaslCid, &[u8])> {
        self.closure.blocks()
    }
}
