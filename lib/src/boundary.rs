//! Boundary checkpoint data, not graph closure, tool authorization or accepted HEAD.

use crate::command::{Command, CommandError};
use crate::format::{validate_drisl, CodecError, DaslCid, DaslCodec, RecordEncoder, ValueView};
use crate::limits::Limits;
use crate::usage::Usage;
use alloc::vec::Vec;
use core::{fmt, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryError {
    Codec(CodecError),
    Command(CommandError),
    WrongKind,
    UnsupportedVersion,
    InvalidSchema,
}

impl From<CodecError> for BoundaryError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<CommandError> for BoundaryError {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(f),
            Self::Command(error) => error.fmt(f),
            Self::WrongKind => f.write_str("block is not a Boundary"),
            Self::UnsupportedVersion => f.write_str("unsupported Boundary version"),
            Self::InvalidSchema => f.write_str(
                "Boundary requires exact fields, structural links, exit variant and Usage",
            ),
        }
    }
}

impl core::error::Error for BoundaryError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Command(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ExitSpec<'a> {
    AwaitActor,
    AwaitAction(&'a Command),
}

/// Command bytes are validated and borrowed from the Boundary, not copied or dispatched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exit<'a> {
    AwaitActor,
    AwaitAction(&'a [u8]),
}

#[derive(Clone, Copy, Debug)]
pub struct BoundarySpec<'a> {
    pub ordinal: u64,
    pub input: DaslCid,
    pub memory: Option<DaslCid>,
    pub exit: ExitSpec<'a>,
    pub usage: Usage,
}

/// One immutable structural block with strict link profiles. Link validity does not
/// establish availability, target block kinds, Definition memory profile, contiguous
/// ordinals, action-slot identity or acceptance. Those require typed graph/lifecycle checks.
#[derive(Debug, Eq, PartialEq)]
pub struct Boundary {
    encoded: Vec<u8>,
    ordinal: u64,
    input: DaslCid,
    memory: Option<DaslCid>,
    command: Option<Range<usize>>,
    usage: Usage,
}

impl Boundary {
    /// Encode a bounded checkpoint record, not accepted authority or a dispatch request.
    ///
    /// ```
    /// use raiv::boundary::{Boundary, BoundarySpec, ExitSpec, Exit};
    /// use raiv::format::{DaslCid, DaslCodec};
    /// use raiv::limits::Limits;
    /// use raiv::usage::Usage;
    /// let spec = BoundarySpec { ordinal: 0,
    ///     input: DaslCid::from_digest(DaslCodec::Drisl, [1; 32]), memory: None,
    ///     exit: ExitSpec::AwaitActor, usage: Usage { model_calls: 0, local_tool_calls: 0,
    ///         input_tokens: None, output_tokens: None, billed_units: None } };
    /// let boundary = Boundary::encode(&spec, &Limits::default())?;
    /// assert_eq!(boundary.exit(), Exit::AwaitActor);
    /// # Ok::<(), raiv::boundary::BoundaryError>(())
    /// ```
    pub fn encode(spec: &BoundarySpec<'_>, limits: &Limits) -> Result<Self, BoundaryError> {
        let bounds = limits.decode_limits();
        if bounds.max_depth < 3 {
            return Err(CodecError::DepthLimit.into());
        }
        spec.input
            .require_state()
            .map_err(|_| CodecError::InvalidLink)?;
        if let Some(memory) = spec.memory {
            memory
                .require_state()
                .map_err(|_| CodecError::InvalidLink)?;
        }
        if let ExitSpec::AwaitAction(command) = spec.exit {
            Command::validate_encoded(command.as_bytes(), &command_limits(limits))?;
        }
        let mut encoder = RecordEncoder::new(&bounds)?;
        encoder.map(7)?;
        encoder.text("exit")?;
        match spec.exit {
            ExitSpec::AwaitActor => {
                encoder.map(1)?;
                encoder.text("tag")?;
                encoder.text("await_actor")?;
            }
            ExitSpec::AwaitAction(command) => {
                encoder.map(2)?;
                encoder.text("tag")?;
                encoder.text("await_action")?;
                encoder.text("command")?;
                encoder.encoded(command.as_bytes())?;
            }
        }
        encoder.text("kind")?;
        encoder.text("boundary")?;
        encoder.text("input")?;
        encoder.link(spec.input)?;
        encoder.text("usage")?;
        spec.usage.write(&mut encoder)?;
        encoder.text("memory")?;
        match spec.memory {
            Some(memory) => encoder.link(memory)?,
            None => encoder.null()?,
        }
        encoder.text("ordinal")?;
        encoder.unsigned(spec.ordinal)?;
        encoder.text("version")?;
        encoder.unsigned(1)?;
        let encoded = encoder.finish();
        let mut boundary = Self::inspect(&encoded, limits)?;
        boundary.encoded = encoded;
        Ok(boundary)
    }

    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Self, BoundaryError> {
        let mut boundary = Self::inspect(bytes, limits)?;
        boundary.encoded = crate::format::copy_validated(bytes)?;
        Ok(boundary)
    }

