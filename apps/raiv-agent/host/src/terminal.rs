//! Native two-region presentation/input port. No inference, admission, pin selection or persistence.

mod session;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
pub use session::TerminalSession;
use std::{collections::VecDeque, fmt};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiError {
    Bounds,
    Limit,
    Allocation,
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bounds => "terminal presentation bounds invalid",
            Self::Limit => "terminal presentation/input bound; submission preserved",
            Self::Allocation => "terminal presentation allocation failed",
        })
    }
}

impl std::error::Error for UiError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Operator,
    Model,
    /// Accepted Memory presentation; never an editable or restored transcript.
    Memory,
    Status,
    Error,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Self::Operator => "You",
            Self::Model => "Agent",
            Self::Memory => "Memory",
            Self::Status => "",
            Self::Error => "Error",
        }
    }

    fn styles(self) -> (Style, Style) {
        let label = Style::default().add_modifier(Modifier::BOLD);
        match self {
            Self::Operator => (label.fg(Color::Cyan), Style::default()),
            Self::Model => (label.fg(Color::Green), Style::default()),
            Self::Memory => (
                label.fg(Color::Magenta),
                Style::default().fg(Color::Magenta),
            ),
            Self::Status => (Style::default(), Style::default().fg(Color::DarkGray)),
            Self::Error => (label.fg(Color::Red), Style::default().fg(Color::Red)),
        }
    }
}

/// Right-aligned role label plus separator; text wraps beside it.
const GUTTER: usize = 9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    None,
    Submit,
    Exit,
}

/// Independent presentation caps, not Core context policy or permission to discard agent DATA.
#[derive(Clone, Copy, Debug)]
pub struct UiLimits {
    pub prompt_bytes: usize,
    pub history_bytes: usize,
    pub history_entries: usize,
}

struct Entry {
    role: Role,
    text: String,
}

pub struct TerminalUi {
    limits: UiLimits,
    prompt: String,
    cursor: usize,
    history: VecDeque<Entry>,
    history_bytes: usize,
    history_scroll: usize,
    rendered_lines: usize,
    pending: bool,
    enabled: bool,
    visible: bool,
}

