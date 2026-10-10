//! Keys in, bytes out.
//!
//! A client reports a key as a `ShellKey` (a name plus modifier bits) and typed text as a
//! string; the server turns a key into the bytes the program expects with `encode`, which
//! is the one place that knows xterm's legacy encodings and the emulator's current modes.
//! `classify` is the carve-out rule every surface shares: in a shell world a key either
//! switches worlds (the ten switching actions, and nothing else) or goes to the shell.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::keybindings::KeyBindings;
use crate::websocket::{ShellKey, ShellModeMsg};

/// The only Clay actions a key keeps in a shell world: switching between worlds.
pub const WORLD_SWITCH_ACTIONS: [&str; 10] = [
    "world_next",
    "world_prev",
    "world_all_next",
    "world_all_prev",
    "world_activity",
    "world_previous",
    "world_forward",
    "recent_worlds",
    "world_socket_prev",
    "world_socket_next",
];

pub fn is_world_switch(action: &str) -> bool {
    WORLD_SWITCH_ACTIONS.contains(&action)
}

/// What a keystroke means in a live shell world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run this Clay action (a world switch).
    Switch(String),
    /// Send this key to the shell.
    Forward(ShellKey),
    /// A plain printable character: send it as text.
    Text(char),
}

/// A crossterm key event as a `ShellKey`, or None for a bare modifier / unknown code.
pub fn key_from_crossterm(code: KeyCode, mods: KeyModifiers) -> Option<ShellKey> {
    let name = match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Esc => "Escape".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        _ => return None,
    };
    let mut shift = mods.contains(KeyModifiers::SHIFT);
    // crossterm reports Shift-Tab as BackTab, not Tab+SHIFT.
    if matches!(code, KeyCode::BackTab) {
        shift = true;
    }
    Some(ShellKey { key: name, ctrl: mods.contains(KeyModifiers::CONTROL), alt: mods.contains(KeyModifiers::ALT), shift })
}

/// Decide a keystroke in a live shell world. `esc_armed`: a bare Escape is waiting for a
/// possible `Esc-<key>` world-switch chord, so this key may complete one.
pub fn classify(code: KeyCode, mods: KeyModifiers, bindings: &KeyBindings, esc_armed: bool) -> Decision {
    // An armed Escape: does this key complete a world-switch chord (Esc-} etc.)?
    if esc_armed {
        if let Some(name) = crate::keybindings::escape_key_to_name(code, mods) {
            if let Some(action) = bindings.get_action(&name) {
                if is_world_switch(action) {
                    return Decision::Switch(action.to_string());
                }
            }
        }
    }
    // A key bound to a world switch, resolved immediately (Alt-x == Esc-x, no buffering).
    if let Some(name) = crate::keybindings::key_event_to_name(code, mods) {
        if let Some(action) = bindings.get_action(&name) {
            if is_world_switch(action) {
                return Decision::Switch(action.to_string());
            }
        }
    }
    // A plain printable character is text (so an IME and the input path agree).
    if let KeyCode::Char(c) = code {
        if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return Decision::Text(c);
        }
    }
    match key_from_crossterm(code, mods) {
        Some(key) => Decision::Forward(key),
        // Unknown code: a bare modifier or similar - nothing to send.
        None => Decision::Text('\0'),
    }
}

/// The bytes a program expects for `key`, given its current modes.
pub fn encode(key: &ShellKey, mode: &ShellModeMsg) -> Vec<u8> {
    let name = key.key.as_str();
    // A single printable character (possibly with Ctrl/Alt).
    if name.chars().count() == 1 {
        let c = name.chars().next().unwrap();
        return encode_char(c, key, mode);
    }
    // The modifier parameter xterm uses: 1 + bits (shift 1, alt 2, ctrl 4).
    let m = 1 + (key.shift as u8) + 2 * (key.alt as u8) + 4 * (key.ctrl as u8);
    let alt_prefix = |bytes: Vec<u8>| -> Vec<u8> {
        if key.alt && !modified_csi_applies(name) {
            let mut v = vec![0x1b];
            v.extend(bytes);
            v
        } else {
            bytes
        }
    };
    match name {
        "Enter" => alt_prefix(vec![b'\r']),
        "Tab" if key.shift => b"\x1b[Z".to_vec(),
        "Tab" => alt_prefix(vec![b'\t']),
        "Backspace" if key.ctrl => vec![0x08],
        "Backspace" if key.alt => vec![0x1b, 0x7f],
        "Backspace" => vec![0x7f],
        "Escape" => vec![0x1b],
        "Up" => cursor_key(b'A', m, mode.app_cursor),
        "Down" => cursor_key(b'B', m, mode.app_cursor),
        "Right" => cursor_key(b'C', m, mode.app_cursor),
        "Left" => cursor_key(b'D', m, mode.app_cursor),
        "Home" => cursor_key(b'H', m, mode.app_cursor),
        "End" => cursor_key(b'F', m, mode.app_cursor),
        "Insert" => tilde_key(2, m),
        "Delete" => tilde_key(3, m),
        "PageUp" => tilde_key(5, m),
        "PageDown" => tilde_key(6, m),
        "F1" => fkey(b'P', m),
        "F2" => fkey(b'Q', m),
        "F3" => fkey(b'R', m),
        "F4" => fkey(b'S', m),
        "F5" => tilde_key(15, m),
        "F6" => tilde_key(17, m),
        "F7" => tilde_key(18, m),
        "F8" => tilde_key(19, m),
        "F9" => tilde_key(20, m),
        "F10" => tilde_key(21, m),
        "F11" => tilde_key(23, m),
        "F12" => tilde_key(24, m),
        _ => Vec::new(),
    }
}

