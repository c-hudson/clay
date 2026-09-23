//! Turning a chat message into Clay output lines. Pure functions, shared by Discord
//! and Slack; the protocol modules do the service-specific mention/markup resolution
//! first and hand plain text here.
//!
//! Line shape (what triggers see): `#general <Alice> hello`, `@alice <Alice> hi` for a
//! DM, `#general * Alice joined` for a system/action message. Continuation lines of a
//! multi-line message are indented two spaces. **No line ever starts with `[`** -
//! `util::strip_mud_tag` would strip it as a MUD tag and hide the channel.

/// Strip everything a remote user could use to drive the terminal or corrupt the
/// display: ESC (so no ANSI of theirs), every other C0 control except `\n`/`\t` (tab
/// becomes a space), DEL, and every C1 control (U+0080-U+009F - see CLAUDE.md: a C1
/// written to a UTF-8 terminal can swallow the redraw that follows it). `\r` goes too:
/// a lone CR would let a message overwrite its own tag.
pub fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' => out.push('\n'),
            '\t' => out.push(' '),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c if ('\u{80}'..='\u{9f}').contains(&c) => {}
            // Unicode line/paragraph separators would split a line behind our back.
            '\u{2028}' | '\u{2029}' => out.push('\n'),
            c => out.push(c),
        }
    }
    out
}

/// Sanitize a single-line field (a name or channel label): newlines become spaces.
pub fn sanitize_inline(s: &str) -> String {
    sanitize(s).replace('\n', " ").trim().to_string()
}

/// Build the output lines for one message. `tag` is `#channel`/`@user` (already
/// sanitized by the caller via `sanitize_inline`), `author` the display name, `body`
/// the resolved text; `extras` are appended as their own indented lines
/// (attachments, embeds). An empty body with extras puts the first extra on the
/// first line so an attachment-only message isn't a blank `<Alice>`.
pub fn message_lines(tag: &str, author: &str, prefix: &str, body: &str, extras: &[String]) -> Vec<String> {
    let author = sanitize_inline(author);
    let body = sanitize(body);
    let mut body_lines: Vec<String> = body.split('\n').map(|l| l.trim_end().to_string()).collect();
    while body_lines.len() > 1 && body_lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        body_lines.pop();
    }
    let mut extras: Vec<String> = extras.iter().map(|e| sanitize_inline(e)).filter(|e| !e.is_empty()).collect();
    let mut first = body_lines.remove(0);
    if first.is_empty() && body_lines.is_empty() && !extras.is_empty() {
        first = extras.remove(0);
    }
    let head = if prefix.is_empty() {
        format!("{} <{}> {}", tag, author, first)
    } else {
        format!("{} <{}> {} {}", tag, author, prefix, first)
    };
    let mut lines = vec![head.trim_end().to_string()];
    for l in body_lines {
        lines.push(format!("  {}", l));
    }
    for e in extras {
        lines.push(format!("  {}", e));
    }
    lines
}

/// A system/action line: `#general * Alice joined the server`.
pub fn action_line(tag: &str, text: &str) -> String {
    format!("{} * {}", tag, sanitize_inline(text))
}

/// Shorten `s` to at most `max` characters, adding `…` when cut.
pub fn ellipsize(s: &str, max: usize) -> String {
    let flat = sanitize_inline(s);
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max.saturating_sub(1)).collect::<String>().trim_end().to_string();
    out.push('…');
    out
}

/// Split outgoing text into chunks of at most `limit` characters (Discord counts
/// characters, not bytes), preferring newline, then whitespace, boundaries. Never
/// returns an empty chunk; an empty input gives no chunks.
pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut out = Vec::new();
    let mut rest: &str = text;
    while !rest.is_empty() {
        if rest.chars().count() <= limit {
            out.push(rest.to_string());
            break;
        }
        // Byte offset just past the `limit`-th character.
        let hard = rest.char_indices().nth(limit).map(|(i, _)| i).unwrap_or(rest.len());
        let window = &rest[..hard];
        let cut = window
            .rfind('\n')
            .filter(|&i| i > 0)
            .map(|i| (i, i + 1))
            .or_else(|| window.rfind(char::is_whitespace).filter(|&i| i > 0).map(|i| {
                let w = window[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                (i, i + w)
            }))
            .unwrap_or((hard, hard));
        let chunk = &rest[..cut.0];
        if !chunk.is_empty() {
            out.push(chunk.to_string());
        }
        rest = &rest[cut.1..];
    }
    out.retain(|c| !c.trim().is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_controls() {
        assert_eq!(sanitize("a\x1b[31mred\x1b[0m\u{9f}x\u{85}y\r\n\tz\x07"), "a[31mred[0mxy\n z");
        assert_eq!(sanitize_inline("Al\nice\x1b"), "Al ice");
        assert_eq!(sanitize("a\u{2028}b"), "a\nb");
    }

    #[test]
    fn lines_shape() {
        assert_eq!(message_lines("#general", "Alice", "", "hello", &[]), vec!["#general <Alice> hello"]);
        assert_eq!(
            message_lines("#general", "Alice", "", "one\ntwo\n\n", &["[file: a.png https://x/a.png]".into()]),
            vec!["#general <Alice> one", "  two", "  [file: a.png https://x/a.png]"]
        );
        // Attachment-only message: the file goes on the first line.
        assert_eq!(
            message_lines("@alice", "Alice", "", "", &["[file: a.png u]".into()]),
            vec!["@alice <Alice> [file: a.png u]"]
        );
        assert_eq!(
            message_lines("#g", "Bob", "(edited)", "fixed", &[]),
            vec!["#g <Bob> (edited) fixed"]
        );
        assert_eq!(action_line("#g", "Alice joined"), "#g * Alice joined");
    }

    #[test]
    fn never_starts_with_bracket_so_tags_survive() {
        for l in message_lines("#general", "[x]", "", "[y]\n[z]", &[]) {
            assert!(!l.starts_with('['), "{}", l);
            assert_eq!(crate::util::strip_mud_tag(&l), l);
        }
    }

    #[test]
    fn split() {
        assert!(split_message("", 10).is_empty());
        assert_eq!(split_message("short", 2000), vec!["short"]);
        let s = "a".repeat(2000);
        assert_eq!(split_message(&s, 2000), vec![s.clone()]);
        // Word boundary.
        assert_eq!(split_message("hello world again", 11), vec!["hello", "world again"]);
        // Newline preferred over space.
        assert_eq!(split_message("aa bb\ncc dd", 9), vec!["aa bb", "cc dd"]);
        // One huge word is hard-split by characters, not bytes.
        let long: String = "é".repeat(5000);
        let parts = split_message(&long, 2000);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.chars().count() <= 2000));
        assert_eq!(parts.concat(), long);
    }

    #[test]
    fn ellipsize_counts_chars() {
        assert_eq!(ellipsize("hello", 10), "hello");
        assert_eq!(ellipsize("hello world", 6), "hello…");
    }
}