fn display_text(text: &str) -> Result<String, UiError> {
    let mut result = String::new();
    result
        .try_reserve_exact(text.len())
        .map_err(|_| UiError::Allocation)?;
    for c in text.chars() {
        if c == '\n'
            || (!c.is_control() && !matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        {
            result.push(c);
        } else {
            result.push(' ');
        }
    }
    Ok(result)
}

impl TerminalUi {
    pub fn new(limits: UiLimits) -> Result<Self, UiError> {
        if limits.prompt_bytes == 0 || limits.history_bytes == 0 || limits.history_entries == 0 {
            return Err(UiError::Bounds);
        }
        Ok(Self {
            limits,
            prompt: String::new(),
            cursor: 0,
            history: VecDeque::new(),
            history_bytes: 0,
            history_scroll: 0,
            rendered_lines: 0,
            pending: false,
            enabled: true,
            visible: false,
        })
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Presentation-only completion. Caller must prove native outcome separately.
    /// Refused-before-attempt may retain text; unresolved native attempts must disable further submit.
    pub fn complete_input(&mut self, clear: bool) {
        if clear {
            self.prompt.clear();
            self.cursor = 0;
        }
        self.pending = false;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Bounded RAM-only history; never restored/projected into guest context.
    /// Old presentation entries may be evicted, but inbound over-cap entries fail atomically.
    pub fn push_history(&mut self, role: Role, text: &str) -> Result<(), UiError> {
        if text.len() > self.limits.history_bytes {
            return Err(UiError::Limit);
        }
        let text = display_text(text)?;
        self.history
            .try_reserve(1)
            .map_err(|_| UiError::Allocation)?;
        while self.history.len() >= self.limits.history_entries
            || self.history_bytes > self.limits.history_bytes - text.len()
        {
            let old = self.history.pop_front().expect("bounded nonempty history");
            self.history_bytes -= old.text.len();
        }
        self.history_bytes += text.len();
        self.history.push_back(Entry { role, text });
        Ok(())
    }

    fn boundary_after(&self, cursor: usize) -> usize {
        self.prompt
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.prompt.len()))
            .find(|i| *i >= cursor)
            .unwrap_or(self.prompt.len())
    }

    fn left(&self) -> usize {
        self.prompt[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn right(&self) -> usize {
        self.prompt[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }

    fn insert(&mut self, text: &str) -> Result<(), UiError> {
        if self
            .prompt
            .len()
            .checked_add(text.len())
            .is_none_or(|n| n > self.limits.prompt_bytes)
        {
            return Err(UiError::Limit);
        }
        self.prompt
            .try_reserve(text.len())
            .map_err(|_| UiError::Allocation)?;
        self.prompt.insert_str(self.cursor, text);
        self.cursor = self.boundary_after(self.cursor + text.len());
        Ok(())
    }

    fn vertical(&mut self, down: bool) {
        let line_start = self.prompt[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        let col = self.prompt[line_start..self.cursor].width();
        let (start, end) = if down {
            let Some(next) = self.prompt[self.cursor..]
                .find('\n')
                .map(|i| self.cursor + i + 1)
            else {
                return;
            };
            (
                next,
                self.prompt[next..]
                    .find('\n')
                    .map_or(self.prompt.len(), |i| next + i),
            )
        } else {
            if line_start == 0 {
                return;
            };
            let end = line_start - 1;
            let start = self.prompt[..end].rfind('\n').map_or(0, |i| i + 1);
            (start, end)
        };
        let mut width = 0;
        let mut cursor = start;
        for g in self.prompt[start..end].graphemes(true) {
            if width + g.width() > col {
                break;
            }
            width += g.width();
            cursor += g.len();
        }
        self.cursor = cursor;
    }

    /// Only native terminal Event DATA. Submit signal is not Actor/Receipt approval or acceptance.
    pub fn handle(&mut self, event: Event) -> Result<Signal, UiError> {
        if let Event::Resize(_, _) = event {
            return Ok(Signal::None);
        }
        if let Event::Key(key) = &event {
            if key.kind == KeyEventKind::Release {
                return Ok(Signal::None);
            }
            if key.code == KeyCode::Esc
                || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
            {
                return Ok(Signal::Exit);
            }
            match key.code {
                KeyCode::PageUp => {
                    self.history_scroll = self.history_scroll.saturating_add(5);
                    return Ok(Signal::None);
                }
                KeyCode::PageDown => {
                    self.history_scroll = self.history_scroll.saturating_sub(5);
                    return Ok(Signal::None);
                }
                _ => {}
            }
        }
        if self.pending || !self.enabled {
            return Ok(Signal::None);
        }
        match event {
            Event::Paste(text) => {
                if text.len() > self.limits.prompt_bytes.saturating_sub(self.prompt.len()) {
                    return Err(UiError::Limit);
                }
                let text = display_text(&text)?;
                self.insert(&text)?;
            }
            Event::Key(key) => {
                let control = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Enter
                        if key.kind == KeyEventKind::Press
                            && !key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        if self.visible && !self.prompt.trim().is_empty() {
                            self.pending = true;
                            self.history_scroll = 0;
                            return Ok(Signal::Submit);
                        }
                    }
                    KeyCode::Enter => {
                        if key.kind == KeyEventKind::Press {
                            self.insert("\n")?;
                        }
                    }
                    KeyCode::Char('j') if control => self.insert("\n")?,
                    KeyCode::Char('u') if control => {
                        self.prompt.clear();
                        self.cursor = 0;
                    }
                    KeyCode::Char('a') if control => self.cursor = 0,
                    KeyCode::Char('e') if control => self.cursor = self.prompt.len(),
                    KeyCode::Home => {
                        self.cursor = self.prompt[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
                    }
                    KeyCode::End => {
                        self.cursor = self.prompt[self.cursor..]
                            .find('\n')
                            .map_or(self.prompt.len(), |i| self.cursor + i)
                    }
                    KeyCode::Left => self.cursor = self.left(),
                    KeyCode::Right => self.cursor = self.right(),
                    KeyCode::Up => self.vertical(false),
                    KeyCode::Down => self.vertical(true),
                    KeyCode::Backspace => {
                        let left = self.left();
                        self.prompt.drain(left..self.cursor);
                        self.cursor = self.boundary_after(left);
                    }
                    KeyCode::Delete => {
                        let right = self.right();
                        self.prompt.drain(self.cursor..right);
                        self.cursor = self.boundary_after(self.cursor);
                    }
                    KeyCode::Char(c)
                        if !control
                            && !key.modifiers.contains(KeyModifiers::ALT)
                            && !c.is_control() =>
                    {
                        let mut buf = [0; 4];
                        self.insert(c.encode_utf8(&mut buf))?;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(Signal::None)
    }

    /// Exactly two regions, all transient; no status/header/sidebar overlay or native approval implied.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let input_height = (area.height / 2).min(5);
        let history_height = area.height - input_height;
        let history = Rect::new(area.x, area.y, area.width, history_height);
        let input = Rect::new(area.x, area.y + history_height, area.width, input_height);
        self.visible = area.width >= 8 && area.height >= 6;
        let history_block = Block::default()
            .borders(Borders::ALL)
            .title(" Raiv · conversation shown for this launch only ");
        let input_block = Block::default()
            .borders(Borders::ALL)
            .title(if self.pending {
                " Prompt · waiting for the agent… "
            } else if !self.enabled {
                " Prompt · closed · Esc exit "
            } else {
                " Prompt · Enter send · Ctrl+J newline · PgUp/PgDn scroll · Esc exit "
            });
        let history_inner = history_block.inner(history);
        let input_inner = input_block.inner(input);
        let mut lines = Vec::new();
        // Narrow terminals label every line instead of spending a fixed gutter.
        let compact = (history_inner.width as usize) < 4 * GUTTER;
        for entry in &self.history {
            if entry.role == Role::Operator && !lines.is_empty() {
                lines.push(Line::default());
            }
            let (label_style, text_style) = entry.role.styles();
            let label = entry.role.label();
            let used = if compact { label.len() + 2 } else { GUTTER };
            let width = (history_inner.width as usize).saturating_sub(used);
            for (index, line) in wrap(&entry.text, width).into_iter().enumerate() {
                let gutter = match (entry.role, index) {
                    (Role::Status, _) if compact => "· ".to_owned(),
                    (Role::Status, _) => format!("{:>w$} ", "·", w = GUTTER - 1),
                    _ if compact => format!("{label}: "),
                    (_, 0) => format!("{label:>w$} │ ", w = GUTTER - 3),
                    _ => format!("{:>w$} │ ", "", w = GUTTER - 3),
                };
                lines.push(Line::from(vec![
                    Span::styled(gutter, label_style),
                    Span::styled(line, text_style),
                ]));
            }
        }
        if self.history.is_empty() {
            lines.push(Line::from(
                "No conversation restored; the agent continues from its accepted Memory.",
            ));
        }
        // Follow new output only at the end; otherwise keep the reader's position.
        if self.history_scroll > 0 {
            self.history_scroll += lines.len().saturating_sub(self.rendered_lines);
        }
        self.rendered_lines = lines.len();
        let max_scroll = lines.len().saturating_sub(history_inner.height as usize);
        self.history_scroll = self.history_scroll.min(max_scroll);
        let start = max_scroll - self.history_scroll;
        frame.render_widget(history_block, history);
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(start)
                    .take(history_inner.height as usize)
                    .collect::<Vec<_>>(),
            ),
            history_inner,
        );
        frame.render_widget(input_block, input);
        if !self.visible {
            frame.render_widget(Paragraph::new("Resize terminal"), input_inner);
            return;
        }
        let (lines, row, col) = prompt_lines(&self.prompt, self.cursor, input_inner.width as usize);
        let start = row.saturating_sub(input_inner.height.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(start)
                    .take(input_inner.height as usize)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            input_inner,
        );
        if !self.pending && self.enabled && input_inner.height > 0 && input_inner.width > 0 {
            frame.set_cursor_position(Position::new(
                input_inner.x + col.min(input_inner.width as usize - 1) as u16,
                input_inner.y + (row - start).min(input_inner.height as usize - 1) as u16,
            ));
        }
    }
}

/// Word-aware history wrapping; words wider than the line break by grapheme.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let body = paragraph.trim_start_matches(' ');
        let mut line = " ".repeat((paragraph.len() - body.len()).min(width - 1));
        let mut col = line.len();
        let mut fresh = true;
        for word in body.split(' ') {
            let w = word.width();
            if !fresh && col > 0 && col + 1 + w > width {
                lines.push(std::mem::take(&mut line));
                col = 0;
            } else if !fresh {
                line.push(' ');
                col += 1;
            }
            fresh = false;
            for g in word.graphemes(true) {
                if col > 0 && col + g.width() > width {
                    lines.push(std::mem::take(&mut line));
                    col = 0;
                }
                line.push_str(g);
                col += g.width();
            }
        }
        lines.push(line);
    }
    lines
}

fn prompt_lines(text: &str, cursor: usize, width: usize) -> (Vec<String>, usize, usize) {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let mut row = 0;
    let mut col = 0;
    let mut caret = (0, 0);
    for (offset, g) in text.grapheme_indices(true) {
        if g != "\n" && col + g.width() > width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        if offset == cursor {
            caret = (row, col);
        }
        if g == "\n" {
            lines.push(String::new());
            row += 1;
            col = 0;
        } else {
            lines[row].push_str(g);
            col += g.width();
        }
    }
    if cursor == text.len() {
        if col == width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        caret = (row, col);
    }
    (lines, caret.0, caret.1)
}

#[cfg(test)]
mod tests;
