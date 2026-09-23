//! Target/server "specs": what a user types (or the Fetch picker writes) to name a
//! Discord/Slack server, channel or user.
//!
//! Accepted forms, all case-insensitive for names:
//! - a raw id: `123456789012345678` (Discord snowflake) or `C0123ABCD` (Slack)
//! - a name: `general`, `#general`, `@alice`, `My Server`
//! - the picker's form: `general (123456789012345678)` — the id wins, the name is for
//!   humans, so a renamed channel still resolves.
//!
//! Numeric ids in settings files from before the overhaul keep working unchanged.

use super::ChatKind;

/// What a spec says it refers to. `#` forces a channel, `@` forces a user, and a bare
/// word lets the resolver try channels first, then users.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecHint {
    Any,
    Channel,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub hint: SpecHint,
    /// An id, when the spec contained one (bare or in the picker's trailing `(id)`).
    pub id: Option<String>,
    /// The name part, lower-cased, without `#`/`@`. Empty when the spec was a bare id.
    pub name: String,
}

/// Does `s` look like an id for this service? Discord ids are 15-21 digit snowflakes.
/// Slack ids are upper-case alphanumerics starting with a type letter (C channel,
/// G private channel, D direct message, U/W user, T team) - Slack channel *names* are
/// always lower-case, so there is no ambiguity.
pub fn looks_like_id(kind: ChatKind, s: &str) -> bool {
    match kind {
        ChatKind::Discord => (15..=21).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()),
        ChatKind::Slack => {
            s.len() >= 8
                && s.len() <= 14
                && matches!(s.as_bytes()[0], b'C' | b'G' | b'D' | b'U' | b'W' | b'T' | b'B')
                && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        }
    }
}

/// Parse a spec; `None` for an empty/blank string.
pub fn parse(kind: ChatKind, raw: &str) -> Option<Spec> {
    let mut s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let mut hint = SpecHint::Any;
    if let Some(rest) = s.strip_prefix('#') {
        hint = SpecHint::Channel;
        s = rest.trim_start();
    } else if let Some(rest) = s.strip_prefix('@') {
        hint = SpecHint::User;
        s = rest.trim_start();
    }
    // Picker form: "name (id)".
    if s.ends_with(')') {
        if let Some(open) = s.rfind('(') {
            let inner = &s[open + 1..s.len() - 1];
            if looks_like_id(kind, inner) {
                let name = s[..open].trim();
                let name = name.strip_prefix('#').or_else(|| name.strip_prefix('@')).unwrap_or(name);
                return Some(Spec { hint, id: Some(inner.to_string()), name: name.to_lowercase() });
            }
        }
    }
    if looks_like_id(kind, s) {
        return Some(Spec { hint, id: Some(s.to_string()), name: String::new() });
    }
    if s.is_empty() {
        return None;
    }
    Some(Spec { hint, id: None, name: s.to_lowercase() })
}

/// Parse a comma-separated list of specs (the "Show Only" filter). Blank entries are
/// skipped, so `""` is the empty filter (show everything).
pub fn parse_list(kind: ChatKind, raw: &str) -> Vec<Spec> {
    raw.split(',').filter_map(|p| parse(kind, p)).collect()
}

/// The picker's canonical form for a pick: `name (id)`.
pub fn picker_value(name: &str, id: &str) -> String {
    if name.is_empty() {
        id.to_string()
    } else {
        format!("{} ({})", name, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNOW: &str = "123456789012345678";

    #[test]
    fn empty_is_none() {
        assert_eq!(parse(ChatKind::Discord, "   "), None);
        assert_eq!(parse(ChatKind::Discord, "#"), None);
        assert!(parse_list(ChatKind::Discord, " , ,").is_empty());
    }

    #[test]
    fn bare_id_and_names() {
        let s = parse(ChatKind::Discord, SNOW).unwrap();
        assert_eq!(s.id.as_deref(), Some(SNOW));
        assert_eq!(s.hint, SpecHint::Any);
        let s = parse(ChatKind::Discord, "#General").unwrap();
        assert_eq!((s.hint, s.id, s.name.as_str()), (SpecHint::Channel, None, "general"));
        let s = parse(ChatKind::Discord, "@Alice").unwrap();
        assert_eq!((s.hint, s.name.as_str()), (SpecHint::User, "alice"));
        // A short number is a name, not an id (e.g. a channel called "2024").
        assert_eq!(parse(ChatKind::Discord, "2024").unwrap().id, None);
    }

    #[test]
    fn picker_form() {
        let s = parse(ChatKind::Discord, &format!("general ({})", SNOW)).unwrap();
        assert_eq!(s.id.as_deref(), Some(SNOW));
        assert_eq!(s.name, "general");
        let s = parse(ChatKind::Discord, &format!("@alice ({})", SNOW)).unwrap();
        assert_eq!((s.hint, s.name.as_str()), (SpecHint::User, "alice"));
        // Parentheses that don't hold an id are part of the name.
        let s = parse(ChatKind::Discord, "lounge (old)").unwrap();
        assert_eq!((s.id, s.name.as_str()), (None, "lounge (old)"));
        assert_eq!(picker_value("general", SNOW), format!("general ({})", SNOW));
    }

    #[test]
    fn slack_ids() {
        assert!(looks_like_id(ChatKind::Slack, "C0123ABCD"));
        assert!(looks_like_id(ChatKind::Slack, "U02ABCDEF12"));
        assert!(!looks_like_id(ChatKind::Slack, "general"));
        assert!(!looks_like_id(ChatKind::Slack, "C01"));
        let s = parse(ChatKind::Slack, "random (C0123ABCD)").unwrap();
        assert_eq!(s.id.as_deref(), Some("C0123ABCD"));
        // A Discord snowflake is not a Slack id, and vice versa.
        assert_eq!(parse(ChatKind::Slack, SNOW).unwrap().id, None);
        assert_eq!(parse(ChatKind::Discord, "C0123ABCD").unwrap().id, None);
    }

    #[test]
    fn list() {
        let l = parse_list(ChatKind::Discord, "#general, random ,, 123456789012345678");
        assert_eq!(l.len(), 3);
        assert_eq!(l[1].name, "random");
    }
}
