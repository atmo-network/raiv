//! Immutable candidate checkpoint data, not authoritative HEAD, placement or dispatch.

use crate::{
    boundary::Exit,
    format::{CodecError, DaslCid},
    graph::RestoredAgent,
    preparation::InputBinding,
    state::Continuation,
};
use alloc::vec::Vec;

/// Complete validated candidate closure plus source/event binding and exact block delta.
/// Creation does not accept the transition or authorize effects. The host must bind
/// authorized execution, place the required data, resolve CAS/event consumption and
/// accept this exact root before dispatching its embedded Command.
#[derive(Debug, Eq, PartialEq)]
pub struct CommitCandidate {
    binding: InputBinding,
    snapshot: RestoredAgent,
    new_cids: Vec<DaslCid>,
}
impl CommitCandidate {
    pub(crate) fn from_validated(
        binding: InputBinding,
        snapshot: RestoredAgent,
        source: &RestoredAgent,
    ) -> Result<Self, CodecError> {
        let mut new_cids = Vec::new();
        for (cid, _) in snapshot.blocks() {
            if !source.blocks().any(|(old, _)| old == cid) {
                new_cids
                    .try_reserve(1)
                    .map_err(|_| CodecError::AllocationFailure)?;
                new_cids.push(cid);
            }
        }
        new_cids.sort_unstable();
        Ok(Self {
            binding,
            snapshot,
            new_cids,
        })
    }
    pub fn binding(&self) -> InputBinding {
        self.binding
    }
    pub fn new_root(&self) -> DaslCid {
        self.snapshot.root()
    }
    pub fn disposition(&self) -> Continuation {
        self.snapshot.state().continuation()
    }
    /// Validated DATA for inspection/recovery, never evidence of host acceptance.
    pub fn snapshot(&self) -> &RestoredAgent {
        &self.snapshot
    }
    /// The whole exact required closure, not just blocks newly authored by this run.
    pub fn blocks(&self) -> impl Iterator<Item = (DaslCid, &[u8])> {
        self.snapshot.blocks()
    }
    /// Exact unique delta relative to the source closure, ordered by binary CID.
    /// Unchanged or previously retained Memory blocks are reused, not emitted twice.
    pub fn new_blocks(&self) -> impl ExactSizeIterator<Item = (DaslCid, &[u8])> {
        self.new_cids.iter().map(|cid| {
            (
                *cid,
                self.snapshot
                    .blocks()
                    .find(|(key, _)| key == cid)
                    .expect("validated candidate delta")
                    .1,
            )
        })
    }
    /// Same bytes embedded in the final Boundary, not another mutable dispatch payload.
    pub fn command_bytes(&self) -> Option<&[u8]> {
        let episode = self.snapshot.current_episode()?;
        let last = episode
            .boundaries()
            .last()
            .expect("validated nonempty Episode");
        match self
            .snapshot
            .boundary(last)
            .expect("validated Boundary")
            .exit()
        {
            Exit::AwaitAction(bytes) => Some(bytes),
            Exit::AwaitActor => None,
        }
    }
    /// Transfer checkpoint data only. The host must resolve acceptance before using
    /// this as canonical continuation; a Rust snapshot does not establish HEAD.
    pub fn into_snapshot(self) -> RestoredAgent {
        self.snapshot
    }
}
