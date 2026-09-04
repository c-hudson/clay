//! One canonical key-name grammar, shared by `keybindings.dat`, `/bind`,
//! `/def -b`/`-B`, `key_event_to_name`/`escape_key_to_name`
//! (`keybindings.rs`), and (via the wire) the web/GUI keybind editor.
//!
//! TinyFugue-parity plan, finding A / Phase 2 step P2.1. Before this module
//! existed, `keybindings.rs::key_event_to_name` produced one string format,
//! `tf::hooks::parse_key_name` produced a DIFFERENT, incompatible one for the
//! same logical key (`Esc-j` vs `Alt-J`, always upper-cased), and
//! `input_handler::canonical_to_tf_key_name` tried to paper over the gap with
//! a lossy one-way translation. That translation lost case (`Alt-j`/`Alt-J`
//! collided) and never understood chords (`^X^R`) or TF's raw escape-byte
//! spellings (`^[[A`) at all - see the plan's finding A for the bug list.
//! Every caller now goes through [`parse_key_name`] and [`KeySeq::canonical`]
//! instead, so there is exactly one notion of what a key name looks like.
//!
//! # Grammar
//!
//! A key name is a sequence of one or more whitespace-free **tokens**
//! ([`KeyToken`]), written back to back with no separator (a "chord" like
//! `^X^R` is just two tokens in a row). Each token is one of:
//!
//! - `^<char>` - a control character ([`KeyToken::Ctrl`]). Canonical casing
//!   upper-cases a letter (`^a` -> `^A`); non-letters pass through as-is
//!   (`^?` = DEL/Backspace, `^I` = Tab, `^M`/`^J` = Enter, `^@`, `^]`, ...).
//!   `^[` is never a plain `Ctrl('[')` token - it always introduces the next
//!   rule instead (see raw forms below).
//! - a named key ([`KeyToken::Named`]): `Up Down Left Right PageUp PageDown
//!   Home End Insert Delete Backspace Tab Enter Escape Space F1..F20`.
//!   Case-insensitive on input, canonical casing on output; aliases `PgUp
//!   PgDn Ins Del BS Return Esc` (and `Cr`, kept for backward compatibility)
//!   fold to their canonical spelling.
//! - a modified named key ([`KeyToken::Modified`]): `Ctrl-Up`, `Shift-Up`,
//!   `Alt-Up`, `Ctrl-Left`, ... - a REAL terminal modifier bit on a special
//!   key, distinct from the Esc-prefix chord below (`Alt-Up` is one atomic
//!   keypress; `Esc-Up` is Escape then Up as two sequential keystrokes).
//! - `Esc-<token>` ([`KeyToken::Esc`]): Escape, then one more token,
//!   recursively - `Esc-b`, `Esc-J` (case is significant here: `Esc-j` !=
//!   `Esc-J`), `Esc-^N`, `Esc-Left`, `Esc-{`, `Esc-Tab`, `Esc-Backspace`,
//!   `Esc-Space`, `Esc-0`. `Ctrl-<letter>`/`^X`, `Alt-x`/`Meta-x`/`@x` (all
//!   three preserving case) are accepted input spellings for the same
//!   `^X`/`Esc-x` canonical forms - `Alt-<NamedKey>`/`Meta-<NamedKey>` are the
//!   exception, which normalise to [`KeyToken::Modified`] instead, matching
//!   the real terminal-modifier case above.
//! - a single printable character ([`KeyToken::Char`]), case-sensitive.
//!
//! # TF raw forms
//!
//! [`parse_key_name`] also accepts (and normalises) TinyFugue's own raw
//! spellings, straight off the wire from a terminal:
//!
//! - `^[` as a literal two-character prefix means "the Esc byte, 0x1B" -
//!   either the start of a known multi-byte special-key sequence (below), or
//!   an Esc-prefix chord over whatever follows it.
//! - The special-key sequences xterm/vt100-family terminals send for arrows,
//!   the editor keypad and function keys F1-F12 (see [`match_raw_sequence`];
//!   copied from TinyFugue's own `tf-lib/kbbind.tf` `~keyseq` table,
//!   including its skipped 16/22 in the F-key run), plus their `;2`/`;3`/`;5`
//!   Shift/Alt/Ctrl modifier variants and `^[[Z` (Shift-Tab).
//! - Numeric character escapes `\033` (octal), `\0x1B` (hex), `\27`
//!   (decimal), and `\e`/`\E` - all four spell the same Esc byte (`/help
//!   bind`'s own examples), so `\033b` and `^[b` normalise to the identical
//!   `Esc-b`.
//!
//! Anything that doesn't fit this grammar is a hard [`Result::Err`] with a
//! human-readable reason, never a silent guess.

/// One of the "named" keys the grammar recognizes on their own (`Up`,
/// `Tab`, `F5`, ...). [`NamedKey::parse`] is case-insensitive and accepts the
/// documented aliases; [`NamedKey::canonical`] always renders the same
/// spelling regardless of how it was spelled on input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,
    Backspace,
    Tab,
    Enter,
    Escape,
    Space,
    /// F1..F20 (TF's own function-key raw-sequence table stops at F20).
    F(u8),
}

impl NamedKey {
    /// Parse a bare word (no modifier prefix) into a named key. Case
    /// insensitive; accepts `PgUp/PgDn/Ins/Del/BS/Return/Esc` as aliases
    /// (plus `Cr`, an older alias this module keeps working).
    pub fn parse(word: &str) -> Option<Self> {
        let lower = word.to_ascii_lowercase();
        Some(match lower.as_str() {
            "up" => NamedKey::Up,
            "down" => NamedKey::Down,
            "left" => NamedKey::Left,
            "right" => NamedKey::Right,
            "pageup" | "pgup" => NamedKey::PageUp,
            "pagedown" | "pgdn" => NamedKey::PageDown,
            "home" => NamedKey::Home,
            "end" => NamedKey::End,
            "insert" | "ins" => NamedKey::Insert,
            "delete" | "del" => NamedKey::Delete,
            "backspace" | "bs" => NamedKey::Backspace,
            "tab" => NamedKey::Tab,
            "enter" | "return" | "cr" => NamedKey::Enter,
            "escape" | "esc" => NamedKey::Escape,
            "space" => NamedKey::Space,
            _ => return Self::parse_function_key(&lower),
        })
    }

