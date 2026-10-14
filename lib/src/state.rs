//! AgentState root data, not restored graph consistency, authoritative HEAD or host admission.

use crate::format::{
    validate_drisl, CodecError, DaslCid, DaslCodec, DecodeLimits, Id32, RecordEncoder, ValueView,
};
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    Codec(CodecError),
    WrongKind,
    UnsupportedVersion,
    UnsupportedFormat,
    InvalidSchema,
}

impl From<CodecError> for StateError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::WrongKind => f.write_str("block is not an AgentState"),
            Self::UnsupportedVersion => f.write_str("unsupported AgentState version"),
            Self::UnsupportedFormat => f.write_str("unsupported AgentState format"),
            Self::InvalidSchema => f.write_str("AgentState requires exact fields, raw IDs, structural links and consistent continuation/current-episode shapes"),
        }
    }
}

impl core::error::Error for StateError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Continuation {
    AwaitActor,
    AwaitAction { action_id: Id32 },
}

impl Continuation {
    fn check_episode(self, present: bool) -> Result<(), StateError> {
        if matches!(self, Self::AwaitAction { .. }) == present {
            Ok(())
        } else {
            Err(StateError::InvalidSchema)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AgentStateSpec<'a> {
    pub agent_id: Id32,
    pub revision: u64,
    pub definition: DaslCid,
    pub memory: Option<DaslCid>,
    pub continuation: Continuation,
    pub current_episode: Option<DaslCid>,
    pub history: &'a [DaslCid],
}

/// Complete immutable `raiv-state/1` root. AwaitActor requires null current_episode;
/// AwaitAction requires an Episode Link and raw action ID. Codec validity does not prove
/// genesis, target kinds, accepted history, Definition-bound limits/memory, current
/// episode openness, command-slot identity, revision transitions or host authority.
#[derive(Debug, Eq, PartialEq)]
pub struct AgentState {
    encoded: Vec<u8>,
    agent_id: Id32,
    revision: u64,
    definition: DaslCid,
    memory: Option<DaslCid>,
    continuation: Continuation,
    current_episode: Option<DaslCid>,
    history: Range<usize>,
    history_count: u64,
}

impl AgentState {
    /// Encode a bounded root data record without declaring it authoritative HEAD.
    ///
    /// ```
    /// use raiv::state::{AgentState, AgentStateSpec, Continuation};
    /// use raiv::format::{DaslCid, DaslCodec, DecodeLimits};
    /// let spec = AgentStateSpec { agent_id: [1; 32], revision: 0,
    ///     definition: DaslCid::from_digest(DaslCodec::Drisl, [2; 32]), memory: None,
    ///     continuation: Continuation::AwaitActor, current_episode: None, history: &[] };
    /// let state = AgentState::encode(&spec, &DecodeLimits::default())?;
    /// assert_eq!(state.continuation(), Continuation::AwaitActor);
    /// assert_eq!(state.history_count(), 0);
    /// # Ok::<(), raiv::state::StateError>(())
    /// ```
    pub fn encode(spec: &AgentStateSpec<'_>, bounds: &DecodeLimits) -> Result<Self, StateError> {
        let depth = if spec.history.is_empty() { 3 } else { 4 };
        if bounds.max_depth < depth {
            return Err(CodecError::DepthLimit.into());
        }
        if spec.history.len() as u64 > bounds.max_collection_entries {
            return Err(CodecError::CollectionLimit.into());
        }
        spec.continuation
            .check_episode(spec.current_episode.is_some())?;
        spec.definition
            .require_state()
            .map_err(|_| CodecError::InvalidLink)?;
        for link in spec
            .memory
            .into_iter()
            .chain(spec.current_episode)
            .chain(spec.history.iter().copied())
        {
            link.require_state().map_err(|_| CodecError::InvalidLink)?;
        }
        let mut encoder = RecordEncoder::new(bounds)?;
        encoder.map(10)?;
        encoder.text("kind")?;
        encoder.text("agent_state")?;
        encoder.text("format")?;
        encoder.text("raiv-state/1")?;
        encoder.text("memory")?;
        match spec.memory {
            Some(memory) => encoder.link(memory)?,
            None => encoder.null()?,
        }
        encoder.text("history")?;
        encoder.list(spec.history.len() as u64)?;
        for episode in spec.history {
            encoder.link(*episode)?;
        }
        encoder.text("version")?;
        encoder.unsigned(1)?;
        encoder.text("agent_id")?;
        encoder.bytes(&spec.agent_id)?;
        encoder.text("revision")?;
        encoder.unsigned(spec.revision)?;
        encoder.text("definition")?;
        encoder.link(spec.definition)?;
        encoder.text("continuation")?;
        match spec.continuation {
            Continuation::AwaitActor => {
                encoder.map(1)?;
                encoder.text("tag")?;
                encoder.text("await_actor")?;
            }
            Continuation::AwaitAction { action_id } => {
                encoder.map(2)?;
                encoder.text("tag")?;
                encoder.text("await_action")?;
                encoder.text("action_id")?;
                encoder.bytes(&action_id)?;
            }
        }
        encoder.text("current_episode")?;
        match spec.current_episode {
            Some(episode) => encoder.link(episode)?,
            None => encoder.null()?,
        }
        let encoded = encoder.finish();
        let mut state = Self::inspect(&encoded, bounds)?;
        state.encoded = encoded;
        Ok(state)
    }

    pub fn decode(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, StateError> {
        let mut state = Self::inspect(bytes, bounds)?;
        state.encoded = crate::format::copy_validated(bytes)?;
        Ok(state)
    }

    fn inspect(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, StateError> {
        validate_drisl(bytes, bounds)?;
        let fields = ValueView::preflighted(bytes)
            .fields([
                "kind",
                "version",
                "format",
                "agent_id",
                "revision",
                "definition",
                "memory",
                "continuation",
                "current_episode",
                "history",
            ])
            .map_err(|_| StateError::InvalidSchema)?;
        if fields[0].text().map_err(|_| StateError::InvalidSchema)? != "agent_state" {
            return Err(StateError::WrongKind);
        }
        if fields[1]
            .unsigned()
            .map_err(|_| StateError::InvalidSchema)?
            != 1
        {
            return Err(StateError::UnsupportedVersion);
        }
        if fields[2].text().map_err(|_| StateError::InvalidSchema)? != "raiv-state/1" {
            return Err(StateError::UnsupportedFormat);
        }
        let agent_id = fields[3]
            .bytes()
            .map_err(|_| StateError::InvalidSchema)?
            .try_into()
            .map_err(|_| StateError::InvalidSchema)?;
        let revision = fields[4]
            .unsigned()
            .map_err(|_| StateError::InvalidSchema)?;
        let definition = fields[5].link().map_err(|_| StateError::InvalidSchema)?;
        let memory = if fields[6].is_null() {
            None
        } else {
            Some(fields[6].link().map_err(|_| StateError::InvalidSchema)?)
        };
        let continuation = inspect_continuation(fields[7])?;
        let current_episode = if fields[8].is_null() {
            None
        } else {
            Some(fields[8].link().map_err(|_| StateError::InvalidSchema)?)
        };
        continuation.check_episode(current_episode.is_some())?;
        let history = fields[9].list().map_err(|_| StateError::InvalidSchema)?;
        let history_count = history.len();
        for episode in history {
            episode?.link().map_err(|_| StateError::InvalidSchema)?;
        }
        Ok(Self {
            encoded: Vec::new(),
            agent_id,
            revision,
            definition,
            memory,
            continuation,
            current_episode,
            history: fields[9].span(),
            history_count,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    pub fn cid(&self) -> DaslCid {
        DaslCid::for_block(DaslCodec::Drisl, self.as_bytes())
    }

    pub fn agent_id(&self) -> Id32 {
        self.agent_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn definition_link(&self) -> DaslCid {
        self.definition
    }

    pub fn memory_link(&self) -> Option<DaslCid> {
        self.memory
    }

    pub fn continuation(&self) -> Continuation {
        self.continuation
    }

    pub fn current_episode_link(&self) -> Option<DaslCid> {
        self.current_episode
    }

    pub fn history_count(&self) -> u64 {
        self.history_count
    }

    /// Ordered, infallible iteration over validated original history bytes; no second CID list.
    pub fn history(&self) -> impl Iterator<Item = DaslCid> + '_ {
        ValueView::preflighted(&self.encoded[self.history.clone()])
            .list()
            .expect("validated immutable AgentState history")
            .map(|value| {
                value
                    .expect("validated immutable history element")
                    .link()
                    .expect("validated immutable Episode Link")
            })
    }
}

fn inspect_continuation(view: ValueView<'_>) -> Result<Continuation, StateError> {
    let map = view.map().map_err(|_| StateError::InvalidSchema)?;
    if !matches!(map.len(), 1 | 2) {
        return Err(StateError::InvalidSchema);
    }
    let mut tag = None;
    for field in map {
        let (key, value) = field?;
        if key == "tag" {
            tag = Some(value.text().map_err(|_| StateError::InvalidSchema)?);
        }
    }
    match tag {
        Some("await_actor") => {
            view.fields(["tag"])
                .map_err(|_| StateError::InvalidSchema)?;
            Ok(Continuation::AwaitActor)
        }
        Some("await_action") => {
            let fields = view
                .fields(["tag", "action_id"])
                .map_err(|_| StateError::InvalidSchema)?;
            let action_id = fields[1]
                .bytes()
                .map_err(|_| StateError::InvalidSchema)?
                .try_into()
                .map_err(|_| StateError::InvalidSchema)?;
            Ok(Continuation::AwaitAction { action_id })
        }
        _ => Err(StateError::InvalidSchema),
    }
}

#[cfg(test)]
mod tests;
