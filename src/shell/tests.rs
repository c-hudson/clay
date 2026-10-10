//! Shell worlds: encoder, screen rendering, activity, and a real-PTY round trip.

use crate::shell::keys::{encode, mouse, text_bytes, wheel_arrows};
use crate::websocket::{ShellKey, ShellModeMsg};

fn key(name: &str) -> ShellKey {
    ShellKey { key: name.to_string(), ctrl: false, alt: false, shift: false }
}

#[test]
fn plain_and_control_characters() {
    let m = ShellModeMsg::default();
    assert_eq!(encode(&key("a"), &m), b"a");
    assert_eq!(encode(&ShellKey { key: "c".into(), ctrl: true, ..Default::default() }, &m), vec![0x03]);
    // Ctrl+Space and Ctrl+@ are NUL.
    assert_eq!(encode(&ShellKey { key: " ".into(), ctrl: true, ..Default::default() }, &m), vec![0x00]);
    // Ctrl+4..7 are the file/group separators (how crossterm delivers ^\ ^] ^^ ^_).
    assert_eq!(encode(&ShellKey { key: "4".into(), ctrl: true, ..Default::default() }, &m), vec![0x1c]);
    assert_eq!(encode(&ShellKey { key: "7".into(), ctrl: true, ..Default::default() }, &m), vec![0x1f]);
    // Alt prefixes with ESC.
    assert_eq!(encode(&ShellKey { key: "x".into(), alt: true, ..Default::default() }, &m), vec![0x1b, b'x']);
}

#[test]
fn named_keys_and_cursor_mode() {
    let mut m = ShellModeMsg::default();
    assert_eq!(encode(&key("Enter"), &m), b"\r");
    assert_eq!(encode(&key("Backspace"), &m), vec![0x7f]);
    assert_eq!(encode(&ShellKey { key: "Backspace".into(), ctrl: true, ..Default::default() }, &m), vec![0x08]);
    assert_eq!(encode(&ShellKey { key: "Tab".into(), shift: true, ..Default::default() }, &m), b"\x1b[Z");
    assert_eq!(encode(&key("Up"), &m), b"\x1b[A");
    m.app_cursor = true;
    assert_eq!(encode(&key("Up"), &m), b"\x1bOA");
    // A modifier turns the arrow into a CSI with a parameter, app-cursor or not.
    assert_eq!(encode(&ShellKey { key: "Up".into(), ctrl: true, ..Default::default() }, &m), b"\x1b[1;5A");
    assert_eq!(encode(&key("F1"), &m), b"\x1bOP");
    assert_eq!(encode(&key("F5"), &m), b"\x1b[15~");
    assert_eq!(encode(&key("Delete"), &m), b"\x1b[3~");
}

#[test]
fn paste_bracketing_and_injection() {
    let mut m = ShellModeMsg::default();
    // Without bracketed paste, newlines become carriage returns.
    assert_eq!(text_bytes("a\nb", true, &m), b"a\rb");
    m.bracketed_paste = true;
    assert_eq!(text_bytes("ab", true, &m), b"\x1b[200~ab\x1b[201~");
    // An end-of-paste marker inside the text can't smuggle out of the bracket.
    assert_eq!(text_bytes("a\x1b[201~b", true, &m), b"\x1b[200~ab\x1b[201~");
    // Non-paste text goes as-is.
    assert_eq!(text_bytes("hi", false, &m), b"hi");
}

#[test]
fn mouse_only_when_tracked() {
    let mut m = ShellModeMsg::default();
    assert_eq!(mouse("down", 0, 3, 4, false, false, false, &m), None);
    m.mouse_click = true;
    m.sgr_mouse = true;
    // SGR report: button 0, 1-based column/row, 'M' for press.
    assert_eq!(mouse("down", 0, 3, 4, false, false, false, &m), Some(b"\x1b[<0;4;5M".to_vec()));
    assert_eq!(mouse("up", 0, 3, 4, false, false, false, &m), Some(b"\x1b[<0;4;5m".to_vec()));
    // A wheel with no tracking but an alt screen + alternate scroll becomes three arrows.
    let mut alt = ShellModeMsg { alt_screen: true, alternate_scroll: true, ..Default::default() };
    assert_eq!(mouse("wheel_up", 0, 0, 0, false, false, false, &alt), None);
    assert_eq!(wheel_arrows("wheel_up", &alt), Some(b"\x1b[A\x1b[A\x1b[A".to_vec()));
    alt.app_cursor = true;
    assert_eq!(wheel_arrows("wheel_down", &alt), Some(b"\x1bOB\x1bOB\x1bOB".to_vec()));
}

