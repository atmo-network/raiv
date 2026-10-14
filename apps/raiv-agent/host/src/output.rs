//! Borrowed closed guest output DATA; neither authenticated execution nor a dispatch permit.
use minicbor::Decoder;
use raiv::{
    definition::{ToolControl, ToolDescriptorView},
    format::{validate_drisl, validate_model_value, DaslCid, DecodeLimits},
    graph::MaterializedBlocks,
    limits::Limits,
    state::Continuation,
};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    Malformed,
    Version,
    Limit,
    Integrity,
    Allocation,
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "invalid closed guest output schema",
            Self::Version => "unsupported guest output version",
            Self::Limit => "guest output resource limit",
            Self::Integrity => "guest output block integrity failure",
            Self::Allocation => "guest output metadata allocation failure",
        })
    }
}
impl std::error::Error for DecodeError {}
#[derive(Clone, Copy)]
pub enum Output<'a> {
    Initialized {
        root: DaslCid,
        blocks: Blocks<'a>,
    },
    Infer {
        request_id: u64,
        request: Request<'a>,
    },
    Boundary(Candidate<'a>),
    Error {
        code: &'a str,
    },
}
#[derive(Clone, Copy)]
pub struct Block<'a> {
    pub cid: DaslCid,
    pub body: &'a [u8],
}
#[derive(Clone, Copy)]
pub struct Blocks<'a> {
    bytes: &'a [u8],
    len: u64,
}
impl<'a> Blocks<'a> {
    pub fn len(self) -> u64 {
        self.len
    }
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    pub fn iter(self) -> impl Iterator<Item = Block<'a>> {
        list(self.bytes)
            .expect("validated blocks")
            .map(|bytes| block(bytes).expect("validated block"))
    }
}
impl MaterializedBlocks for Blocks<'_> {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.iter().find(|b| b.cid == cid).map(|b| b.body)
    }
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    context: &'a [u8],
    parameters: &'a [u8],
    tools: &'a [u8],
    limits: Limits,
}
impl<'a> Request<'a> {
    pub fn context_bytes(self) -> &'a [u8] {
        self.context
    }
    pub fn model_parameters_bytes(self) -> &'a [u8] {
        self.parameters
    }
    pub fn limits(self) -> Limits {
        self.limits
    }
    pub fn tools(self) -> impl Iterator<Item = ToolDescriptorView<'a>> {
        list(self.tools)
            .expect("validated tools")
            .map(|b| tool(b).expect("validated tool"))
    }
}
#[derive(Clone, Copy)]
pub struct Candidate<'a> {
    agent: [u8; 32],
    revision: u64,
    expected: DaslCid,
    event: [u8; 32],
    digest: [u8; 32],
    root: DaslCid,
    blocks: Blocks<'a>,
    disposition: Continuation,
}
impl<'a> Candidate<'a> {
    pub fn agent_id(self) -> [u8; 32] {
        self.agent
    }
    pub fn expected_revision(self) -> u64 {
        self.revision
    }
    pub fn expected_root(self) -> DaslCid {
        self.expected
    }
    pub fn admitted_event_id(self) -> [u8; 32] {
        self.event
    }
    pub fn admitted_input_digest(self) -> [u8; 32] {
        self.digest
    }
    pub fn new_root(self) -> DaslCid {
        self.root
    }
    pub fn new_blocks(self) -> Blocks<'a> {
        self.blocks
    }
    pub fn disposition(self) -> Continuation {
        self.disposition
    }
}
impl<'a> Output<'a> {
    pub fn decode(bytes: &'a [u8], caller: &Limits) -> Result<Self, DecodeError> {
        preflight(bytes)?;
        let [version, kind, payload] = fields(bytes, ["abi_version", "type", "payload"])?;
        if unsigned(version)? != 1 {
            return Err(DecodeError::Version);
        }
        match text(kind)? {
            "error" => {
                let [code] = fields(payload, ["code"])?;
                let code = text(code)?;
                if code.len() > 256 {
                    return Err(DecodeError::Limit);
                }
                Ok(Self::Error { code })
            }
            "initialized" => {
                let [root, items] = fields(payload, ["root", "blocks"])?;
                let root = cid(root)?;
                let blocks = blocks(items, caller, false)?;
                if !blocks.iter().any(|b| b.cid == root) {
                    return Err(DecodeError::Malformed);
                }
                Ok(Self::Initialized { root, blocks })
            }
            "infer" => {
                let [id, request] = fields(payload, ["request_id", "request"])?;
                let request_id = unsigned(id)?;
                if request_id == 0 {
                    return Err(DecodeError::Malformed);
                }
                Ok(Self::Infer {
                    request_id,
                    request: request_view(request, caller)?,
                })
            }
            "boundary" => {
                let [value] = fields(payload, ["candidate"])?;
                let [agent, revision, expected, event, digest, root, items, disposition] = fields(
                    value,
                    [
                        "agent_id",
                        "expected_revision",
                        "expected_root",
                        "admitted_event_id",
                        "admitted_input_digest",
                        "new_root",
                        "new_blocks",
                        "disposition",
                    ],
                )?;
                let root = cid(root)?;
                let blocks = blocks(items, caller, true)?;
                if !blocks.iter().any(|b| b.cid == root) {
                    return Err(DecodeError::Malformed);
                }
                Ok(Self::Boundary(Candidate {
                    agent: id32(agent)?,
                    revision: unsigned(revision)?,
                    expected: cid(expected)?,
                    event: id32(event)?,
                    digest: id32(digest)?,
                    root,
                    blocks,
                    disposition: continuation(disposition)?,
                }))
            }
            _ => Err(DecodeError::Malformed),
        }
    }
}
fn preflight(bytes: &[u8]) -> Result<(), DecodeError> {
    if bytes.len() > super::MAX_FRAME_BYTES {
        return Err(DecodeError::Limit);
    }
    validate_drisl(
        bytes,
        &DecodeLimits {
            max_block_bytes: super::MAX_FRAME_BYTES as u64,
            max_depth: 64,
            max_collection_entries: 65_536,
        },
    )
    .map_err(|_| DecodeError::Malformed)?;
    let mut d = Decoder::new(bytes);
    let mut entries = 0u64;
    while d.position() < bytes.len() {
        let count = match bytes[d.position()] >> 5 {
            4 => d
                .array()
                .map_err(|_| DecodeError::Malformed)?
                .ok_or(DecodeError::Malformed)?,
            5 => d
                .map()
                .map_err(|_| DecodeError::Malformed)?
                .ok_or(DecodeError::Malformed)?,
            6 => {
                d.tag().map_err(|_| DecodeError::Malformed)?;
                0
            }
            _ => {
                d.skip().map_err(|_| DecodeError::Malformed)?;
                0
            }
        };
        entries = entries.checked_add(count).ok_or(DecodeError::Limit)?;
        if entries > 65_536 {
            return Err(DecodeError::Limit);
        }
    }
    Ok(())
}
fn fields<'a, const N: usize>(
    bytes: &'a [u8],
    keys: [&str; N],
) -> Result<[&'a [u8]; N], DecodeError> {
    let mut d = Decoder::new(bytes);
    if d.map().map_err(|_| DecodeError::Malformed)? != Some(N as u64) {
        return Err(DecodeError::Malformed);
    }
    let mut found = [None; N];
    for _ in 0..N {
        let key = d.str().map_err(|_| DecodeError::Malformed)?;
        let index = keys
            .iter()
            .position(|k| *k == key)
            .ok_or(DecodeError::Malformed)?;
        if found[index].is_some() {
            return Err(DecodeError::Malformed);
        }
        let start = d.position();
        d.skip().map_err(|_| DecodeError::Malformed)?;
        found[index] = Some(&bytes[start..d.position()]);
    }
    if d.position() != bytes.len() {
        return Err(DecodeError::Malformed);
    }
    Ok(found.map(|v| v.expect("exact unique field set")))
}
fn unsigned(bytes: &[u8]) -> Result<u64, DecodeError> {
    Decoder::new(bytes)
        .u64()
        .map_err(|_| DecodeError::Malformed)
}
fn text(bytes: &[u8]) -> Result<&str, DecodeError> {
    Decoder::new(bytes)
        .str()
        .map_err(|_| DecodeError::Malformed)
}
fn raw(bytes: &[u8]) -> Result<&[u8], DecodeError> {
    Decoder::new(bytes)
        .bytes()
        .map_err(|_| DecodeError::Malformed)
}
fn id32(bytes: &[u8]) -> Result<[u8; 32], DecodeError> {
    raw(bytes)?.try_into().map_err(|_| DecodeError::Malformed)
}
fn cid(bytes: &[u8]) -> Result<DaslCid, DecodeError> {
    DaslCid::from_bytes(raw(bytes)?)
        .and_then(DaslCid::require_state)
        .map_err(|_| DecodeError::Malformed)
}
fn list(bytes: &[u8]) -> Result<impl Iterator<Item = &[u8]>, DecodeError> {
    let mut d = Decoder::new(bytes);
    let len = d
        .array()
        .map_err(|_| DecodeError::Malformed)?
        .ok_or(DecodeError::Malformed)?;
    Ok((0..len).map(move |_| {
        let start = d.position();
        d.skip().expect("preflighted list value");
        &bytes[start..d.position()]
    }))
}
fn block(bytes: &[u8]) -> Result<Block<'_>, DecodeError> {
    let [key, body] = fields(bytes, ["cid", "body"])?;
    Ok(Block {
        cid: cid(key)?,
        body: raw(body)?,
    })
}
fn blocks<'a>(bytes: &'a [u8], caller: &Limits, sorted: bool) -> Result<Blocks<'a>, DecodeError> {
    let len = Decoder::new(bytes)
        .array()
        .map_err(|_| DecodeError::Malformed)?
        .ok_or(DecodeError::Malformed)?;
    if len > caller.max_blocks || len > Limits::default().max_blocks {
        return Err(DecodeError::Limit);
    }
    let mut ids = Vec::new();
    ids.try_reserve_exact(len as usize)
        .map_err(|_| DecodeError::Allocation)?;
    let mut total = 0u64;
    let mut previous = None;
    for value in list(bytes)? {
        let b = block(value)?;
        total = total
            .checked_add(b.body.len() as u64)
            .ok_or(DecodeError::Limit)?;
        if total > caller.max_materialized_bytes {
            return Err(DecodeError::Limit);
        }
        validate_drisl(b.body, &caller.decode_limits()).map_err(|_| DecodeError::Limit)?;
        b.cid.verify(b.body).map_err(|_| DecodeError::Integrity)?;
        if sorted && previous.is_some_and(|id| id >= b.cid) {
            return Err(DecodeError::Malformed);
        }
        previous = Some(b.cid);
        ids.push(b.cid);
    }
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(DecodeError::Malformed);
    }
    Ok(Blocks { bytes, len })
}
fn request_view<'a>(bytes: &'a [u8], caller: &Limits) -> Result<Request<'a>, DecodeError> {
    let [version, context, parameters, tools, limits] = fields(
        bytes,
        [
            "projection_version",
            "context",
            "model_parameters",
            "tools",
            "limits",
        ],
    )?;
    if unsigned(version)? != 1 {
        return Err(DecodeError::Version);
    }
    let limits =
        Limits::decode(limits, &caller.decode_limits()).map_err(|_| DecodeError::Malformed)?;
    let bounds = DecodeLimits {
        max_block_bytes: caller.max_context_bytes.min(limits.max_context_bytes),
        max_depth: caller.max_depth.min(limits.max_depth),
        max_collection_entries: caller
            .max_collection_entries
            .min(limits.max_collection_entries),
    };
    let context = raw(context)?;
    validate_model_value(context, &bounds).map_err(|_| DecodeError::Limit)?;
    if Decoder::new(context)
        .array()
        .map_err(|_| DecodeError::Malformed)?
        != Some(7)
    {
        return Err(DecodeError::Malformed);
    }
    validate_model_value(parameters, &bounds).map_err(|_| DecodeError::Limit)?;
    Decoder::new(parameters)
        .map()
        .map_err(|_| DecodeError::Malformed)?;
    let count = Decoder::new(tools)
        .array()
        .map_err(|_| DecodeError::Malformed)?
        .ok_or(DecodeError::Malformed)?;
    if count > bounds.max_collection_entries {
        return Err(DecodeError::Limit);
    }
    let mut ids = Vec::new();
    ids.try_reserve_exact(usize::try_from(count).map_err(|_| DecodeError::Limit)?)
        .map_err(|_| DecodeError::Allocation)?;
    for value in list(tools)? {
        let tool = tool(value)?;
        if ids.contains(&tool.id) {
            return Err(DecodeError::Malformed);
        }
        validate_model_value(tool.arguments_schema, &bounds).map_err(|_| DecodeError::Limit)?;
        Decoder::new(tool.arguments_schema)
            .map()
            .map_err(|_| DecodeError::Malformed)?;
        ids.push(tool.id);
    }
    Ok(Request {
        context,
        parameters,
        tools,
        limits,
    })
}
fn tool(bytes: &[u8]) -> Result<ToolDescriptorView<'_>, DecodeError> {
    let [id, version, control, description, schema, authority] = fields(
        bytes,
        [
            "id",
            "version",
            "control",
            "description",
            "arguments_schema",
            "authority_binding",
        ],
    )?;
    let control = match text(control)? {
        "continue" => ToolControl::Continue,
        "yield" => ToolControl::Yield,
        "finish" => ToolControl::Finish,
        _ => return Err(DecodeError::Malformed),
    };
    let authority_binding = if authority == [0xf6] {
        None
    } else {
        Some(id32(authority)?)
    };
    if control == ToolControl::Yield && authority_binding.is_none() {
        return Err(DecodeError::Malformed);
    }
    Ok(ToolDescriptorView {
        id: text(id)?,
        version: unsigned(version)?,
        control,
        description: text(description)?,
        arguments_schema: schema,
        authority_binding,
    })
}
fn continuation(bytes: &[u8]) -> Result<Continuation, DecodeError> {
    let mut d = Decoder::new(bytes);
    let count = d
        .map()
        .map_err(|_| DecodeError::Malformed)?
        .ok_or(DecodeError::Malformed)?;
    if count == 1 {
        let [tag] = fields(bytes, ["tag"])?;
        if text(tag)? == "await_actor" {
            return Ok(Continuation::AwaitActor);
        }
    }
    if count == 2 {
        let [tag, id] = fields(bytes, ["tag", "action_id"])?;
        if text(tag)? == "await_action" {
            return Ok(Continuation::AwaitAction {
                action_id: id32(id)?,
            });
        }
    }
    Err(DecodeError::Malformed)
}