    /// Whether `word` is even shaped like a function-key spelling (`f`/`F`
    /// followed by one or more digits) - used to turn an out-of-range
    /// function key (`F0`, `F21`) into a clear error instead of silently
    /// falling back to a chord of individual characters (see
    /// `parse_one_token`'s bare-word arm).
    fn looks_like_function_key(word: &str) -> bool {
        word.len() > 1
            && word.starts_with(['f', 'F'])
            && word[1..].bytes().all(|b| b.is_ascii_digit())
    }

    fn parse_function_key(lower: &str) -> Option<Self> {
        let digits = lower.strip_prefix('f')?;
        let n: u8 = digits.parse().ok()?;
        (1..=20).contains(&n).then_some(NamedKey::F(n))
    }

    /// Canonical spelling, independent of how it was written on input.
    pub fn canonical(&self) -> String {
        match self {
            NamedKey::Up => "Up".to_string(),
            NamedKey::Down => "Down".to_string(),
            NamedKey::Left => "Left".to_string(),
            NamedKey::Right => "Right".to_string(),
            NamedKey::PageUp => "PageUp".to_string(),
            NamedKey::PageDown => "PageDown".to_string(),
            NamedKey::Home => "Home".to_string(),
            NamedKey::End => "End".to_string(),
            NamedKey::Insert => "Insert".to_string(),
            NamedKey::Delete => "Delete".to_string(),
            NamedKey::Backspace => "Backspace".to_string(),
            NamedKey::Tab => "Tab".to_string(),
            NamedKey::Enter => "Enter".to_string(),
            NamedKey::Escape => "Escape".to_string(),
            NamedKey::Space => "Space".to_string(),
            NamedKey::F(n) => format!("F{n}"),
        }
    }
}

/// A real terminal modifier bit combined with a named key: `Ctrl-Up`,
/// `Shift-Tab`, `Alt-Down`. Distinct from [`KeyToken::Esc`] wrapping a named
/// key (`Esc-Up`) - that's Escape then Up as two sequential keystrokes, not
/// one physical chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
}

impl Modifier {
    fn prefix(&self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl-",
            Modifier::Shift => "Shift-",
            Modifier::Alt => "Alt-",
        }
    }
}

/// One token of a key name - see the module doc comment for the grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyToken {
    /// A control character, canonical `^<char>` (`^A`, `^?`, `^[`-the-prefix
    /// excepted - see the module doc comment).
    Ctrl(char),
    /// A named key with no modifier: `Up`, `Tab`, `F5`.
    Named(NamedKey),
    /// A named key with a real terminal modifier bit: `Ctrl-Up`, `Alt-Down`.
    Modified(Modifier, NamedKey),
    /// Escape, then one more token: `Esc-b`, `Esc-Left`, `Esc-^N`.
    Esc(Box<KeyToken>),
    /// A single printable character, case-sensitive.
    Char(char),
}

impl KeyToken {
    /// Canonical text for just this one token (see [`KeySeq::canonical`] for
    /// the whole sequence).
    pub fn canonical(&self) -> String {
        match self {
            KeyToken::Ctrl(c) => format!("^{c}"),
            KeyToken::Named(n) => n.canonical(),
            KeyToken::Modified(m, n) => format!("{}{}", m.prefix(), n.canonical()),
            KeyToken::Esc(inner) => format!("Esc-{}", inner.canonical()),
            KeyToken::Char(c) => c.to_string(),
        }
    }
}

/// A full key name: one or more [`KeyToken`]s in sequence (a chord).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySeq(pub Vec<KeyToken>);

impl KeySeq {
    /// The canonical text form of this sequence - concatenating each
    /// token's own canonical spelling, no separator (chords are written back
    /// to back: `^X^R`, not `^X ^R`).
    pub fn canonical(&self) -> String {
        self.0.iter().map(KeyToken::canonical).collect()
    }
}

/// Parse a key name - either already-canonical text, or one of TF's raw
/// forms (see the module doc comment) - into a [`KeySeq`]. Every caller that
/// stores or looks up a key name (`KeyBindings`, `tf::hooks::bind_key`,
/// `/def -b`/`-B`) should route through this so `Esc-j` and `Alt-j`/`^[j`/
/// `\033j` all land on the exact same canonical string.
pub fn parse_key_name(name: &str) -> Result<KeySeq, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Empty key name".to_string());
    }

    let expanded = expand_numeric_escapes(trimmed);
    let mut tokens = Vec::new();
    let mut rest: &str = &expanded;
    while !rest.is_empty() {
        let (token, remaining) = parse_one_token(rest)
            .map_err(|e| format!("Invalid key name {name:?}: {e}"))?;
        tokens.push(token);
        rest = remaining;
    }

    Ok(KeySeq(tokens))
}

/// True iff `candidate`'s own tokens are a genuine, strictly shorter prefix
/// of some key's tokens in `bound_keys` - i.e. more keystrokes could still
/// complete a longer binding starting with `candidate`. Every entry of
/// `bound_keys` is (re-)parsed and compared as a **token vector**, never as
/// canonical text, so e.g. the single token `F1` is never mistaken for a
/// prefix of the unrelated single token `F10` just because their canonical
/// spellings happen to share a leading character - a naive `str::starts_with`
/// on the canonical text would get this wrong. Only a genuine multi-token
/// relationship counts (`^X` is a real prefix of the two-token `^X^R`).
/// Used by `chords::ChordState::push`'s callers (plan Phase 2 step P2.2) to
/// decide whether a keystroke should buffer as a chord prefix.
pub fn is_prefix_of_any<'a>(mut bound_keys: impl Iterator<Item = &'a str>, candidate: &KeySeq) -> bool {
    bound_keys.any(|key| {
        parse_key_name(key)
            .map(|seq| {
                seq.0.len() > candidate.0.len() && seq.0[..candidate.0.len()] == candidate.0[..]
            })
            .unwrap_or(false)
    })
}

/// True iff `name` is already exactly its own canonical form - i.e.
/// `parse_key_name(name)?.canonical() == name`. Used to skip re-parsing
/// already-canonical text (the overwhelmingly common case: every literal in
/// `KeyBindings::tf_defaults()` is already canonical).
pub fn is_canonical(name: &str) -> bool {
    parse_key_name(name).map(|seq| seq.canonical() == name).unwrap_or(false)
}

