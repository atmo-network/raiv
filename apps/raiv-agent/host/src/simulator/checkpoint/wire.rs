//! Closed private native array codec; complete nested-schema/join checks before payload copy.
use super::*;
use minicbor::{encode::Write, Decoder, Encoder};
use raiv::command::CommandView;
type Error = ExecutorCheckpointError;
struct Writer {
    bytes: Vec<u8>,
    max: u64,
}
impl Write for Writer {
    type Error = Error;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n as u64 > self.max)
        {
            return Err(Error::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| Error::Allocation)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
fn enc(e: minicbor::encode::Error<Error>) -> Error {
    e.into_write().unwrap_or(Error::Schema)
}
#[cfg(test)]
pub(super) fn encode(
    agent: Id32,
    program: ProgramBinding,
    rows: &[&Entry],
    limits: &ExecutorLimits,
) -> Result<Vec<u8>, Error> {
    encode_stage(agent, program, rows, limits, None)
}
#[cfg(test)]
pub(super) fn encode_stage(
    agent: Id32,
    program: ProgramBinding,
    rows: &[&Entry],
    limits: &ExecutorLimits,
    submitted: Option<Id32>,
) -> Result<Vec<u8>, Error> {
    encode_change(
        agent,
        program,
        rows,
        limits,
        submitted.map(Change::Submitted),
    )
}
pub(super) fn encode_change(
    agent: Id32,
    program: ProgramBinding,
    rows: &[&Entry],
    limits: &ExecutorLimits,
    change: Option<Change<'_>>,
) -> Result<Vec<u8>, Error> {
    let mut e = Encoder::new(Writer {
        bytes: Vec::new(),
        max: limits.wire.max_block_bytes,
    });
    e.array(5)
        .and_then(|e| e.str("raiv-host/executor-checkpoint"))
        .and_then(|e| e.u64(1))
        .and_then(|e| e.bytes(&agent))
        .and_then(|e| e.array(3))
        .and_then(|e| e.bytes(&program.module_digest))
        .and_then(|e| e.bytes(program.definition.as_bytes()))
        .and_then(|e| e.bytes(&program.execution_domain))
        .and_then(|e| e.array(rows.len() as u64))
        .map_err(enc)?;
    for row in rows {
        let bound = row.limits.encode(&limits.wire).map_err(Error::Limits)?;
        e.array(6)
            .and_then(|e| e.u64(row.anchor.revision))
            .and_then(|e| e.bytes(row.anchor.root.as_bytes()))
            .and_then(|e| e.bytes(row.command.as_bytes()))
            .and_then(|e| e.bytes(&bound))
            .and_then(|e| {
                e.u64(
                    if matches!(change,Some(Change::Submitted(id)) if id==row.command.action_id()) {
                        1
                    } else {
                        row.intents
                    },
                )
            })
            .map_err(enc)?;
        let receipt = match change {
            Some(Change::Terminal(id, receipt)) if id == row.command.action_id() => Some(receipt),
            _ => row.receipt.as_ref(),
        };
        match receipt {
            Some(r) => {
                e.bytes(r.as_bytes()).map_err(enc)?;
            }
            None => {
                e.null().map_err(enc)?;
            }
        }
    }
    Ok(e.into_writer().bytes)
}
fn array(d: &mut Decoder<'_>, n: u64) -> Result<(), Error> {
    if d.array().map_err(|_| Error::Schema)? != Some(n) {
        return Err(Error::Schema);
    }
    Ok(())
}
fn id(d: &mut Decoder<'_>) -> Result<Id32, Error> {
    d.bytes()
        .map_err(|_| Error::Schema)?
        .try_into()
        .map_err(|_| Error::Schema)
}
fn cid(d: &mut Decoder<'_>) -> Result<DaslCid, Error> {
    DaslCid::from_bytes(d.bytes().map_err(|_| Error::Schema)?)
        .and_then(DaslCid::require_state)
        .map_err(|_| Error::Schema)
}
fn number(d: &mut Decoder<'_>) -> Result<u64, Error> {
    d.u64().map_err(|_| Error::Schema)
}
fn span(d: &mut Decoder<'_>, frame: &[u8]) -> Result<Range<usize>, Error> {
    let bytes = d.bytes().map_err(|_| Error::Schema)?;
    let end = d.position();
    let start = end.checked_sub(bytes.len()).ok_or(Error::Schema)?;
    if end > frame.len() {
        return Err(Error::Schema);
    }
    Ok(start..end)
}
pub(super) fn inspect(
    bytes: &[u8],
    limits: &ExecutorLimits,
) -> Result<(Id32, ProgramBinding, Vec<Stored>), Error> {
    validate_drisl(bytes, &limits.wire).map_err(Error::Codec)?;
    let mut d = Decoder::new(bytes);
    array(&mut d, 5)?;
    if d.str().map_err(|_| Error::Schema)? != "raiv-host/executor-checkpoint"
        || number(&mut d)? != 1
    {
        return Err(Error::Schema);
    }
    let agent = id(&mut d)?;
    array(&mut d, 3)?;
    let program = ProgramBinding {
        module_digest: id(&mut d)?,
        definition: cid(&mut d)?,
        execution_domain: id(&mut d)?,
    };
    let n = d.array().map_err(|_| Error::Schema)?.ok_or(Error::Schema)?;
    if n > limits.max_actions {
        return Err(Error::Limit);
    }
    let n = usize::try_from(n).map_err(|_| Error::Limit)?;
    let mut rows: Vec<Stored> = Vec::new();
    rows.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
    let mut unresolved = 0;
    let mut max_revision = 0;
    for _ in 0..n {
        array(&mut d, 6)?;
        let revision = number(&mut d)?;
        if revision == 0 {
            return Err(Error::Schema);
        }
        let root = cid(&mut d)?;
        let command = span(&mut d, bytes)?;
        let saved = Limits::decode(d.bytes().map_err(|_| Error::Schema)?, &limits.wire)
            .map_err(Error::Limits)?;
        let intents = number(&mut d)?;
        if intents > 1 {
            return Err(Error::Schema);
        }
        let receipt = if d.datatype().map_err(|_| Error::Schema)? == minicbor::data::Type::Null {
            d.null().map_err(|_| Error::Schema)?;
            None
        } else {
            Some(span(&mut d, bytes)?)
        };
        let bound = Limits {
            max_block_bytes: saved.max_block_bytes.min(limits.wire.max_block_bytes),
            max_command_bytes: saved.max_command_bytes.min(limits.wire.max_block_bytes),
            max_receipt_bytes: saved.max_receipt_bytes.min(limits.wire.max_block_bytes),
            max_depth: saved.max_depth.min(limits.wire.max_depth),
            max_collection_entries: saved
                .max_collection_entries
                .min(limits.wire.max_collection_entries),
            ..saved
        };
        let cmd = CommandView::decode(&bytes[command.clone()], &bound)
            .map_err(|e| Error::Simulator(SimulatorError::Command(e)))?;
        let digest = raiv::identity::command_digest_view(program.execution_domain, cmd);
        let action_id = cmd.action_id();
        if rows.last().is_some_and(|r| r.record.action_id >= action_id)
            || rows.iter().any(|r| r.record.anchor.revision == revision)
        {
            return Err(Error::Schema);
        }
        if let Some(r) = &receipt {
            let view = ReceiptView::decode(&bytes[r.clone()], &bound)
                .map_err(|e| Error::Simulator(SimulatorError::Receipt(e)))?;
            if intents != 1 || view.action_id() != action_id || view.command_digest() != digest {
                return Err(Error::Binding);
            }
        } else {
            unresolved += 1;
            if unresolved > 1 {
                return Err(Error::Schema);
            }
        }
        max_revision = max_revision.max(revision);
        rows.push(Stored {
            record: ExecutionRecord {
                anchor: Anchor {
                    agent_id: agent,
                    revision,
                    root,
                },
                action_id,
                command_digest: digest,
                status: if receipt.is_some() {
                    Status::Terminal
                } else if intents == 1 {
                    Status::Submitted
                } else {
                    Status::Ready
                },
                submission_intents: intents,
            },
            limits: saved,
            command,
            receipt,
        });
    }
    if d.position() != bytes.len()
        || rows
            .iter()
            .any(|r| r.receipt.is_none() && r.record.anchor.revision != max_revision)
    {
        return Err(Error::Schema);
    }
    Ok((agent, program, rows))
}
