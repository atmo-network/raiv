use super::*;
use crossterm::event::{KeyEvent, KeyEventState};
use ratatui::{backend::TestBackend, Terminal};

#[test]
#[ignore = "owned-PTY witness: run via script -q -e -c with --exact, never ordinary module suite"]
fn owned_tty_session_restores_termios_normal_drop_and_unwind_and_rejects_second_owner() {
    use std::io::IsTerminal;
    assert!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "run only inside explicitly owned PTY"
    );
    fn attrs() -> libc::termios {
        let mut value = std::mem::MaybeUninit::uninit();
        // SAFETY: this witness owns its PTY stdin and passes a valid output termios pointer.
        assert_eq!(
            unsafe { libc::tcgetattr(libc::STDIN_FILENO, value.as_mut_ptr()) },
            0
        );
        // SAFETY: successful tcgetattr initialized the complete output.
        unsafe { value.assume_init() }
    }
    let before = attrs();
    {
        let mut session = TerminalSession::open().unwrap();
        assert!(TerminalSession::open().is_err());
        let mut ui = state();
        session.draw(&mut ui).unwrap();
        assert_eq!(attrs().c_lflag & libc::ICANON, 0);
        let _ = session.read(std::time::Duration::ZERO).unwrap();
    }
    let normal = attrs();
    assert_eq!(normal.c_iflag, before.c_iflag);
    assert_eq!(normal.c_oflag, before.c_oflag);
    assert_eq!(normal.c_lflag, before.c_lflag);
    assert_eq!(normal.c_cc, before.c_cc);
    assert!(std::panic::catch_unwind(|| {
        let _session = TerminalSession::open().unwrap();
        panic!("owned terminal unwind witness");
    })
    .is_err());
    let restored = attrs();
    assert_eq!(restored.c_iflag, before.c_iflag);
    assert_eq!(restored.c_oflag, before.c_oflag);
    assert_eq!(restored.c_lflag, before.c_lflag);
    assert_eq!(restored.c_cc, before.c_cc);
    drop(TerminalSession::open().unwrap());
}

fn state() -> TerminalUi {
    TerminalUi::new(UiLimits {
        prompt_bytes: 64,
        history_bytes: 128,
        history_entries: 4,
    })
    .unwrap()
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn draw(ui: &mut TerminalUi, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| ui.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn utf8_grapheme_editing_multiline_caret_and_atomic_byte_caps() {
    let mut ui = state();
    ui.handle(Event::Paste("e\u{301}🇷🇺👩‍💻中\nabc".into()))
        .unwrap();
    assert_eq!(ui.prompt(), "e\u{301}🇷🇺👩‍💻中\nabc");
    ui.handle(ctrl('a')).unwrap();
    ui.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(ui.cursor, 3);
    ui.handle(key(KeyCode::Delete)).unwrap();
    assert!(!ui.prompt.contains("🇷🇺"));
    ui.handle(key(KeyCode::Right)).unwrap();
    ui.handle(key(KeyCode::Backspace)).unwrap();
    assert!(!ui.prompt.contains("👩‍💻"));
    assert_eq!(ui.prompt(), "e\u{301}中\nabc");
    ui.handle(ctrl('e')).unwrap();
    ui.handle(key(KeyCode::Up)).unwrap();
    assert_eq!(ui.cursor, "e\u{301}中".len());
    ui.handle(key(KeyCode::Home)).unwrap();
    assert_eq!(ui.cursor, 0);
    ui.handle(key(KeyCode::Down)).unwrap();
    assert_eq!(ui.cursor, "e\u{301}中\n".len());
    let before = ui.prompt.clone();
    let cursor = ui.cursor;
    assert_eq!(ui.handle(Event::Paste("x".repeat(65))), Err(UiError::Limit));
    assert_eq!(ui.prompt, before);
    assert_eq!(ui.cursor, cursor);
    ui.handle(ctrl('u')).unwrap();
    assert!(ui.prompt.is_empty());
    ui.handle(ctrl('j')).unwrap();
    assert_eq!(ui.prompt(), "\n");
}

#[test]
fn submit_is_native_presentation_signal_locked_until_caller_result_never_auto_from_paste_or_repeat()
{
    let mut ui = state();
    ui.handle(Event::Paste("one\ntwo".into())).unwrap();
    assert!(!ui.pending);
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::None);
    draw(&mut ui, 20, 8);
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::Submit);
    assert!(ui.is_pending());
    assert_eq!(ui.handle(key(KeyCode::Char('x'))).unwrap(), Signal::None);
    assert_eq!(ui.prompt(), "one\ntwo");
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::None);
    ui.complete_input(false);
    assert_eq!(ui.prompt(), "one\ntwo");
    let repeat = Event::Key(KeyEvent {
        kind: KeyEventKind::Repeat,
        state: KeyEventState::NONE,
        ..KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    });
    assert_eq!(ui.handle(repeat).unwrap(), Signal::None);
    assert_eq!(ui.prompt(), "one\ntwo");
    ui.set_enabled(false);
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::None);
    ui.set_enabled(true);
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::Submit);
    ui.complete_input(true);
    assert!(ui.prompt().is_empty());
    assert_eq!(ui.handle(key(KeyCode::Esc)).unwrap(), Signal::Exit);
}