// ============================================================================
// TF's `key_<name>` naming layer (TinyFugue-parity plan, finding A / Phase 2
// step P2.5). See `/help keys`'s "Mapping Named Keys to functions": the
// character sequence a key generates is bound (via `/def -b`) to a macro
// named `key_<name>`; redefining `key_<name>` is how a script (or a user's
// own `/def key_f5 = ...`) changes what the key does, independent of the
// terminal. `-B<name>` (deprecated upstream, but still accepted - `/help def`)
// names a key the same way, for direct binding instead of going through
// `key_<name>`.
//
// The two functions below are deliberately each other's exact inverse over
// the SAME small table (`tf_base_name`/`tf_base_name_to_named`) - one core
// vocabulary, read in the two directions `token_to_tf_name` (a pressed key ->
// what `key_<name>` to look for) and `tf_name_to_token` (`-B"name"`'s value ->
// the canonical key it binds) need. TF's own vocabulary of named keys is
// `up down left right home end pgup pgdn insert delete tab bspc f1..f20`
// (kbbind.tf's own `~keyname`/`~keyseq` invocations), with `ctrl_`/`shift_`/
// `meta_` modifier prefixes and (`key_<name>` only) an `esc_` prefix for
// "Escape, then this named key" - deliberately a SUBSET of what
// `NamedKey::parse` accepts as an ordinary key-name word (no "enter"/"escape"/
// "space", and Backspace is spelled "bspc" here, matching `/dokey BSPC`, not
// "backspace" - TF's own kbbind.tf has that alias commented out, since ^H/^?
// are "handled internally" per tf-help and never actually reach this layer).
// ============================================================================

/// TF's own `<name>` half of `key_<name>`/`-B<name>` for one bare [`NamedKey`],
/// with no modifier or `Esc-`/`ctrl_`/`shift_`/`meta_` prefix - `None` for a
/// [`NamedKey`] TF's own naming convention doesn't cover at all (`Enter`,
/// `Escape`, `Space` - none of kbbind.tf's `~keyname` calls name any of these).
/// Must stay the exact inverse of [`tf_base_name_to_named`] below.
fn tf_base_name(named: NamedKey) -> Option<String> {
    Some(match named {
        NamedKey::Up => "up".to_string(),
        NamedKey::Down => "down".to_string(),
        NamedKey::Left => "left".to_string(),
        NamedKey::Right => "right".to_string(),
        NamedKey::Home => "home".to_string(),
        NamedKey::End => "end".to_string(),
        NamedKey::PageUp => "pgup".to_string(),
        NamedKey::PageDown => "pgdn".to_string(),
        NamedKey::Insert => "insert".to_string(),
        NamedKey::Delete => "delete".to_string(),
        NamedKey::Tab => "tab".to_string(),
        NamedKey::Backspace => "bspc".to_string(),
        NamedKey::F(n) => format!("f{n}"),
        NamedKey::Enter | NamedKey::Escape | NamedKey::Space => return None,
    })
}

/// The inverse of [`tf_base_name`]: TF's `<name>` (already lower-cased by the
/// caller) back to the [`NamedKey`] it names - `None` for anything outside
/// that same small vocabulary (deliberately NOT delegating to
/// [`NamedKey::parse`], which accepts a broader alias set - "esc", "enter",
/// "cr", "return" - that isn't part of TF's `key_<name>`/`-B` naming at all).
fn tf_base_name_to_named(base: &str) -> Option<NamedKey> {
    match base {
        "up" => Some(NamedKey::Up),
        "down" => Some(NamedKey::Down),
        "left" => Some(NamedKey::Left),
        "right" => Some(NamedKey::Right),
        "home" => Some(NamedKey::Home),
        "end" => Some(NamedKey::End),
        "pgup" => Some(NamedKey::PageUp),
        "pgdn" => Some(NamedKey::PageDown),
        "insert" => Some(NamedKey::Insert),
        "delete" => Some(NamedKey::Delete),
        "tab" => Some(NamedKey::Tab),
        "bspc" => Some(NamedKey::Backspace),
        _ if NamedKey::looks_like_function_key(base) => NamedKey::parse(base),
        _ => None,
    }
}

/// A pressed key's canonical [`KeyToken`] -> TF's own `<name>` for
/// `key_<name>` (`Named(Up)` -> `"up"`, `Modified(Ctrl, Left)` ->
/// `"ctrl_left"`, `Esc(Named(Left))` -> `"esc_left"`). `None` for anything
/// outside TF's own named-key vocabulary - a plain printable character, a
/// bare control character, or an `Esc-<char>` chord over something that
/// isn't itself a named key (`Esc-b`, say) - all of which only ever reach a
/// target through a direct character-sequence `/bind`/`/def -b`, never
/// through the two-level `key_<name>` mapping.
pub fn token_to_tf_name(token: &KeyToken) -> Option<String> {
    match token {
        KeyToken::Named(n) => tf_base_name(*n),
        KeyToken::Modified(Modifier::Ctrl, n) => tf_base_name(*n).map(|b| format!("ctrl_{b}")),
        KeyToken::Modified(Modifier::Shift, n) => tf_base_name(*n).map(|b| format!("shift_{b}")),
        KeyToken::Modified(Modifier::Alt, n) => tf_base_name(*n).map(|b| format!("meta_{b}")),
        KeyToken::Esc(inner) => match inner.as_ref() {
            KeyToken::Named(n) => tf_base_name(*n).map(|b| format!("esc_{b}")),
            _ => None,
        },
        KeyToken::Ctrl(_) | KeyToken::Char(_) => None,
    }
}

/// If `canonical` (already-canonical key-name text, e.g. from
/// `chords::resolve_key_name`) is exactly ONE [`KeyToken`] - i.e. one physical
/// keystroke, not a multi-token chord like `^X^R` (TF's two-level naming never
/// covers a compound sequence) - and that token is one TF's own naming
/// convention covers, its `key_<name>` name. `None` otherwise.
pub fn single_token_tf_name(canonical: &str) -> Option<String> {
    let seq = parse_key_name(canonical).ok()?;
    match seq.0.as_slice() {
        [token] => token_to_tf_name(token),
        _ => None,
    }
}

/// The `key_<name>` macro names to check, in priority order, for one already-
/// resolved physical keystroke (`canonical`, e.g. from
/// `chords::resolve_key_name`) - empty if the keystroke isn't one TF's own
/// `key_<name>` layer covers at all (see [`single_token_tf_name`]).
/// `key_meta_<x>` falls back to `key_esc_<x>` when the first isn't defined
/// (tf-help's `keypad`/keys discussion: some terminals can't tell an Alt-
/// modified key apart from Escape followed by the plain key, so a script that
/// only defines `key_esc_<x>` should still catch an Alt-modified keypress).
pub fn key_macro_names(canonical: &str) -> Vec<String> {
    let Some(tfname) = single_token_tf_name(canonical) else { return Vec::new() };
    let mut names = vec![format!("key_{tfname}")];
    if let Some(rest) = tfname.strip_prefix("meta_") {
        names.push(format!("key_esc_{rest}"));
    }
    names
}

