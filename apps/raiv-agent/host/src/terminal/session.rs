//! Native terminal lifecycle only. Caller owns event admission, grants, pins and live driver.
use super::TerminalUi;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, IsTerminal, Stdout},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
static OWNED: AtomicBool = AtomicBool::new(false);
pub struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _restore: Restore,
}
struct Restore {
    raw: bool,
}
impl Drop for Restore {
    fn drop(&mut self) {
        if self.raw {
            let _ = execute!(
                io::stdout(),
                DisableBracketedPaste,
                Show,
                LeaveAlternateScreen
            );
            let _ = disable_raw_mode();
        }
        OWNED.store(false, Ordering::Release);
    }
}
impl TerminalSession {
    /// Rejects non-TTY stdin/stdout before changing terminal state. No guest/provider/FS/config access.
    pub fn open() -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "interactive terminal requires TTY stdin/stdout",
            ));
        }
        if OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "terminal session already owned",
            ));
        }
        let mut restore = Restore { raw: false };
        enable_raw_mode()?;
        restore.raw = true;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        )?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self {
            terminal,
            _restore: restore,
        })
    }
    pub fn draw(&mut self, ui: &mut TerminalUi) -> io::Result<()> {
        self.terminal.draw(|frame| ui.render(frame)).map(|_| ())
    }
    pub fn read(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    }
}
