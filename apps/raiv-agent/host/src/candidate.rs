//! Source-relative candidate DATA validation; no module approval, CAS or dispatch permission.
use crate::output::{Blocks, Candidate};
use raiv::{
    format::DaslCid,
    graph::{GraphError, MaterializedBlocks, RestoredAgent},
    limits::Limits,
    preparation::PreparedInput,
    state::Continuation,
};
use std::fmt;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationError {
    Binding,
    Transition,
    Delta,
    Graph(GraphError),
}
impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Binding => "candidate/source/input binding mismatch",
            Self::Transition => "candidate is not the single next checkpoint transition",
            Self::Delta => "candidate delta is not exactly source-relative required data",
            Self::Graph(e) => return e.fmt(f),
        })
    }
}
impl std::error::Error for VerificationError {}
/// Restore the complete required closure, then join it to the immutable source and exact
/// packet. Successful return remains protocol DATA, not authoritative HEAD or execution.
/// Caller bounds apply to the resulting closure; the source was independently restored.
pub fn restore(
    candidate: Candidate<'_>,
    source: &RestoredAgent,
    packet: &PreparedInput,
    caller: &Limits,
) -> Result<RestoredAgent, VerificationError> {
    let b = packet.binding();
    let old = source.state();
    if b.agent_id != old.agent_id()
        || b.execution_domain != source.definition().execution_domain()
        || b.expected_root != source.root()
        || b.expected_revision != old.revision()
        || candidate.agent_id() != b.agent_id
        || candidate.expected_root() != b.expected_root
        || candidate.expected_revision() != b.expected_revision
        || candidate.admitted_event_id() != b.admitted_event_id
        || candidate.admitted_input_digest() != b.admitted_input_digest
    {
        return Err(VerificationError::Binding);
    }
    if source.blocks().any(|(cid, _)| {
        source
            .input(cid)
            .is_some_and(|input| input.event_id() == b.admitted_event_id)
    }) {
        return Err(VerificationError::Binding);
    }
    let delta = candidate.new_blocks();
    if delta
        .iter()
        .any(|block| source.blocks().any(|(cid, _)| cid == block.cid))
    {
        return Err(VerificationError::Delta);
    }
    let next = RestoredAgent::restore(candidate.new_root(), &Overlay { source, delta }, caller)
        .map_err(VerificationError::Graph)?;
    let state = next.state();
    if Some(state.revision()) != old.revision().checked_add(1)
        || state.agent_id() != old.agent_id()
        || state.definition_link() != old.definition_link()
        || state.continuation() != candidate.disposition()
    {
        return Err(VerificationError::Transition);
    }
    // Restoration follows only required links. Reject otherwise hash-valid staging extras.
    if delta
        .iter()
        .any(|block| !next.blocks().any(|(cid, _)| cid == block.cid))
    {
        return Err(VerificationError::Delta);
    }
    let closed = matches!(state.continuation(), Continuation::AwaitActor);
    let episode = if closed {
        next.history().last()
    } else {
        next.current_episode()
    }
    .ok_or(VerificationError::Transition)?;
    let count = source.current_episode().map_or(0, |e| e.boundary_count());
    if Some(episode.boundary_count()) != count.checked_add(1) {
        return Err(VerificationError::Transition);
    }
    if let Some(previous) = source.current_episode() {
        if episode.id() != previous.id()
            || episode.trigger_event_id() != previous.trigger_event_id()
            || episode.entry_memory_link() != previous.entry_memory_link()
            || !episode
                .boundaries()
                .take(usize::try_from(count).map_err(|_| VerificationError::Transition)?)
                .eq(previous.boundaries())
        {
            return Err(VerificationError::Transition);
        }
    } else if episode.entry_memory_link() != old.memory_link()
        || episode.trigger_event_id() != packet.input().event_id()
    {
        return Err(VerificationError::Transition);
    }
    let last = episode
        .boundaries()
        .last()
        .ok_or(VerificationError::Transition)?;
    let boundary = next.boundary(last).ok_or(VerificationError::Transition)?;
    if boundary.ordinal() != count
        || boundary.input_link() != packet.input().cid()
        || next
            .input(boundary.input_link())
            .is_none_or(|input| input.as_bytes() != packet.input().as_bytes())
    {
        return Err(VerificationError::Transition);
    }
    let total = old
        .history_count()
        .checked_add(u64::from(closed))
        .ok_or(VerificationError::Transition)?;
    let retained = total.min(source.definition().limits().history_episodes.max(1));
    // No rewritten history, alternate branches or extra boundaries hidden behind a valid root.
    let expected = old.history().chain(closed.then_some(episode.cid()));
    let drop = usize::try_from(total - retained).map_err(|_| VerificationError::Transition)?;
    if !state.history().eq(expected.skip(drop)) {
        return Err(VerificationError::Transition);
    }
    Ok(next)
}
struct Overlay<'a> {
    source: &'a RestoredAgent,
    delta: Blocks<'a>,
}
impl MaterializedBlocks for Overlay<'_> {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.delta.get(cid).or_else(|| {
            self.source
                .blocks()
                .find(|(key, _)| *key == cid)
                .map(|(_, body)| body)
        })
    }
}
