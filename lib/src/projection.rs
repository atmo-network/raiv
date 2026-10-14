//! Owns retained Episode/action views and transient history-fragment encoding, not activation.
//! Views borrow validated checkpoint data; encoding is model data, not authority or a checkpoint.

mod context;
mod encoding;
pub use context::InitialProjection;
pub use encoding::ProjectionError;

use crate::{
    boundary::Exit, command::CommandView, episode::Episode, format::Id32, graph::RestoredAgent,
    memory::Memory, receipt::ReceiptView,
};

/// Deterministic checkpoint history/current-chain view, independent of provider transcripts.
/// Definition history zero hides completed Episodes only; mandatory retained bytes and
/// the current chain remain intact. This is not a complete ModelRequest: policy/tools,
/// new prepared input, live working memory, RAM trajectory and combined context-byte
/// accounting belong to the later context/activation composition.
#[derive(Clone, Copy)]
pub struct HistoryProjection<'a> {
    restored: &'a RestoredAgent,
}
impl<'a> HistoryProjection<'a> {
    pub fn new(restored: &'a RestoredAgent) -> Self {
        Self { restored }
    }

    /// Completed Episodes, oldest to newest, without shrinking the promised window.
    pub fn completed(self) -> impl Iterator<Item = EpisodeProjection<'a>> {
        let enabled = self.restored.definition().limits().history_episodes != 0;
        self.restored
            .history()
            .filter(move |_| enabled)
            .map(move |episode| EpisodeProjection {
                restored: self.restored,
                episode,
            })
    }

    /// Open checkpoint chain, if any. A final pending Command has no recorded Receipt yet.
    /// This does not invent a result or join an unadmitted new ActivationInput.
    pub fn current(self) -> Option<EpisodeProjection<'a>> {
        self.restored
            .current_episode()
            .map(|episode| EpisodeProjection {
                restored: self.restored,
                episode,
            })
    }
}

/// One retained Episode's entry/latest memory and ordered commands/terminal results.
#[derive(Clone, Copy)]
pub struct EpisodeProjection<'a> {
    restored: &'a RestoredAgent,
    episode: &'a Episode,
}
impl<'a> EpisodeProjection<'a> {
    pub fn id(self) -> Id32 {
        self.episode.id()
    }
    pub fn closed(self) -> bool {
        self.episode.closed()
    }
    pub fn entry_memory(self) -> Option<&'a Memory> {
        self.episode.entry_memory_link().map(|cid| {
            self.restored
                .memory_block(cid)
                .expect("restored entry Memory")
        })
    }
    /// Final memory for a completed Episode; latest checkpoint memory for an open one.
    /// A memory-disabled profile returns None, never synthetic empty planes.
    pub fn latest_memory(self) -> Option<&'a Memory> {
        let cid = self
            .episode
            .boundaries()
            .last()
            .expect("restored nonempty Episode");
        self.restored
            .boundary(cid)
            .expect("restored Boundary")
            .memory_link()
            .map(|cid| {
                self.restored
                    .memory_block(cid)
                    .expect("restored Boundary Memory")
            })
    }
    pub fn actions(self) -> impl Iterator<Item = ActionProjection<'a>> {
        let mut boundaries = self.episode.boundaries().peekable();
        core::iter::from_fn(move || loop {
            let cid = boundaries.next()?;
            let boundary = self.restored.boundary(cid).expect("restored Boundary");
            let Exit::AwaitAction(command) = boundary.exit() else {
                continue;
            };
            let receipt = boundaries.peek().map(|next| {
                let next = self
                    .restored
                    .boundary(*next)
                    .expect("restored next Boundary");
                self.restored
                    .input(next.input_link())
                    .expect("restored next input")
                    .receipt_bytes()
                    .expect("restored action-result Receipt")
            });
            return Some(ActionProjection { command, receipt });
        })
    }
}

/// Exact validated embedded records, borrowed from the Command Boundary and following
/// ActivationInput. The final no_op is not another action; its input still supplies
/// the previous Command's Receipt. Closed Episodes have Some receipt for every action.
/// Raw records retain runtime metadata; model-facing normalization is a separate step.
#[derive(Clone, Copy)]
pub struct ActionProjection<'a> {
    command: &'a [u8],
    receipt: Option<&'a [u8]>,
}
impl<'a> ActionProjection<'a> {
    /// Borrow schema-owned typed fields without copying or revalidating the retained payload.
    pub fn command(self) -> CommandView<'a> {
        CommandView::from_validated(self.command)
    }
    pub fn receipt(self) -> Option<ReceiptView<'a>> {
        self.receipt.map(ReceiptView::from_validated)
    }

    pub fn command_bytes(self) -> &'a [u8] {
        self.command
    }
    pub fn receipt_bytes(self) -> Option<&'a [u8]> {
        self.receipt
    }
}