fn modified_csi_applies(name: &str) -> bool {
    // Keys whose modified form is a CSI with a modifier parameter already carry Alt in
    // that parameter, so no ESC prefix is added for them.
    matches!(
        name,
        "Up" | "Down" | "Left" | "Right" | "Home" | "End" | "Insert" | "Delete" | "PageUp" | "PageDown"
            | "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
    )
}

fn cursor_key(final_byte: u8, m: u8, app_cursor: bool) -> Vec<u8> {
    if m != 1 {
        format!("\x1b[1;{m}{}", final_byte as char).into_bytes()
    } else if app_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde_key(n: u8, m: u8) -> Vec<u8> {
    if m != 1 {
        format!("\x1b[{n};{m}~").into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

fn fkey(ss3: u8, m: u8) -> Vec<u8> {
    if m != 1 {
        format!("\x1b[1;{m}{}", ss3 as char).into_bytes()
    } else {
        vec![0x1b, b'O', ss3]
    }
}

fn encode_char(c: char, key: &ShellKey, _mode: &ShellModeMsg) -> Vec<u8> {
    if key.ctrl {
        let upper = c.to_ascii_uppercase();
        let byte = match upper {
            ' ' | '@' => Some(0x00),
            'A'..='Z' => Some(upper as u8 - b'A' + 1),
            '[' => Some(0x1b),
            '\\' | '4' => Some(0x1c),
            ']' | '5' => Some(0x1d),
            '^' | '6' => Some(0x1e),
            '_' | '7' => Some(0x1f),
            '?' => Some(0x7f),
            _ => None,
        };
        if let Some(b) = byte {
            return if key.alt { vec![0x1b, b] } else { vec![b] };
        }
    }
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf).as_bytes().to_vec();
    if key.alt {
        let mut v = vec![0x1b];
        v.extend(s);
        v
    } else {
        s
    }
}

/// Typed or pasted text as bytes. A paste is bracketed when the program asked for it;
/// otherwise its newlines become carriage returns. A literal end-of-paste marker in the
/// text is always removed (paste injection).
pub fn text_bytes(text: &str, paste: bool, mode: &ShellModeMsg) -> Vec<u8> {
    let cleaned = text.replace("\x1b[201~", "");
    if paste {
        if mode.bracketed_paste {
            let mut v = b"\x1b[200~".to_vec();
            v.extend_from_slice(cleaned.as_bytes());
            v.extend_from_slice(b"\x1b[201~");
            return v;
        }
        return cleaned.replace(['\r', '\n'], "\r").into_bytes();
    }
    cleaned.into_bytes()
}

/// A mouse event as a report, if the program asked for one. `kind`: "down"/"up"/"drag"/
/// "move"/"wheel_up"/"wheel_down"; `button`: 0 left, 1 middle, 2 right. `col`/`row` are
/// zero-based cells. None when the program isn't tracking the mouse (or this kind of it),
/// or when a wheel on the alt screen should instead become arrow keys (`wheel_arrows`).
pub fn mouse(
    kind: &str,
    button: u8,
    col: u16,
    row: u16,
    ctrl: bool,
    alt: bool,
    shift: bool,
    mode: &ShellModeMsg,
) -> Option<Vec<u8>> {
    let wheel = kind == "wheel_up" || kind == "wheel_down";
    if !mode.mouse_click {
        return None;
    }
    if !wheel {
        let moving = kind == "drag" || kind == "move";
        if kind == "move" && !mode.mouse_motion {
            return None;
        }
        if kind == "drag" && !(mode.mouse_drag || mode.mouse_motion) {
            return None;
        }
        let _ = moving;
    }
    // Button code: wheel is 64/65, a release in legacy mode is 3.
    let mut cb: u8 = match kind {
        "wheel_up" => 64,
        "wheel_down" => 65,
        "drag" | "move" => 32 + button.min(2),
        _ => button.min(2),
    };
    if shift {
        cb += 4;
    }
    if alt {
        cb += 8;
    }
    if ctrl {
        cb += 16;
    }
    let release = kind == "up";
    if mode.sgr_mouse {
        let c = col + 1;
        let r = row + 1;
        let last = if release { 'm' } else { 'M' };
        Some(format!("\x1b[<{cb};{c};{r}{last}").into_bytes())
    } else {
        // Legacy X10: a release reports button 3.
        let b = if release { 3 } else { cb };
        let cx = (col + 1).min(223) as u8 + 32;
        let cy = (row + 1).min(223) as u8 + 32;
        Some(vec![0x1b, b'[', b'M', b + 32, cx, cy])
    }
}

/// When the program doesn't track the mouse but the alt screen is up with alternate
/// scroll on, a wheel notch is three arrow keys. Returns the bytes, or None.
pub fn wheel_arrows(kind: &str, mode: &ShellModeMsg) -> Option<Vec<u8>> {
    if mode.mouse_click || !mode.alt_screen || !mode.alternate_scroll {
        return None;
    }
    let final_byte = match kind {
        "wheel_up" => b'A',
        "wheel_down" => b'B',
        _ => return None,
    };
    let one = cursor_key(final_byte, 1, mode.app_cursor);
    Some(one.repeat(3))
}
