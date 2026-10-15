//! Private closed positional metadata codec; no dynamic Value or model encoding.

use super::*;
use minicbor::{encode::Write, Decoder, Encoder};
use raiv::{budget::BudgetAdmission, preparation::InputBinding};

struct Writer {
    bytes: Vec<u8>,
    max: u64,
}

impl Write for Writer {
    type Error = CheckpointError;

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n as u64 > self.max)
        {
            return Err(CheckpointError::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| CheckpointError::Allocation)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

fn err(e: minicbor::encode::Error<CheckpointError>) -> CheckpointError {
    e.into_write().unwrap_or(CheckpointError::Schema)
}

pub(super) fn encode(
    data: &CheckpointManifest,
    limits: &ManifestLimits,
) -> Result<Vec<u8>, CheckpointError> {
    let mut e = Encoder::new(Writer {
        bytes: Vec::new(),
        max: limits.wire.max_block_bytes,
    });
    e.array(10)
        .and_then(|e| e.str("raiv-host/local-checkpoint"))
        .and_then(|e| e.u64(1))
        .and_then(|e| e.bytes(&data.anchor.agent_id))
        .and_then(|e| e.bytes(data.anchor.root.as_bytes()))
        .and_then(|e| e.u64(data.anchor.revision))
        .and_then(|e| e.array(3))
        .and_then(|e| e.bytes(&data.program.module_digest))
        .and_then(|e| e.bytes(data.program.definition.as_bytes()))
        .and_then(|e| e.bytes(&data.program.execution_domain))
        .and_then(|e| e.array(2))
        .and_then(|e| e.u64(data.resources.memory_bytes as u64))
        .and_then(|e| e.u64(data.resources.fuel_per_step))
        .and_then(|e| e.str("local-only"))
        .and_then(|e| e.array(data.required.len() as u64))
        .map_err(err)?;
    for cid in &data.required {
        e.bytes(cid.as_bytes()).map_err(err)?;
    }
    e.array(data.admissions.len() as u64).map_err(err)?;
    for r in &data.admissions {
        let b = r.binding;
        let grant = r
            .grant
            .encode(&limits.wire)
            .map_err(|_| CheckpointError::Schema)?;
        e.array(5)
            .and_then(|e| e.array(6))
            .and_then(|e| e.bytes(&b.agent_id))
            .and_then(|e| e.bytes(&b.execution_domain))
            .and_then(|e| e.bytes(b.expected_root.as_bytes()))
            .and_then(|e| e.u64(b.expected_revision))
            .and_then(|e| e.bytes(&b.admitted_event_id))
            .and_then(|e| e.bytes(&b.admitted_input_digest))
            .and_then(|e| e.bytes(&grant))
            .and_then(|e| {
                e.u64(match r.status {
                    AdmissionStatus::Reserved => 0,
                    AdmissionStatus::Attempted => 1,
                    AdmissionStatus::Launched => 2,
                    AdmissionStatus::Consumed => 3,
                })
            })
            .and_then(|e| e.u64(r.launch_attempts))
            .map_err(err)?;
        match r.accepted_root {
            Some(cid) => {
                e.bytes(cid.as_bytes()).map_err(err)?;
            }
            None => {
                e.null().map_err(err)?;
            }
        }
    }
    let bytes = e.into_writer().bytes;
    validate_drisl(&bytes, &limits.wire).map_err(CheckpointError::Codec)?;
    Ok(bytes)
}

fn array(d: &mut Decoder<'_>, n: u64) -> Result<(), CheckpointError> {
    if d.array().map_err(|_| CheckpointError::Schema)? != Some(n) {
        return Err(CheckpointError::Schema);
    }
    Ok(())
}

fn number(d: &mut Decoder<'_>) -> Result<u64, CheckpointError> {
    d.u64().map_err(|_| CheckpointError::Schema)
}

fn id(d: &mut Decoder<'_>) -> Result<[u8; 32], CheckpointError> {
    d.bytes()
        .map_err(|_| CheckpointError::Schema)?
        .try_into()
        .map_err(|_| CheckpointError::Schema)
}

fn cid(d: &mut Decoder<'_>) -> Result<DaslCid, CheckpointError> {
    DaslCid::from_bytes(d.bytes().map_err(|_| CheckpointError::Schema)?)
        .and_then(DaslCid::require_state)
        .map_err(|_| CheckpointError::Schema)
}

fn text(d: &mut Decoder<'_>, expected: &str) -> Result<(), CheckpointError> {
    if d.str().map_err(|_| CheckpointError::Schema)? != expected {
        return Err(CheckpointError::Schema);
    }
    Ok(())
}

fn count(d: &mut Decoder<'_>, max: u64) -> Result<usize, CheckpointError> {
    let n = d
        .array()
        .map_err(|_| CheckpointError::Schema)?
        .ok_or(CheckpointError::Schema)?;
    if n > max {
        return Err(CheckpointError::Limit);
    }
    usize::try_from(n).map_err(|_| CheckpointError::Limit)
}

pub(super) fn decode(
    bytes: &[u8],
    limits: &ManifestLimits,
) -> Result<CheckpointManifest, CheckpointError> {
    let mut d = Decoder::new(bytes);
    array(&mut d, 10)?;
    text(&mut d, "raiv-host/local-checkpoint")?;
    if number(&mut d)? != 1 {
        return Err(CheckpointError::Schema);
    }
    let anchor = Anchor {
        agent_id: id(&mut d)?,
        root: cid(&mut d)?,
        revision: number(&mut d)?,
    };
    array(&mut d, 3)?;
    let program = ProgramBinding {
        module_digest: id(&mut d)?,
        definition: cid(&mut d)?,
        execution_domain: id(&mut d)?,
    };
    array(&mut d, 2)?;
    let resources = VmLimits {
        memory_bytes: usize::try_from(number(&mut d)?).map_err(|_| CheckpointError::Schema)?,
        fuel_per_step: number(&mut d)?,
    };
    text(&mut d, "local-only")?;
    let n = count(&mut d, limits.max_blocks)?;
    let mut required = Vec::new();
    required
        .try_reserve_exact(n)
        .map_err(|_| CheckpointError::Allocation)?;
    for _ in 0..n {
        required.push(cid(&mut d)?);
    }
    let n = count(&mut d, limits.max_admissions)?;
    let mut admissions = Vec::new();
    admissions
        .try_reserve_exact(n)
        .map_err(|_| CheckpointError::Allocation)?;
    for _ in 0..n {
        array(&mut d, 5)?;
        array(&mut d, 6)?;
        let binding = InputBinding {
            agent_id: id(&mut d)?,
            execution_domain: id(&mut d)?,
            expected_root: cid(&mut d)?,
            expected_revision: number(&mut d)?,
            admitted_event_id: id(&mut d)?,
            admitted_input_digest: id(&mut d)?,
        };
        let grant = BudgetAdmission::decode(
            d.bytes().map_err(|_| CheckpointError::Schema)?,
            &limits.wire,
        )
        .map_err(|_| CheckpointError::Schema)?;
        let status = match number(&mut d)? {
            0 => AdmissionStatus::Reserved,
            1 => AdmissionStatus::Attempted,
            2 => AdmissionStatus::Launched,
            3 => AdmissionStatus::Consumed,
            _ => return Err(CheckpointError::Schema),
        };
        let launch_attempts = number(&mut d)?;
        let accepted_root =
            if d.datatype().map_err(|_| CheckpointError::Schema)? == minicbor::data::Type::Null {
                d.null().map_err(|_| CheckpointError::Schema)?;
                None
            } else {
                Some(cid(&mut d)?)
            };
        admissions.push(AdmissionRecord {
            binding,
            grant,
            status,
            launch_attempts,
            accepted_root,
        });
    }
    if d.position() != bytes.len() {
        return Err(CheckpointError::Schema);
    }
    Ok(CheckpointManifest {
        anchor,
        program,
        resources,
        required,
        admissions,
    })
}