/// The inverse of [`token_to_tf_name`]: `-B<name>`'s value (TF's own
/// deprecated named-key binding, `/help def`'s `-B` entry) resolved back to
/// the canonical [`KeyToken`] it binds - case-insensitive, matching real TF's
/// own `-B` ("must be spelled as shown, but capitalization is ignored").
/// `nkp*` (numeric-keypad) names are rejected with a clear error: Clay's
/// grammar has no keypad-distinct token (a keypad key and its main-keyboard
/// equivalent are indistinguishable once decoded), so there is no canonical
/// key to bind them to - bind the raw character sequence with `-b`/`/bind`
/// instead.
pub fn tf_name_to_token(name: &str) -> Result<KeyToken, String> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("nkp") {
        return Err(format!(
            "{name:?} is a numeric-keypad key name (nkp*) with no Clay equivalent - \
             bind the raw character sequence instead (see /help bind)."
        ));
    }
    let (modifier, base) = if let Some(b) = lower.strip_prefix("esc_") {
        (None, b)
    } else if let Some(b) = lower.strip_prefix("ctrl_") {
        (Some(Modifier::Ctrl), b)
    } else if let Some(b) = lower.strip_prefix("shift_") {
        (Some(Modifier::Shift), b)
    } else if let Some(b) = lower.strip_prefix("meta_") {
        (Some(Modifier::Alt), b)
    } else {
        (None, lower.as_str())
    };
    let named = tf_base_name_to_named(base)
        .ok_or_else(|| format!("Unknown named key: {name:?}"))?;
    Ok(match (lower.starts_with("esc_"), modifier) {
        (true, _) => KeyToken::Esc(Box::new(KeyToken::Named(named))),
        (false, Some(m)) => KeyToken::Modified(m, named),
        (false, None) => KeyToken::Named(named),
    })
}

