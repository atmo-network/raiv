//! Episode block data and ordered references, not typed graph restoration or lifecycle history.

use crate::format::{
    validate_drisl, CodecError, DaslCid, DaslCodec, DecodeLimits, Id32, RecordEncoder, ValueView,
};
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpisodeError {
    Codec(CodecError),
    WrongKind,
    UnsupportedVersion,
    InvalidSchema,
}

impl From<CodecError> for EpisodeError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for EpisodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::WrongKind => f.write_str("block is not an Episode"),
            Self::UnsupportedVersion => f.write_str("unsupported Episode version"),
            Self::InvalidSchema => f.write_str("Episode requires exact raw IDs, nullable memory Link, ordered Boundary Links and boolean closed state"),
        }
    }
}

impl core::error::Error for EpisodeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EpisodeSpec<'a> {
    pub id: Id32,
    pub trigger_event_id: Id32,
    pub entry_memory: Option<DaslCid>,
    pub boundaries: &'a [DaslCid],
    pub closed: bool,
}

/// Immutable structural block. Reference order is retained, never sorted or deduplicated.
/// The codec does not prove episode-ID derivation, target block kinds, nonempty accepted
/// history, ordinal continuity, final-exit/closed consistency or Definition memory policy.
#[derive(Debug, Eq, PartialEq)]
pub struct Episode {
    encoded: Vec<u8>,
    id: Id32,
    trigger_event_id: Id32,
    entry_memory: Option<DaslCid>,
    boundaries: Range<usize>,
    boundary_count: u64,
    closed: bool,
}

impl Episode {
    /// Encode bounded data, not an accepted or graph-consistent episode.
    ///
    /// ```
    /// use raiv::episode::{Episode, EpisodeSpec};
    /// use raiv::format::{DaslCid, DaslCodec, DecodeLimits};
    /// let links = [DaslCid::from_digest(DaslCodec::Drisl, [3; 32])];
    /// let spec = EpisodeSpec { id: [1; 32], trigger_event_id: [2; 32],
    ///     entry_memory: None, boundaries: &links, closed: false };
    /// let episode = Episode::encode(&spec, &DecodeLimits::default())?;
    /// assert_eq!(episode.boundary_count(), 1);
    /// assert_eq!(episode.boundaries().next(), Some(links[0]));
    /// # Ok::<(), raiv::episode::EpisodeError>(())
    /// ```
    pub fn encode(spec: &EpisodeSpec<'_>, bounds: &DecodeLimits) -> Result<Self, EpisodeError> {
        let depth = if !spec.boundaries.is_empty() {
            4
        } else if spec.entry_memory.is_some() {
            3
        } else {
            2
        };
        if bounds.max_depth < depth {
            return Err(CodecError::DepthLimit.into());
        }
        if spec.boundaries.len() as u64 > bounds.max_collection_entries {
            return Err(CodecError::CollectionLimit.into());
        }
        if let Some(memory) = spec.entry_memory {
            memory
                .require_state()
                .map_err(|_| CodecError::InvalidLink)?;
        }
        for boundary in spec.boundaries {
            boundary
                .require_state()
                .map_err(|_| CodecError::InvalidLink)?;
        }
        let mut encoder = RecordEncoder::new(bounds)?;
        encoder.map(7)?;
        encoder.text("id")?;
        encoder.bytes(&spec.id)?;
        encoder.text("kind")?;
        encoder.text("episode")?;
        encoder.text("closed")?;
        encoder.boolean(spec.closed)?;
        encoder.text("version")?;
        encoder.unsigned(1)?;
        encoder.text("boundaries")?;
        encoder.list(spec.boundaries.len() as u64)?;
        for boundary in spec.boundaries {
            encoder.link(*boundary)?;
        }
        encoder.text("entry_memory")?;
        match spec.entry_memory {
            Some(memory) => encoder.link(memory)?,
            None => encoder.null()?,
        }
        encoder.text("trigger_event_id")?;
        encoder.bytes(&spec.trigger_event_id)?;
        let encoded = encoder.finish();
        let mut episode = Self::inspect(&encoded, bounds)?;
        episode.encoded = encoded;
        Ok(episode)
    }

    pub fn decode(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, EpisodeError> {
        let mut episode = Self::inspect(bytes, bounds)?;
        episode.encoded = crate::format::copy_validated(bytes)?;
        Ok(episode)
    }

    fn inspect(bytes: &[u8], bounds: &DecodeLimits) -> Result<Self, EpisodeError> {
        validate_drisl(bytes, bounds)?;
        let fields = ValueView::preflighted(bytes)
            .fields([
                "kind",
                "version",
                "id",
                "trigger_event_id",
                "entry_memory",
                "boundaries",
                "closed",
            ])
            .map_err(|_| EpisodeError::InvalidSchema)?;
        if fields[0].text().map_err(|_| EpisodeError::InvalidSchema)? != "episode" {
            return Err(EpisodeError::WrongKind);
        }
        if fields[1]
            .unsigned()
            .map_err(|_| EpisodeError::InvalidSchema)?
            != 1
        {
            return Err(EpisodeError::UnsupportedVersion);
        }
        let id = fields[2]
            .bytes()
            .map_err(|_| EpisodeError::InvalidSchema)?
            .try_into()
            .map_err(|_| EpisodeError::InvalidSchema)?;
        let trigger_event_id = fields[3]
            .bytes()
            .map_err(|_| EpisodeError::InvalidSchema)?
            .try_into()
            .map_err(|_| EpisodeError::InvalidSchema)?;
        let entry_memory = if fields[4].is_null() {
            None
        } else {
            Some(fields[4].link().map_err(|_| EpisodeError::InvalidSchema)?)
        };
        let boundaries = fields[5].list().map_err(|_| EpisodeError::InvalidSchema)?;
        let boundary_count = boundaries.len();
        for boundary in boundaries {
            boundary?.link().map_err(|_| EpisodeError::InvalidSchema)?;
        }
        let closed = fields[6]
            .boolean()
            .map_err(|_| EpisodeError::InvalidSchema)?;
        Ok(Self {
            encoded: Vec::new(),
            id,
            trigger_event_id,
            entry_memory,
            boundaries: fields[5].span(),
            boundary_count,
            closed,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    pub fn cid(&self) -> DaslCid {
        DaslCid::for_block(DaslCodec::Drisl, self.as_bytes())
    }

    pub fn id(&self) -> Id32 {
        self.id
    }

    pub fn trigger_event_id(&self) -> Id32 {
        self.trigger_event_id
    }

    pub fn entry_memory_link(&self) -> Option<DaslCid> {
        self.entry_memory
    }

    pub fn boundary_count(&self) -> u64 {
        self.boundary_count
    }

    /// An infallible iterator over the already validated immutable list; no second CID vector.
    pub fn boundaries(&self) -> impl Iterator<Item = DaslCid> + '_ {
        ValueView::preflighted(&self.encoded[self.boundaries.clone()])
            .list()
            .expect("validated immutable Episode list")
            .map(|value| {
                value
                    .expect("validated immutable list element")
                    .link()
                    .expect("validated immutable Boundary Link")
            })
    }

    pub fn closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests;
