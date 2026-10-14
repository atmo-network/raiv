//! Private canonical CBOR transport; state records remain opaque exact DRISL bytes.
use alloc::vec::Vec;
use minicbor::{encode::Write, Decoder, Encoder};
use raiv::format::{validate_drisl, DecodeLimits};

pub const MAX_FRAME_BYTES: usize = 8_388_608;
const MAX_ENTRIES: u64 = 65_536;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AbiError {
    InvalidFrame,
    Version,
    UnsupportedOperation,
    AlreadyInitialized,
    InvalidState,
    Allocation,
    FrameLimit,
    Busy,
    InvalidActivation,
    UnsupportedProfile,
    RegistryMismatch,
    WrongRequest,
    InferenceFailure,
    RunFailure,
    CandidateFailure,
}
impl AbiError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidFrame => "invalid_abi_frame",
            Self::Version => "unsupported_abi_version",
            Self::UnsupportedOperation => "guest_operation_not_implemented",
            Self::AlreadyInitialized => "already_initialized",
            Self::InvalidState => "invalid_initial_state",
            Self::Allocation => "allocation_failure",
            Self::FrameLimit => "abi_frame_limit",
            Self::Busy => "guest_instance_busy",
            Self::InvalidActivation => "invalid_activation",
            Self::UnsupportedProfile => "unsupported_scripted_profile",
            Self::RegistryMismatch => "compiled_registry_mismatch",
            Self::WrongRequest => "wrong_inference_request",
            Self::InferenceFailure => "inference_failed",
            Self::RunFailure => "activation_failed",
            Self::CandidateFailure => "candidate_failed",
        }
    }
}
struct Writer(Vec<u8>);
impl Write for Writer {
    type Error = AbiError;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let end = self
            .0
            .len()
            .checked_add(bytes.len())
            .ok_or(AbiError::FrameLimit)?;
        if end > MAX_FRAME_BYTES {
            return Err(AbiError::FrameLimit);
        }
        self.0
            .try_reserve(bytes.len())
            .map_err(|_| AbiError::Allocation)?;
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}
fn preflight(bytes: &[u8]) -> Result<(), AbiError> {
    validate_drisl(
        bytes,
        &DecodeLimits {
            max_block_bytes: MAX_FRAME_BYTES as u64,
            max_depth: 64,
            max_collection_entries: MAX_ENTRIES,
        },
    )
    .map_err(|_| AbiError::InvalidFrame)?;
    let mut decoder = Decoder::new(bytes);
    let mut total = 0u64;
    while decoder.position() < bytes.len() {
        let count = match bytes[decoder.position()] >> 5 {
            4 => decoder
                .array()
                .map_err(|_| AbiError::InvalidFrame)?
                .ok_or(AbiError::InvalidFrame)?,
            5 => decoder
                .map()
                .map_err(|_| AbiError::InvalidFrame)?
                .ok_or(AbiError::InvalidFrame)?,
            6 => {
                decoder.tag().map_err(|_| AbiError::InvalidFrame)?;
                0
            }
            _ => {
                decoder.skip().map_err(|_| AbiError::InvalidFrame)?;
                0
            }
        };
        total = total.checked_add(count).ok_or(AbiError::FrameLimit)?;
        if total > MAX_ENTRIES {
            return Err(AbiError::FrameLimit);
        }
    }
    Ok(())
}
pub struct Initialize<'a> {
    pub agent_id: [u8; 32],
    pub definition: &'a [u8],
    pub memory: Option<&'a [u8]>,
}
fn envelope(bytes: &[u8]) -> Result<(&str, &[u8]), AbiError> {
    preflight(bytes)?;
    let mut decoder = Decoder::new(bytes);
    if decoder.map().map_err(|_| AbiError::InvalidFrame)? != Some(3) {
        return Err(AbiError::InvalidFrame);
    }
    let mut version = None;
    let mut kind = None;
    let mut payload = None;
    for _ in 0..3 {
        match decoder.str().map_err(|_| AbiError::InvalidFrame)? {
            "abi_version" => version = Some(decoder.u64().map_err(|_| AbiError::InvalidFrame)?),
            "type" => kind = Some(decoder.str().map_err(|_| AbiError::InvalidFrame)?),
            "payload" => {
                let start = decoder.position();
                decoder.skip().map_err(|_| AbiError::InvalidFrame)?;
                payload = Some(&bytes[start..decoder.position()]);
            }
            _ => return Err(AbiError::InvalidFrame),
        }
    }
    if version != Some(1) {
        return Err(AbiError::Version);
    }
    Ok((
        kind.ok_or(AbiError::InvalidFrame)?,
        payload.ok_or(AbiError::InvalidFrame)?,
    ))
}
fn initialize(payload: &[u8]) -> Result<Initialize<'_>, AbiError> {
    let mut decoder = Decoder::new(payload);
    if decoder.map().map_err(|_| AbiError::InvalidFrame)? != Some(3) {
        return Err(AbiError::InvalidFrame);
    }
    let mut agent = None;
    let mut definition = None;
    let mut memory = None;
    for _ in 0..3 {
        match decoder.str().map_err(|_| AbiError::InvalidFrame)? {
            "agent_id" => {
                agent = Some(
                    decoder
                        .bytes()
                        .map_err(|_| AbiError::InvalidFrame)?
                        .try_into()
                        .map_err(|_| AbiError::InvalidFrame)?,
                )
            }
            "definition" => definition = Some(decoder.bytes().map_err(|_| AbiError::InvalidFrame)?),
            "memory" => {
                if decoder.datatype().map_err(|_| AbiError::InvalidFrame)?
                    == minicbor::data::Type::Null
                {
                    decoder.null().map_err(|_| AbiError::InvalidFrame)?;
                    memory = Some(None);
                } else {
                    memory = Some(Some(decoder.bytes().map_err(|_| AbiError::InvalidFrame)?));
                }
            }
            _ => return Err(AbiError::InvalidFrame),
        }
    }
    Ok(Initialize {
        agent_id: agent.ok_or(AbiError::InvalidFrame)?,
        definition: definition.ok_or(AbiError::InvalidFrame)?,
        memory: memory.ok_or(AbiError::InvalidFrame)?,
    })
}
pub enum Input<'a> {
    Initialize(Initialize<'a>),
    Activate(Activate<'a>),
    InferenceError { request_id: u64 },
    InferenceReply { request_id: u64, reply: &'a [u8] },
}
pub struct Activate<'a> {
    pub root: raiv::format::DaslCid,
    pub input: &'a [u8],
    pub blocks: Vec<(raiv::format::DaslCid, &'a [u8])>,
}
impl raiv::graph::MaterializedBlocks for Activate<'_> {
    fn get(&self, cid: raiv::format::DaslCid) -> Option<&[u8]> {
        self.blocks
            .iter()
            .find(|(key, _)| *key == cid)
            .map(|(_, body)| *body)
    }
}
pub fn decode(bytes: &[u8]) -> Result<Input<'_>, AbiError> {
    let (kind, payload) = envelope(bytes)?;
    match kind {
        "initialize" => Ok(Input::Initialize(initialize(payload)?)),
        "activate" => Ok(Input::Activate(activate(payload)?)),
        "inference_reply" => {
            let mut d = Decoder::new(payload);
            if d.map().map_err(|_| AbiError::InvalidFrame)? != Some(2) {
                return Err(AbiError::InvalidFrame);
            }
            let mut id = None;
            let mut reply = None;
            for _ in 0..2 {
                match d.str().map_err(|_| AbiError::InvalidFrame)? {
                    "request_id" => id = Some(d.u64().map_err(|_| AbiError::InvalidFrame)?),
                    "reply" => {
                        let start = d.position();
                        if d.datatype().map_err(|_| AbiError::InvalidFrame)?
                            != minicbor::data::Type::Map
                        {
                            return Err(AbiError::InvalidFrame);
                        }
                        d.skip().map_err(|_| AbiError::InvalidFrame)?;
                        reply = Some(&payload[start..d.position()]);
                    }
                    _ => return Err(AbiError::InvalidFrame),
                }
            }
            Ok(Input::InferenceReply {
                request_id: id.ok_or(AbiError::InvalidFrame)?,
                reply: reply.ok_or(AbiError::InvalidFrame)?,
            })
        }
        "inference_error" => {
            let mut d = Decoder::new(payload);
            if d.map().map_err(|_| AbiError::InvalidFrame)? != Some(2) {
                return Err(AbiError::InvalidFrame);
            }
            let mut id = None;
            let mut code = None;
            for _ in 0..2 {
                match d.str().map_err(|_| AbiError::InvalidFrame)? {
                    "request_id" => id = Some(d.u64().map_err(|_| AbiError::InvalidFrame)?),
                    "code" => code = Some(d.str().map_err(|_| AbiError::InvalidFrame)?),
                    _ => return Err(AbiError::InvalidFrame),
                }
            }
            if code.ok_or(AbiError::InvalidFrame)?.len() > 256 {
                return Err(AbiError::FrameLimit);
            }
            Ok(Input::InferenceError {
                request_id: id.ok_or(AbiError::InvalidFrame)?,
            })
        }
        _ => Err(AbiError::UnsupportedOperation),
    }
}
fn activate(payload: &[u8]) -> Result<Activate<'_>, AbiError> {
    use raiv::format::{DaslCid, DaslCodec};
    let limits = raiv::limits::Limits::default();
    let mut d = Decoder::new(payload);
    if d.map().map_err(|_| AbiError::InvalidFrame)? != Some(3) {
        return Err(AbiError::InvalidFrame);
    }
    let mut root = None;
    let mut input = None;
    let mut blocks = None;
    for _ in 0..3 {
        match d.str().map_err(|_| AbiError::InvalidFrame)? {
            "root" => {
                let cid = DaslCid::from_bytes(d.bytes().map_err(|_| AbiError::InvalidFrame)?)
                    .map_err(|_| AbiError::InvalidFrame)?;
                cid.require_state().map_err(|_| AbiError::InvalidFrame)?;
                root = Some(cid);
            }
            "input" => {
                let bytes = d.bytes().map_err(|_| AbiError::InvalidFrame)?;
                if bytes.len() as u64 > limits.max_block_bytes {
                    return Err(AbiError::FrameLimit);
                }
                input = Some(bytes);
            }
            "blocks" => {
                let count = d
                    .array()
                    .map_err(|_| AbiError::InvalidFrame)?
                    .ok_or(AbiError::InvalidFrame)?;
                if count > limits.max_blocks {
                    return Err(AbiError::FrameLimit);
                }
                let mut items: Vec<(DaslCid, &[u8])> = Vec::new();
                items
                    .try_reserve_exact(count as usize)
                    .map_err(|_| AbiError::Allocation)?;
                let mut total = 0u64;
                for _ in 0..count {
                    if d.map().map_err(|_| AbiError::InvalidFrame)? != Some(2) {
                        return Err(AbiError::InvalidFrame);
                    }
                    let mut cid = None;
                    let mut body = None;
                    for _ in 0..2 {
                        match d.str().map_err(|_| AbiError::InvalidFrame)? {
                            "cid" => {
                                cid = Some(
                                    DaslCid::from_bytes(
                                        d.bytes().map_err(|_| AbiError::InvalidFrame)?,
                                    )
                                    .map_err(|_| AbiError::InvalidFrame)?,
                                )
                            }
                            "body" => body = Some(d.bytes().map_err(|_| AbiError::InvalidFrame)?),
                            _ => return Err(AbiError::InvalidFrame),
                        }
                    }
                    let cid = cid.ok_or(AbiError::InvalidFrame)?;
                    let body = body.ok_or(AbiError::InvalidFrame)?;
                    total = total
                        .checked_add(body.len() as u64)
                        .ok_or(AbiError::FrameLimit)?;
                    if body.len() as u64 > limits.max_block_bytes
                        || total > limits.max_materialized_bytes
                    {
                        return Err(AbiError::FrameLimit);
                    }
                    cid.require_state().map_err(|_| AbiError::InvalidFrame)?;
                    if DaslCid::for_block(DaslCodec::Drisl, body) != cid
                        || items.iter().any(|(key, _)| *key == cid)
                    {
                        return Err(AbiError::InvalidActivation);
                    }
                    items.push((cid, body));
                }
                blocks = Some(items);
            }
            _ => return Err(AbiError::InvalidFrame),
        }
    }
    Ok(Activate {
        root: root.ok_or(AbiError::InvalidFrame)?,
        input: input.ok_or(AbiError::InvalidFrame)?,
        blocks: blocks.ok_or(AbiError::InvalidFrame)?,
    })
}
pub fn infer(request: raiv::activation::ModelRequest<'_>) -> Result<Vec<u8>, AbiError> {
    let limits = request
        .limits()
        .encode(&raiv::format::DecodeLimits::default())
        .map_err(|_| AbiError::InvalidActivation)?;
    let mut e = Encoder::new(Writer(Vec::new()));
    let result = (|| -> Result<(), minicbor::encode::Error<AbiError>> {
        e.map(3)?
            .str("type")?
            .str("infer")?
            .str("payload")?
            .map(2)?
            .str("request")?
            .map(5)?
            .str("tools")?
            .array(request.tools().count() as u64)?;
        for tool in request.tools() {
            e.map(6)?
                .str("id")?
                .str(tool.id)?
                .str("control")?
                .str(match tool.control {
                    raiv::definition::ToolControl::Continue => "continue",
                    raiv::definition::ToolControl::Yield => "yield",
                    raiv::definition::ToolControl::Finish => "finish",
                })?
                .str("version")?
                .u64(tool.version)?
                .str("description")?
                .str(tool.description)?
                .str("arguments_schema")?;
            e.writer_mut()
                .write_all(tool.arguments_schema)
                .map_err(minicbor::encode::Error::write)?;
            e.str("authority_binding")?;
            if let Some(authority) = tool.authority_binding {
                e.bytes(&authority)?;
            } else {
                e.null()?;
            }
        }
        e.str("limits")?;
        e.writer_mut()
            .write_all(&limits)
            .map_err(minicbor::encode::Error::write)?;
        e.str("context")?
            .bytes(request.context_bytes())?
            .str("model_parameters")?;
        e.writer_mut()
            .write_all(request.model_parameters_bytes())
            .map_err(minicbor::encode::Error::write)?;
        e.str("projection_version")?
            .u64(request.projection_version())?
            .str("request_id")?
            .u64(request.binding().request_id)?
            .str("abi_version")?
            .u8(1)?;
        Ok(())
    })();
    result.map_err(|_| AbiError::Allocation)?;
    let output = e.into_writer().0;
    preflight(&output)?;
    Ok(output)
}
pub fn boundary(candidate: &raiv::commit::CommitCandidate) -> Result<Vec<u8>, AbiError> {
    let binding = candidate.binding();
    let mut e = Encoder::new(Writer(Vec::new()));
    let result = (|| -> Result<(), minicbor::encode::Error<AbiError>> {
        e.map(3)?
            .str("type")?
            .str("boundary")?
            .str("payload")?
            .map(1)?
            .str("candidate")?
            .map(8)?
            .str("agent_id")?
            .bytes(&binding.agent_id)?
            .str("new_root")?
            .bytes(candidate.new_root().as_bytes())?
            .str("new_blocks")?
            .array(candidate.new_blocks().len() as u64)?;
        for (cid, body) in candidate.new_blocks() {
            e.map(2)?
                .str("cid")?
                .bytes(cid.as_bytes())?
                .str("body")?
                .bytes(body)?;
        }
        e.str("disposition")?;
        match candidate.disposition() {
            raiv::state::Continuation::AwaitActor => {
                e.map(1)?.str("tag")?.str("await_actor")?;
            }
            raiv::state::Continuation::AwaitAction { action_id } => {
                e.map(2)?
                    .str("tag")?
                    .str("await_action")?
                    .str("action_id")?
                    .bytes(&action_id)?;
            }
        }
        e.str("expected_root")?
            .bytes(binding.expected_root.as_bytes())?
            .str("admitted_event_id")?
            .bytes(&binding.admitted_event_id)?
            .str("expected_revision")?
            .u64(binding.expected_revision)?
            .str("admitted_input_digest")?
            .bytes(&binding.admitted_input_digest)?
            .str("abi_version")?
            .u8(1)?;
        Ok(())
    })();
    result.map_err(|_| AbiError::Allocation)?;
    let output = e.into_writer().0;
    preflight(&output)?;
    Ok(output)
}
pub fn initialized(genesis: &raiv::graph::Genesis) -> Result<Vec<u8>, AbiError> {
    let mut e = Encoder::new(Writer(Vec::new()));
    let result = (|| -> Result<(), minicbor::encode::Error<AbiError>> {
        e.map(3)?
            .str("type")?
            .str("initialized")?
            .str("payload")?
            .map(2)?
            .str("root")?
            .bytes(genesis.root().as_bytes())?
            .str("blocks")?
            .array(genesis.blocks().count() as u64)?;
        for (cid, body) in genesis.blocks() {
            e.map(2)?
                .str("cid")?
                .bytes(cid.as_bytes())?
                .str("body")?
                .bytes(body)?;
        }
        e.str("abi_version")?.u8(1)?;
        Ok(())
    })();
    result.map_err(|_| AbiError::Allocation)?;
    Ok(e.into_writer().0)
}
pub fn error(error: AbiError) -> Result<Vec<u8>, AbiError> {
    let mut e = Encoder::new(Writer(Vec::new()));
    e.map(3)
        .and_then(|e| e.str("type"))
        .and_then(|e| e.str("error"))
        .and_then(|e| e.str("payload"))
        .and_then(|e| e.map(1))
        .and_then(|e| e.str("code"))
        .and_then(|e| e.str(error.code()))
        .and_then(|e| e.str("abi_version"))
        .and_then(|e| e.u8(1))
        .map_err(|_| AbiError::Allocation)?;
    Ok(e.into_writer().0)
}