/// Expand TF's `\<number>` and `\e`/`\E` numeric-escape spellings of a raw
/// byte into the plain `^X` (or literal-character) notation the rest of the
/// parser understands, so every other rule only ever has to deal with one
/// notation. TF accepts octal (a leading `0` then more octal digits), hex
/// (`0x`/`0X` prefix) and plain decimal (`/help bind`: "the escape character
/// can be given by any of these forms: ^[, \033, \0x1B, or \27").
fn expand_numeric_escapes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if bytes[i] == b'\\' && i + 1 < input.len() {
            if let Some((value, consumed)) = parse_numeric_escape(&input[i + 1..]) {
                out.push_str(&byte_to_notation(value));
                i += 1 + consumed;
                continue;
            }
        }
        let ch = input[i..].chars().next().expect("i < input.len()");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Parse one numeric escape from `rest` (the text right after a `\`).
/// Returns the byte value and how many bytes of `rest` it consumed, or
/// `None` if `rest` doesn't start with a recognized numeric-escape spelling
/// (in which case the caller leaves the backslash alone).
fn parse_numeric_escape(rest: &str) -> Option<(u32, usize)> {
    let bytes = rest.as_bytes();
    let first = *bytes.first()?;

    if first == b'e' || first == b'E' {
        return Some((0x1B, 1));
    }
    if first == b'0' && matches!(bytes.get(1), Some(b'x' | b'X')) {
        let mut idx = 2;
        let mut val: u32 = 0;
        let mut n = 0;
        while idx < bytes.len() && n < 2 && bytes[idx].is_ascii_hexdigit() {
            val = val * 16 + (bytes[idx] as char).to_digit(16).expect("checked hexdigit");
            idx += 1;
            n += 1;
        }
        return (n > 0).then_some((val, idx));
    }
    if first == b'0' && matches!(bytes.get(1), Some(b'0'..=b'7')) {
        let mut idx = 1;
        let mut val: u32 = 0;
        let mut n = 0;
        while idx < bytes.len() && n < 3 && (b'0'..=b'7').contains(&bytes[idx]) {
            val = val * 8 + (bytes[idx] - b'0') as u32;
            idx += 1;
            n += 1;
        }
        return (n > 0).then_some((val, idx));
    }
    if first.is_ascii_digit() {
        let mut idx = 0;
        let mut val: u32 = 0;
        let mut n = 0;
        while idx < bytes.len() && n < 3 && bytes[idx].is_ascii_digit() {
            val = val * 10 + (bytes[idx] - b'0') as u32;
            idx += 1;
            n += 1;
        }
        return Some((val, idx));
    }
    None
}

/// Render a raw byte value the way the rest of the grammar spells it: `^X`
/// notation for the C0 control codes and DEL, the literal character for
/// anything printable.
fn byte_to_notation(value: u32) -> String {
    match value {
        0 => "^@".to_string(),
        1..=26 => format!("^{}", (b'A' + (value as u8 - 1)) as char),
        27 => "^[".to_string(),
        28 => "^\\".to_string(),
        29 => "^]".to_string(),
        30 => "^^".to_string(),
        31 => "^_".to_string(),
        127 => "^?".to_string(),
        32..=126 => (value as u8 as char).to_string(),
        _ => char::from_u32(value).map(|c| c.to_string()).unwrap_or_default(),
    }
}

/// Case-insensitively strip `prefix` from the front of `s`, returning the
/// remainder. `None` if `s` is shorter than `prefix` or doesn't match (safe
/// against `s` containing multi-byte chars: `str::get` refuses to slice off
/// a char boundary rather than panicking).
fn strip_ci_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = s.get(..prefix.len())?;
    candidate.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// Consume a maximal leading run of ASCII alphanumerics (a "word", for
/// matching against [`NamedKey::parse`]) from the front of `s`.
fn consume_word(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(s.len());
    s.split_at(end)
}

/// The multi-byte raw sequences a vt100/xterm-family terminal sends for
/// arrows, the editor keypad, and F1-F12 - what follows a literal `^[` byte
/// (already stripped by the caller). Copied from TinyFugue's own
/// `tf-lib/kbbind.tf` `~keyseq` table, including its skipped F-key codes 16
/// and 22 (xterm's own historical gap, not a Clay omission). Matched via
/// [`match_raw_sequence`] against the longest entry first, though nothing in
/// this table actually collides as a prefix of another entry.
fn raw_sequence_table() -> Vec<(&'static str, KeyToken)> {
    use KeyToken::{Modified, Named};
    use Modifier::{Alt, Ctrl, Shift};
    use NamedKey::*;

    let mut table = vec![
        ("[A", Named(Up)), ("OA", Named(Up)),
        ("[B", Named(Down)), ("OB", Named(Down)),
        ("[C", Named(Right)), ("OC", Named(Right)),
        ("[D", Named(Left)), ("OD", Named(Left)),

        ("[1;5A", Modified(Ctrl, Up)),
        ("[1;5B", Modified(Ctrl, Down)),
        ("[1;5C", Modified(Ctrl, Right)),
        ("[1;5D", Modified(Ctrl, Left)),
        ("[1;3A", Modified(Alt, Up)),
        ("[1;3B", Modified(Alt, Down)),
        ("[1;3C", Modified(Alt, Right)),
        ("[1;3D", Modified(Alt, Left)),
        ("[1;2A", Modified(Shift, Up)),
        ("[1;2B", Modified(Shift, Down)),
        ("[1;2C", Modified(Shift, Right)),
        ("[1;2D", Modified(Shift, Left)),

        ("[2~", Named(Insert)),
        ("[3~", Named(Delete)),
        ("[5~", Named(PageUp)),
        ("[6~", Named(PageDown)),
        ("[1~", Named(Home)), ("[H", Named(Home)), ("OH", Named(Home)),
        ("[4~", Named(End)), ("[F", Named(End)), ("OF", Named(End)),

        // Editor keypad with Ctrl/Meta/Shift, for versions of xterm with
        // modifyCursorKeys (finding 41 / Job 22c: mirrors kbbind.tf's own
        // ctrl_/meta_/shift_ insert/delete/home/end/pgup/pgdn ~keyseq
        // entries, which this table previously had no equivalent of at all -
        // so e.g. `^[[6;5~` (real TF's `ctrl_pgdn`) failed to parse as
        // anything sensible instead of naming `Ctrl-PageDown`).
        ("[2;5~", Modified(Ctrl, Insert)),
        ("[3;5~", Modified(Ctrl, Delete)),
        ("[1;5~", Modified(Ctrl, Home)), ("[1;5H", Modified(Ctrl, Home)),
        ("[4;5~", Modified(Ctrl, End)), ("[1;5F", Modified(Ctrl, End)),
        ("[5;5~", Modified(Ctrl, PageUp)),
        ("[6;5~", Modified(Ctrl, PageDown)),

        ("[2;3~", Modified(Alt, Insert)),
        ("[3;3~", Modified(Alt, Delete)),
        ("[1;3~", Modified(Alt, Home)), ("[1;3H", Modified(Alt, Home)),
        ("[4;3~", Modified(Alt, End)), ("[1;3F", Modified(Alt, End)),
        ("[5;3~", Modified(Alt, PageUp)),
        ("[6;3~", Modified(Alt, PageDown)),

        ("[2;2~", Modified(Shift, Insert)),
        ("[3;2~", Modified(Shift, Delete)),
        ("[1;2~", Modified(Shift, Home)), ("[1;2H", Modified(Shift, Home)),
        ("[4;2~", Modified(Shift, End)), ("[1;2F", Modified(Shift, End)),
        ("[5;2~", Modified(Shift, PageUp)),
        ("[6;2~", Modified(Shift, PageDown)),

        ("[Z", Modified(Shift, Tab)),

        // Function keys - vt100/vt220/xterm codes skip 16 and 22.
        ("[11~", Named(F(1))), ("OP", Named(F(1))),
        ("[12~", Named(F(2))), ("OQ", Named(F(2))),
        ("[13~", Named(F(3))), ("OR", Named(F(3))),
        ("[14~", Named(F(4))), ("OS", Named(F(4))),
        ("[15~", Named(F(5))),
        ("[17~", Named(F(6))),
        ("[18~", Named(F(7))),
        ("[19~", Named(F(8))),
        ("[20~", Named(F(9))),
        ("[21~", Named(F(10))),
        ("[23~", Named(F(11))),
        ("[24~", Named(F(12))),
    ];
    table.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    table
}

/// Try to match the front of `rest` (the text right after a literal `^[`)
/// against [`raw_sequence_table`]. `Some` consumes the matched sequence
/// entirely; `None` means `rest` isn't one of these raw special-key
/// spellings (the caller then treats the `^[` as an Esc-prefix chord over
/// whatever token comes next instead).
fn match_raw_sequence(rest: &str) -> Option<(KeyToken, &str)> {
    for (seq, token) in raw_sequence_table() {
        if let Some(leftover) = rest.strip_prefix(seq) {
            return Some((token, leftover));
        }
    }
    None
}

/// Parse exactly one [`KeyToken`] off the front of `s`, returning it and
/// whatever's left (so the caller can loop for a chord). See the module doc
/// comment for the grammar this implements.
fn parse_one_token(s: &str) -> Result<(KeyToken, &str), String> {
    if s.is_empty() {
        return Err("empty key token".to_string());
    }

    // "^[..." - either a raw special-key sequence, or an Esc-prefix chord.
    if let Some(rest) = s.strip_prefix("^[") {
        if rest.is_empty() {
            // A bare Escape keypress with nothing following it.
            return Ok((KeyToken::Named(NamedKey::Escape), rest));
        }
        if let Some((token, remaining)) = match_raw_sequence(rest) {
            return Ok((token, remaining));
        }
        let (inner, remaining) = parse_one_token(rest)?;
        return Ok((KeyToken::Esc(Box::new(inner)), remaining));
    }

    // "^<char>" - a literal control character (never reached for "^[" itself,
    // handled above).
    if let Some(rest) = s.strip_prefix('^') {
        let mut chars = rest.char_indices();
        let (_, c) = chars.next()
            .ok_or_else(|| format!("Incomplete control-key sequence: {s:?}"))?;
        let end = chars.next().map(|(i, _)| i).unwrap_or(rest.len());
        let canon = if c.is_ascii_alphabetic() { c.to_ascii_uppercase() } else { c };
        return Ok((KeyToken::Ctrl(canon), &rest[end..]));
    }

    for prefix in ["ctrl-", "ctrl+"] {
        if let Some(rest) = strip_ci_prefix(s, prefix) {
            return parse_ctrl_prefixed(rest, s);
        }
    }
    for prefix in ["shift-", "shift+"] {
        if let Some(rest) = strip_ci_prefix(s, prefix) {
            return parse_shift_prefixed(rest, s);
        }
    }
    for prefix in ["alt-", "alt+", "meta-", "meta+"] {
        if let Some(rest) = strip_ci_prefix(s, prefix) {
            return parse_alt_like_prefixed(rest, s);
        }
    }
    if let Some(rest) = strip_ci_prefix(s, "esc-") {
        return parse_esc_dash_prefixed(rest, s);
    }
    if let Some(rest) = s.strip_prefix('@') {
        if !rest.is_empty() {
            return parse_alt_like_prefixed(rest, s);
        }
    }

    // A bare named-key word, e.g. "Up", "F5", "Tab" (no modifier prefix).
    let (word, after) = consume_word(s);
    if !word.is_empty() {
        if let Some(named) = NamedKey::parse(word) {
            return Ok((KeyToken::Named(named), after));
        }
        if NamedKey::looks_like_function_key(word) {
            // Looks like an attempted "F<n>" spelling but out of range
            // (F0, F21, ...) - a clear error beats silently decomposing it
            // into a chord of individual characters below.
            return Err(format!("Invalid function key: {word}"));
        }
    }

    // Fall back to a single printable character (the grammar's last resort;
    // this is also what makes an arbitrary literal sequence like "hello"
    // round-trip as itself - a chord of single-char tokens).
    let c = s.chars().next().expect("s is non-empty");
    if c.is_whitespace() || c.is_control() {
        return Err(format!("Unrecognized key sequence: {s:?}"));
    }
    Ok((KeyToken::Char(c), &s[c.len_utf8()..]))
}

/// `Ctrl-<...>` / `Ctrl+<...>`: either a named key (`Ctrl-Up` ->
/// `Modified(Ctrl, Up)`) or a single letter (`Ctrl-A` -> `^A`) - anything
/// else is an error (there's no ASCII control code for e.g. `Ctrl-1`).
fn parse_ctrl_prefixed<'a>(rest: &'a str, original: &str) -> Result<(KeyToken, &'a str), String> {
    let (word, after) = consume_word(rest);
    if !word.is_empty() {
        if let Some(named) = NamedKey::parse(word) {
            return Ok((KeyToken::Modified(Modifier::Ctrl, named), after));
        }
    }
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => {
            Ok((KeyToken::Ctrl(c.to_ascii_uppercase()), ""))
        }
        _ => Err(format!("Invalid Ctrl key: {original}")),
    }
}

/// `Shift-<...>` / `Shift+<...>`: only meaningful for a named key
/// (`Shift-Up`, `Shift-Tab`) - `Shift-<letter>` is just the uppercase letter,
/// so it isn't part of this grammar.
fn parse_shift_prefixed<'a>(rest: &'a str, original: &str) -> Result<(KeyToken, &'a str), String> {
    let (word, after) = consume_word(rest);
    if !word.is_empty() {
        if let Some(named) = NamedKey::parse(word) {
            return Ok((KeyToken::Modified(Modifier::Shift, named), after));
        }
    }
    Err(format!("Invalid Shift key: {original}"))
}

/// `Alt-<...>` / `Alt+<...>` / `Meta-<...>` / `Meta+<...>` / `@<...>`: a
/// named key normalises to a real `Modified(Alt, ...)` token (matching a
/// genuine terminal Alt+arrow event); anything else is TF's documented
/// "these all mean Esc-x" equivalence (`Alt-x`/`Meta-x`/`@x` -> `Esc-x`,
/// case preserved - this is what fixes the old `Alt-j`/`Alt-J` collision,
/// finding A).
fn parse_alt_like_prefixed<'a>(rest: &'a str, original: &str) -> Result<(KeyToken, &'a str), String> {
    if rest.is_empty() {
        return Err(format!("Invalid Alt/Meta key: {original}"));
    }
    let (inner, remaining) = parse_one_token(rest)?;
    let token = match inner {
        KeyToken::Named(n) => KeyToken::Modified(Modifier::Alt, n),
        other => KeyToken::Esc(Box::new(other)),
    };
    Ok((token, remaining))
}

/// `Esc-<...>`: Escape, then whatever token comes next - always wraps,
/// never converts a named key into a `Modified` token (that's only for the
/// real-modifier `Alt-`/`Meta-`/`@` spellings above).
fn parse_esc_dash_prefixed<'a>(rest: &'a str, original: &str) -> Result<(KeyToken, &'a str), String> {
    if rest.is_empty() {
        return Err(format!("Invalid Esc- key: {original}"));
    }
    let (inner, remaining) = parse_one_token(rest)?;
    Ok((KeyToken::Esc(Box::new(inner)), remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(name: &str) -> String {
        parse_key_name(name).unwrap_or_else(|e| panic!("{name:?} should parse: {e}")).canonical()
    }

    // ---- Canonical grammar: existing names, chords, round-trip ----

    #[test]
    fn test_canonical_forms_round_trip() {
        // Every one of these is already its own canonical spelling - parsing
        // it and re-rendering it must reproduce the exact same string.
        for name in [
            "^A", "^Z", "^?", "^I", "^M", "^J", "^@", "^]",
            "Up", "Down", "Left", "Right", "PageUp", "PageDown", "Home", "End",
            "Insert", "Delete", "Backspace", "Tab", "Enter", "Escape", "Space",
            "F1", "F9", "F12", "F20",
            "Ctrl-Up", "Ctrl-Down", "Ctrl-Left", "Ctrl-Right",
            "Shift-Up", "Shift-Down", "Shift-Tab",
            "Alt-Up", "Alt-Down",
            "Esc-b", "Esc-J", "Esc-j", "Esc-^N", "Esc-Left", "Esc-{", "Esc-Tab",
            "Esc-Backspace", "Esc-Space", "Esc-0", "Esc--", "Esc-.", "Esc-_",
            "^X^R", "^X[", "^X]", "^X{", "^X}", "^X^?",
            "a", "hello",
        ] {
            assert_eq!(canon(name), name, "canonical form {name:?} should round-trip");
            assert!(is_canonical(name), "{name:?} should report as canonical");
        }
    }

    #[test]
    fn test_esc_case_is_significant() {
        // finding A, bug 1: Esc-j and Esc-J must be two DIFFERENT canonical
        // keys, not collide under a shared upper-cased form.
        assert_eq!(canon("Esc-j"), "Esc-j");
        assert_eq!(canon("Esc-J"), "Esc-J");
        assert_ne!(canon("Esc-j"), canon("Esc-J"));
    }

    #[test]
    fn test_named_keys_case_insensitive_input_canonical_output() {
        assert_eq!(canon("up"), "Up");
        assert_eq!(canon("UP"), "Up");
        assert_eq!(canon("pgup"), "PageUp");
        assert_eq!(canon("PGDN"), "PageDown");
        assert_eq!(canon("ins"), "Insert");
        assert_eq!(canon("del"), "Delete");
        assert_eq!(canon("bs"), "Backspace");
        assert_eq!(canon("return"), "Enter");
        assert_eq!(canon("esc"), "Escape");
        assert_eq!(canon("f5"), "F5");
    }

    #[test]
    fn test_ctrl_alt_shift_word_prefixes() {
        assert_eq!(canon("Ctrl-A"), "^A");
        assert_eq!(canon("ctrl+x"), "^X");
        assert_eq!(canon("Ctrl-Up"), "Ctrl-Up");
        assert_eq!(canon("Ctrl-PgUp"), "Ctrl-PageUp");
        assert_eq!(canon("Shift-Up"), "Shift-Up");
        assert_eq!(canon("Shift-Tab"), "Shift-Tab");
        assert_eq!(canon("Alt-Up"), "Alt-Up");
        // Alt/Meta/@ + a plain char all mean Esc-<char>, case preserved.
        assert_eq!(canon("Alt-j"), "Esc-j");
        assert_eq!(canon("Alt-J"), "Esc-J");
        assert_eq!(canon("Meta-w"), "Esc-w");
        assert_eq!(canon("@w"), "Esc-w");
    }

    // ---- TF raw forms -> canonical ----

    #[test]
    fn test_raw_esc_prefix_chords() {
        assert_eq!(canon("^[b"), "Esc-b");
        assert_eq!(canon("^[J"), "Esc-J");
        assert_eq!(canon("^[^N"), "Esc-^N");
        assert_eq!(canon("^["), "Escape");
    }

    #[test]
    fn test_raw_arrow_sequences() {
        assert_eq!(canon("^[[A"), "Up");
        assert_eq!(canon("^[OA"), "Up");
        assert_eq!(canon("^[[B"), "Down");
        assert_eq!(canon("^[[C"), "Right");
        assert_eq!(canon("^[[D"), "Left");
    }

    #[test]
    fn test_raw_arrow_modifier_variants() {
        assert_eq!(canon("^[[1;5A"), "Ctrl-Up");
        assert_eq!(canon("^[[1;5B"), "Ctrl-Down");
        assert_eq!(canon("^[[1;5C"), "Ctrl-Right");
        assert_eq!(canon("^[[1;5D"), "Ctrl-Left");
        assert_eq!(canon("^[[1;3A"), "Alt-Up");
        assert_eq!(canon("^[[1;2A"), "Shift-Up");
    }

    #[test]
    fn test_raw_editor_keypad() {
        assert_eq!(canon("^[[2~"), "Insert");
        assert_eq!(canon("^[[3~"), "Delete");
        assert_eq!(canon("^[[5~"), "PageUp");
        assert_eq!(canon("^[[6~"), "PageDown");
        assert_eq!(canon("^[[1~"), "Home");
        assert_eq!(canon("^[[H"), "Home");
        assert_eq!(canon("^[OH"), "Home");
        assert_eq!(canon("^[[4~"), "End");
        assert_eq!(canon("^[[F"), "End");
        assert_eq!(canon("^[OF"), "End");
    }

    /// Finding 41 / Job 22c: the editor keypad's Ctrl/Meta/Shift variants (real TF's own
    /// `ctrl_pgdn`/`ctrl_home`/etc. `~keyseq` entries from `tf-lib/kbbind.tf`) had no raw-form
    /// table entries at all before this fix, so e.g. `^[[6;5~` (Ctrl-PageDown) didn't parse as
    /// a `PageDown` token with a Ctrl modifier - it decomposed into a chord of stray
    /// punctuation/digit characters instead.
    #[test]
    fn test_raw_editor_keypad_modifier_variants() {
        assert_eq!(canon("^[[2;5~"), "Ctrl-Insert");
        assert_eq!(canon("^[[3;5~"), "Ctrl-Delete");
        assert_eq!(canon("^[[1;5~"), "Ctrl-Home");
        assert_eq!(canon("^[[1;5H"), "Ctrl-Home");
        assert_eq!(canon("^[[4;5~"), "Ctrl-End");
        assert_eq!(canon("^[[1;5F"), "Ctrl-End");
        assert_eq!(canon("^[[5;5~"), "Ctrl-PageUp");
        assert_eq!(canon("^[[6;5~"), "Ctrl-PageDown");

        assert_eq!(canon("^[[2;3~"), "Alt-Insert");
        assert_eq!(canon("^[[3;3~"), "Alt-Delete");
        assert_eq!(canon("^[[1;3~"), "Alt-Home");
        assert_eq!(canon("^[[1;3H"), "Alt-Home");
        assert_eq!(canon("^[[4;3~"), "Alt-End");
        assert_eq!(canon("^[[1;3F"), "Alt-End");
        assert_eq!(canon("^[[5;3~"), "Alt-PageUp");
        assert_eq!(canon("^[[6;3~"), "Alt-PageDown");

        assert_eq!(canon("^[[2;2~"), "Shift-Insert");
        assert_eq!(canon("^[[3;2~"), "Shift-Delete");
        assert_eq!(canon("^[[1;2~"), "Shift-Home");
        assert_eq!(canon("^[[1;2H"), "Shift-Home");
        assert_eq!(canon("^[[4;2~"), "Shift-End");
        assert_eq!(canon("^[[1;2F"), "Shift-End");
        assert_eq!(canon("^[[5;2~"), "Shift-PageUp");
        assert_eq!(canon("^[[6;2~"), "Shift-PageDown");
    }

    #[test]
    fn test_raw_function_keys_skip_16_and_22() {
        let expected = [
            ("^[[11~", "F1"), ("^[OP", "F1"),
            ("^[[12~", "F2"), ("^[OQ", "F2"),
            ("^[[13~", "F3"), ("^[OR", "F3"),
            ("^[[14~", "F4"), ("^[OS", "F4"),
            ("^[[15~", "F5"),
            ("^[[17~", "F6"),
            ("^[[18~", "F7"),
            ("^[[19~", "F8"),
            ("^[[20~", "F9"),
            ("^[[21~", "F10"),
            ("^[[23~", "F11"),
            ("^[[24~", "F12"),
        ];
        for (raw, want) in expected {
            assert_eq!(canon(raw), want, "raw {raw:?}");
        }
    }

    #[test]
    fn test_raw_shift_tab() {
        assert_eq!(canon("^[[Z"), "Shift-Tab");
    }

    #[test]
    fn test_numeric_escapes_all_mean_esc() {
        assert_eq!(canon("\\033"), "Escape");
        assert_eq!(canon("\\0x1B"), "Escape");
        assert_eq!(canon("\\27"), "Escape");
        assert_eq!(canon("\\e"), "Escape");
        assert_eq!(canon("\\E"), "Escape");
        // Followed by more input, exactly like the "^[b" chord form.
        assert_eq!(canon("\\033b"), "Esc-b");
        assert_eq!(canon("\\0x1Bb"), "Esc-b");
        assert_eq!(canon("\\27b"), "Esc-b");
        assert_eq!(canon("\\eb"), "Esc-b");
    }

    // ---- Error cases ----

    #[test]
    fn test_error_cases() {
        assert!(parse_key_name("").is_err());
        assert!(parse_key_name("   ").is_err());
        assert!(parse_key_name("F0").is_err());
        assert!(parse_key_name("F21").is_err());
        assert!(parse_key_name("Ctrl-1").is_err());
        assert!(parse_key_name("Ctrl-Nonsense").is_err());
        assert!(parse_key_name("Shift-a").is_err());
        assert!(parse_key_name("Shift-1").is_err());
        assert!(parse_key_name("Esc-").is_err());
        assert!(parse_key_name("Alt-").is_err());
        assert!(parse_key_name("^").is_err(), "a lone caret has no control-key target");
        assert!(!is_canonical(""));
        assert!(!is_canonical("^"));
    }

    #[test]
    fn test_arbitrary_literal_sequence_is_a_char_chord() {
        // Not a documented "special" grammar form, but a graceful fallback:
        // any leftover printable text decomposes into single-char tokens and
        // therefore round-trips as itself (matches the old parser's "allow
        // arbitrary key sequences as-is").
        assert_eq!(canon("hello"), "hello");
        let seq = parse_key_name("hello").unwrap();
        assert_eq!(seq.0.len(), 5);
    }

    // ---- `key_<name>`/`-B<name>` naming layer (plan Job 21 / P2.5) ----

    #[test]
    fn test_single_token_tf_name_bare_named_keys() {
        assert_eq!(single_token_tf_name("Up"), Some("up".to_string()));
        assert_eq!(single_token_tf_name("Home"), Some("home".to_string()));
        assert_eq!(single_token_tf_name("PageUp"), Some("pgup".to_string()));
        assert_eq!(single_token_tf_name("PageDown"), Some("pgdn".to_string()));
        assert_eq!(single_token_tf_name("Backspace"), Some("bspc".to_string()));
        assert_eq!(single_token_tf_name("Tab"), Some("tab".to_string()));
        assert_eq!(single_token_tf_name("F5"), Some("f5".to_string()));
        assert_eq!(single_token_tf_name("F20"), Some("f20".to_string()));
    }

    #[test]
    fn test_single_token_tf_name_modifiers_and_esc() {
        assert_eq!(single_token_tf_name("Ctrl-Left"), Some("ctrl_left".to_string()));
        assert_eq!(single_token_tf_name("Shift-Up"), Some("shift_up".to_string()));
        assert_eq!(single_token_tf_name("Alt-Left"), Some("meta_left".to_string()));
        assert_eq!(single_token_tf_name("Esc-Left"), Some("esc_left".to_string()));
    }

    #[test]
    fn test_single_token_tf_name_excludes_plain_chars_and_chords() {
        // Plain characters and arbitrary Esc-<char> chords aren't part of TF's
        // named-key vocabulary at all - only a direct character-sequence bind
        // reaches them.
        assert_eq!(single_token_tf_name("a"), None);
        assert_eq!(single_token_tf_name("Esc-b"), None);
        assert_eq!(single_token_tf_name("^A"), None);
        // A genuine multi-token chord is never "one physical keystroke".
        assert_eq!(single_token_tf_name("^X^R"), None);
    }

    #[test]
    fn test_key_macro_names_meta_falls_back_to_esc() {
        assert_eq!(key_macro_names("Esc-Left"), vec!["key_esc_left".to_string()]);
        assert_eq!(
            key_macro_names("Alt-Left"),
            vec!["key_meta_left".to_string(), "key_esc_left".to_string()],
            "key_meta_<x> must be tried before falling back to key_esc_<x>"
        );
        assert_eq!(key_macro_names("F5"), vec!["key_f5".to_string()]);
        assert_eq!(key_macro_names("a"), Vec::<String>::new());
    }

    #[test]
    fn test_tf_name_to_token_round_trips_single_token_tf_name() {
        for name in ["Up", "Down", "Left", "Right", "Home", "End", "Insert",
            "Delete", "Tab", "Backspace", "F1", "F20"] {
            let tfname = single_token_tf_name(name).unwrap_or_else(|| panic!("{name} should be named"));
            let token = tf_name_to_token(&tfname).unwrap_or_else(|e| panic!("{tfname:?}: {e}"));
            assert_eq!(KeySeq(vec![token]).canonical(), name, "round trip via tfname {tfname:?}");
        }
    }

    #[test]
    fn test_tf_name_to_token_examples_from_the_plan() {
        // plan Job 21/P2.5's own worked examples for the -B inverse mapping.
        assert_eq!(
            tf_name_to_token("up").map(|t| KeySeq(vec![t]).canonical()),
            Ok("Up".to_string())
        );
        assert_eq!(
            tf_name_to_token("ctrl_left").map(|t| KeySeq(vec![t]).canonical()),
            Ok("Ctrl-Left".to_string())
        );
        assert_eq!(
            tf_name_to_token("esc_left").map(|t| KeySeq(vec![t]).canonical()),
            Ok("Esc-Left".to_string())
        );
        assert_eq!(
            tf_name_to_token("f5").map(|t| KeySeq(vec![t]).canonical()),
            Ok("F5".to_string())
        );
    }

    #[test]
    fn test_tf_name_to_token_case_insensitive() {
        assert_eq!(tf_name_to_token("UP"), tf_name_to_token("up"));
        assert_eq!(tf_name_to_token("F5"), tf_name_to_token("f5"));
        assert_eq!(tf_name_to_token("Ctrl_Left"), tf_name_to_token("ctrl_left"));
    }

    #[test]
    fn test_tf_name_to_token_rejects_nkp_and_unknown_names() {
        assert!(tf_name_to_token("nkp5").is_err(), "nkp* has no Clay equivalent");
        assert!(tf_name_to_token("nkpTab").is_err());
        assert!(tf_name_to_token("bogus").is_err());
        // "escape"/"enter"/"space" aren't part of TF's key_<name>/-B vocabulary.
        assert!(tf_name_to_token("escape").is_err());
    }
}