/// A screen rendered from bytes fed to a mock terminal, so the row text and the activity
/// rule can be checked without a PTY.
#[cfg(unix)]
mod rendered {
    use crate::shell::screen::{render_rows, ShellScreen};
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    fn term_with(bytes: &[&[u8]]) -> (Term<VoidListener>, ShellScreen) {
        let size = TermSize::new(20, 5);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        let mut screen = ShellScreen::new(20, 5);
        for chunk in bytes {
            parser.advance(&mut term, chunk);
        }
        let snap = render_rows(&term);
        screen.apply(snap);
        (term, screen)
    }

    #[test]
    fn text_and_colour_survive() {
        let (_t, screen) = term_with(&[b"\x1b[31mred\x1b[0m plain"]);
        assert!(screen.lines[0].contains("red"), "row: {:?}", screen.lines[0]);
        assert!(screen.lines[0].contains("\x1b[") && screen.lines[0].contains("31"), "has colour: {:?}", screen.lines[0]);
    }

    #[test]
    fn activity_only_on_new_printable() {
        let size = TermSize::new(20, 5);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        let mut screen = ShellScreen::new(20, 5);
        parser.advance(&mut term, b"hello");
        let first = screen.apply(render_rows(&term));
        assert!(first.gained_printable, "first text is activity");
        // Move the cursor around: no new characters, no activity.
        parser.advance(&mut term, b"\x1b[H\x1b[5C");
        let moved = screen.apply(render_rows(&term));
        assert!(!moved.gained_printable, "cursor movement is not activity");
        // Clear the line: characters removed, not gained.
        parser.advance(&mut term, b"\x1b[2K");
        let cleared = screen.apply(render_rows(&term));
        assert!(!cleared.gained_printable, "clearing is not activity");
        // New text: activity again.
        parser.advance(&mut term, b"\x1b[Hworld");
        let more = screen.apply(render_rows(&term));
        assert!(more.gained_printable, "new text is activity");
    }

    #[test]
    fn delta_then_full_frame_shape() {
        let (_t, mut screen) = term_with(&[b"line one"]);
        // full_msg does not advance the frame; delta_msg does.
        let before = screen.frame;
        let _full = screen.full_msg(0);
        assert_eq!(screen.frame, before, "a full frame does not advance");
        let _delta = screen.delta_msg(0);
        assert_eq!(screen.frame, before + 1, "a delta advances the frame");
        assert!(!screen.has_changes(), "a delta clears the changed flag");
    }
}

/// A real PTY: start a short-lived shell, see its output and its exit code, with Clay's
/// own child reaper running on the side (the regression that spun the event loop).
#[cfg(unix)]
#[test]
fn real_pty_runs_and_exits() {
    use crate::shell::{ShellSession, Size};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::AppEvent>(100);
        let session = ShellSession::start("test", 1, Some("printf 'hi-there\\n'; exit 3"), Size { cols: 40, rows: 10 }, tx).unwrap();

        // A reaper hammering waitpid(0) the way the event loop does.
        let stop = Arc::new(AtomicBool::new(false));
        let reaper = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    crate::platform::reap_zombie_children();
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };

        let mut exit_code = None;
        let mut saw_text = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(crate::AppEvent::Shell(_, _, ev))) => match ev {
                    crate::shell::ShellEvent::Wakeup => {
                        let snap = session.with_term(crate::shell::screen::render_rows);
                        if snap.lines.iter().any(|l| l.contains("hi-there")) {
                            saw_text = true;
                        }
                    }
                    crate::shell::ShellEvent::Exited(code) => {
                        exit_code = Some(code);
                        break;
                    }
                    _ => {}
                },
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {}
            }
        }
        stop.store(true, Ordering::Relaxed);
        let _ = reaper.join();
        assert!(saw_text, "the shell's output reached the screen");
        assert_eq!(exit_code, Some(Some(3)), "the shell's exit code was reported");
    });
}