    fn inspect(bytes: &[u8], limits: &Limits) -> Result<Self, BoundaryError> {
        validate_drisl(bytes, &limits.decode_limits())?;
        let fields = ValueView::preflighted(bytes)
            .fields([
                "kind", "version", "ordinal", "input", "memory", "exit", "usage",
            ])
            .map_err(|_| BoundaryError::InvalidSchema)?;
        if fields[0].text().map_err(|_| BoundaryError::InvalidSchema)? != "boundary" {
            return Err(BoundaryError::WrongKind);
        }
        if fields[1]
            .unsigned()
            .map_err(|_| BoundaryError::InvalidSchema)?
            != 1
        {
            return Err(BoundaryError::UnsupportedVersion);
        }
        let ordinal = fields[2]
            .unsigned()
            .map_err(|_| BoundaryError::InvalidSchema)?;
        let input = fields[3].link().map_err(|_| BoundaryError::InvalidSchema)?;
        let memory = if fields[4].is_null() {
            None
        } else {
            Some(fields[4].link().map_err(|_| BoundaryError::InvalidSchema)?)
        };
        let exit = fields[5].map().map_err(|_| BoundaryError::InvalidSchema)?;
        if !matches!(exit.len(), 1 | 2) {
            return Err(BoundaryError::InvalidSchema);
        }
        let mut tag = None;
        for field in exit {
            let (key, value) = field?;
            if key == "tag" {
                tag = Some(value.text().map_err(|_| BoundaryError::InvalidSchema)?);
            }
        }
        let command = match tag {
            Some("await_actor") => {
                fields[5]
                    .fields(["tag"])
                    .map_err(|_| BoundaryError::InvalidSchema)?;
                None
            }
            Some("await_action") => {
                let exit = fields[5]
                    .fields(["tag", "command"])
                    .map_err(|_| BoundaryError::InvalidSchema)?;
                Command::validate_encoded(exit[1].encoded(), &command_limits(limits))?;
                Some(exit[1].span())
            }
            _ => return Err(BoundaryError::InvalidSchema),
        };
        let usage = Usage::from_view(fields[6]).map_err(|_| BoundaryError::InvalidSchema)?;
        Ok(Self {
            encoded: Vec::new(),
            ordinal,
            input,
            memory,
            command,
            usage,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    pub fn cid(&self) -> DaslCid {
        DaslCid::for_block(DaslCodec::Drisl, self.as_bytes())
    }

    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }

    pub fn input_link(&self) -> DaslCid {
        self.input
    }

    pub fn memory_link(&self) -> Option<DaslCid> {
        self.memory
    }

    pub fn exit(&self) -> Exit<'_> {
        match &self.command {
            Some(span) => Exit::AwaitAction(&self.encoded[span.clone()]),
            None => Exit::AwaitActor,
        }
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }
}

fn command_limits(limits: &Limits) -> Limits {
    Limits {
        max_depth: limits.max_depth.saturating_sub(2),
        ..*limits
    }
}

#[cfg(test)]
mod tests;
