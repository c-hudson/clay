//! A GPUI keystroke as the console's key event, so the console's own code names it
//! (`keybindings::key_event_to_token`, `chords`) and every client agrees on what a key is
//! called.

use crossterm::event::{KeyCode, KeyModifiers};

/// `None` for a keystroke Clay never binds: Cmd/Win combinations stay the system's (copy,
/// paste, quit), and bare modifier presses.
pub fn console_key(ks: &gpui::Keystroke) -> Option<(KeyCode, KeyModifiers)> {
    let m = &ks.modifiers;
    if m.platform {
        return None;
    }
    let mut mods = KeyModifiers::NONE;
    if m.control {
        mods |= KeyModifiers::CONTROL;
    }
    if m.alt {
        mods |= KeyModifiers::ALT;
    }
    if m.shift {
        mods |= KeyModifiers::SHIFT;
    }
    let code = match ks.key.as_str() {
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "enter" => KeyCode::Enter,
        "tab" if m.shift => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "space" => KeyCode::Char(' '),
        k if k.len() > 1 && k.starts_with('f') && k[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            KeyCode::F(k[1..].parse().expect("checked"))
        }
        k if k.chars().count() == 1 => {
            let c = k.chars().next().expect("one char");
            // With Ctrl or Alt the base key is the name (Alt-j is Esc-j, and Option-j on
            // a Mac would otherwise type ∆); otherwise the character as typed, so a
            // shifted one keeps its case (Esc-J is not Esc-j).
            let ch = if m.control || m.alt {
                if m.shift { c.to_ascii_uppercase() } else { c }
            } else {
                ks.key_char.as_deref().and_then(|s| s.chars().next()).unwrap_or(c)
            };
            if !(m.control || m.alt) {
                mods.remove(KeyModifiers::SHIFT);
            }
            KeyCode::Char(ch)
        }
        _ => return None,
    };
    Some((code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> gpui::Keystroke {
        gpui::Keystroke::parse(s).unwrap()
    }

    #[test]
    fn keystrokes_name_like_the_console() {
        let name = |s: &str| console_key(&ks(s)).and_then(|(c, m)| crate::keybindings::key_event_to_name(c, m));
        assert_eq!(name("pageup").as_deref(), Some("PageUp"));
        assert_eq!(name("shift-up").as_deref(), Some("Shift-Up"));
        assert_eq!(name("ctrl-l").as_deref(), Some("^L"));
        assert_eq!(name("alt-w").as_deref(), Some("Esc-w"));
        assert_eq!(name("f2").as_deref(), Some("F2"));
        assert_eq!(name("ctrl-pagedown").as_deref(), Some("Ctrl-PageDown"));
        assert_eq!(name("a"), None, "a plain letter is typed input");
        assert_eq!(console_key(&ks("cmd-c")), None, "Cmd/Win combinations stay the system's");
    }
}
