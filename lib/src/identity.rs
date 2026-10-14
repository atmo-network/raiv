//! Owns the pinned domain-separated episode/action/command identities, not registration,
//! provider IDs, transaction nonces, canonical HEAD, or execution authorization.

use crate::command::Command;
use crate::format::{domain_hash, Id32};

/// SHA256(DRISL(["raiv/episode/1", execution_domain, agent_id, trigger_event_id])).
pub fn episode_id(execution_domain: Id32, agent_id: Id32, trigger_event_id: Id32) -> Id32 {
    domain_hash(
        "raiv/episode/1",
        &[execution_domain, agent_id, trigger_event_id],
        None,
        None,
    )
    .expect("fixed Raiv episode prefix fits 128 bytes")
}

/// SHA256(DRISL(["raiv/action/1", execution_domain, agent_id, episode_id, action_index])).
/// This identifies a logical slot, not an external transaction or redelivery attempt.
pub fn action_id(
    execution_domain: Id32,
    agent_id: Id32,
    episode_id: Id32,
    action_index: u64,
) -> Id32 {
    domain_hash(
        "raiv/action/1",
        &[execution_domain, agent_id, episode_id],
        Some(action_index),
        None,
    )
    .expect("fixed Raiv action prefix fits 128 bytes")
}

/// SHA256(DRISL(["raiv/command/1", execution_domain, Command])). The validated command
/// is appended exactly as encoded, with no re-encoding, new state block, or hidden I/O.
pub fn command_digest(execution_domain: Id32, command: &Command) -> Id32 {
    command_digest_in_validated(execution_domain, command.as_bytes())
}

/// Same fixed command domain over a validated borrowed Command view; no payload copy,
/// re-encoding, registry approval or execution authority is supplied by hashing it.
pub fn command_digest_view(
    execution_domain: Id32,
    command: crate::command::CommandView<'_>,
) -> Id32 {
    command_digest_in_validated(execution_domain, command.as_bytes())
}

/// Internal borrowed form; callers retain immutable schema-validated Command bytes.
pub(crate) fn command_digest_in_validated(execution_domain: Id32, command: &[u8]) -> Id32 {
    domain_hash("raiv/command/1", &[execution_domain], None, Some(command))
        .expect("fixed Raiv command prefix fits 128 bytes")
}

#[cfg(test)]
mod tests;