#[test]
fn bounded_ephemeral_history_evicts_only_presentation_overflow_is_atomic_and_control_data_cannot_execute(
) {
    let mut ui = state();
    for text in ["a", "b", "c", "d", "e"] {
        ui.push_history(Role::Model, text).unwrap();
    }
    assert_eq!(ui.history.len(), 4);
    assert_eq!(ui.history.front().unwrap().text, "b");
    let bytes = ui.history_bytes;
    assert_eq!(
        ui.push_history(Role::Model, &"x".repeat(129)),
        Err(UiError::Limit)
    );
    assert_eq!(ui.history_bytes, bytes);
    assert_eq!(ui.history.len(), 4);
    ui.push_history(Role::Model, "\x1b[2J\r\u{202e}Status:fake\nnot authority")
        .unwrap();
    assert!(!ui.history.back().unwrap().text.contains('\x1b'));
    assert!(!ui.history.back().unwrap().text.contains('\u{202e}'));
    let screen = draw(&mut ui, 40, 12);
    // Model text wraps beside an empty gutter; it can never occupy the label column.
    assert!(screen.contains(" Agent │ "));
    assert!(screen.contains("       │ not authority"));
    assert!(!screen
        .lines()
        .any(|row| row.trim_start_matches(['│', ' ']).starts_with("Status")));
    assert!(!screen.contains('\x1b'));
    assert_eq!(
        ui.history_bytes,
        ui.history.iter().map(|v| v.text.len()).sum::<usize>()
    );
}

#[test]
fn rendered_two_regions_narrow_wide_resize_scroll_and_long_caret_are_truthful() {
    let mut ui = state();
    ui.push_history(Role::Status, "Native admission is NOT implied")
        .unwrap();
    ui.push_history(Role::Operator, "hello").unwrap();
    ui.push_history(Role::Model, "context stays guest-owned")
        .unwrap();
    ui.handle(Event::Paste("abc 中\nnext".into())).unwrap();
    for (w, h) in [(12, 7), (40, 12), (120, 30)] {
        let screen = draw(&mut ui, w, h);
        eprintln!("\n--- TestBackend {w}x{h} ---\n{screen}");
        assert!(screen.contains("Prompt"));
        assert!(screen.contains("Agent"));
        assert!(screen.contains("Raiv"));
        assert!(ui.visible);
        assert_eq!(screen.lines().filter(|row| row.starts_with('┌')).count(), 2);
    }
    draw(&mut ui, 2, 2);
    assert!(!ui.visible);
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::None);
    ui.push_history(
        Role::Model,
        "many long lines here repeated for scrolling"
            .repeat(2)
            .as_str(),
    )
    .unwrap();
    draw(&mut ui, 14, 9);
    ui.handle(key(KeyCode::PageUp)).unwrap();
    assert!(ui.history_scroll > 0);
    let old = draw(&mut ui, 14, 9);
    ui.handle(key(KeyCode::PageDown)).unwrap();
    assert_ne!(old, draw(&mut ui, 14, 9));
    ui.handle(Event::Resize(40, 12)).unwrap();
    draw(&mut ui, 40, 12);
    assert_eq!(ui.prompt(), "abc 中\nnext");
}

#[test]
fn invalid_limits_empty_submit_release_events_and_non_tty_do_not_change_owned_terminal_or_ui() {
    assert!(TerminalUi::new(UiLimits {
        prompt_bytes: 0,
        history_bytes: 1,
        history_entries: 1
    })
    .is_err());
    let mut ui = state();
    draw(&mut ui, 40, 12);
    assert_eq!(ui.handle(key(KeyCode::Enter)).unwrap(), Signal::None);
    let release = Event::Key(KeyEvent {
        kind: KeyEventKind::Release,
        state: KeyEventState::NONE,
        ..KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
    });
    ui.handle(release).unwrap();
    assert!(ui.prompt.is_empty());
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        assert!(TerminalSession::open().is_err());
    }
}

#[test]
fn history_wraps_at_words_and_keeps_a_scrolled_back_reader_in_place() {
    assert_eq!(wrap("alpha beta gamma", 11), ["alpha beta", "gamma"]);
    assert_eq!(wrap("  indented\n\nx", 20), ["  indented", "", "x"]);
    assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
    assert_eq!(wrap("中中中", 4), ["中中", "中"]);
    let mut ui = state();
    ui.push_history(Role::Model, "l1\nl2\nl3\nl4\nl5").unwrap();
    draw(&mut ui, 40, 9);
    ui.handle(key(KeyCode::PageUp)).unwrap();
    let before = draw(&mut ui, 40, 9);
    ui.push_history(Role::Status, "new boundary").unwrap();
    let after = draw(&mut ui, 40, 9);
    assert_eq!(
        before, after,
        "new output must not move a scrolled-back reader"
    );
    ui.handle(key(KeyCode::PageDown)).unwrap();
    assert!(draw(&mut ui, 40, 9).contains("new boundary"));
    ui.push_history(Role::Memory, "{\"intents\":{}}").unwrap();
    assert!(draw(&mut ui, 40, 9).contains("Memory │ {\"intents\":{}}"));
}
