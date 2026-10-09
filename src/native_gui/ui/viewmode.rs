//! What a window of output is for: the main window (anything), a `/window <world>` window
//! (that world only) or a `/window --grep` window (only the lines matching, nothing else
//! on screen). app.js's `LOCK_WORLD` and `GREP_MODE`.

/// A window's mode.
#[derive(Clone, Debug, Default)]
pub struct ViewMode {
    /// The world this window keeps to (by name: indices change across a reconnect).
    pub lock: Option<String>,
    pub grep: Option<Grep>,
}

/// A grep window's pattern.
#[derive(Clone, Debug)]
pub struct Grep {
    /// `None`: the pattern didn't compile, and every line shows (as app.js does).
    re: Option<regex::Regex>,
}

impl Grep {
    /// A glob (`*` any run, `?` one character), or with `regex` a regular expression;
    /// either way case-insensitive.
    pub fn new(pattern: &str, regex: bool) -> Grep {
        let source = if regex {
            pattern.to_string()
        } else {
            let mut out = String::new();
            for c in pattern.chars() {
                match c {
                    '*' => out.push_str(".*"),
                    '?' => out.push('.'),
                    c => out.push_str(&regex::escape(&c.to_string())),
                }
            }
            out
        };
        let re = regex::RegexBuilder::new(&source).case_insensitive(true).build().ok();
        Grep { re }
    }

    /// Whether a line shows: its displayed text (no escape codes, no MUD tag) matches.
    pub fn matches(&self, raw: &str) -> bool {
        let Some(re) = &self.re else { return true };
        let plain = super::find::plain(raw);
        re.is_match(&crate::native_gui::core::style::strip_mud_tag(&plain))
    }
}

/// `/window --grep pattern [-w world] [--regexp]`, the pattern optionally quoted
/// (app.js `sendCommand`): `None` when there's no `--grep`.
pub fn parse_grep_window(args: &str) -> Option<(String, Option<String>, bool)> {
    let at = args.find("--grep")?;
    let rest = args[at + 6..].trim_start();
    if rest.len() == args[at + 6..].len() && !rest.is_empty() {
        return None; // "--grepx", not the flag
    }
    let pattern = match rest.chars().next()? {
        q @ ('"' | '\'') => {
            let body = &rest[1..];
            body[..body.find(q)?].to_string()
        }
        _ => rest.split_whitespace().next()?.to_string(),
    };
    if pattern.is_empty() {
        return None;
    }
    let mut words = args.split_whitespace();
    let mut world = None;
    while let Some(w) = words.next() {
        if w == "-w" {
            world = words.next().map(str::to_string);
        }
    }
    Some((pattern, world, args.contains("--regexp")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_and_regexes_match_the_shown_text() {
        let g = Grep::new("*tells you*", false);
        assert!(g.matches("Bob \x1b[1mtells you\x1b[0m hi"));
        assert!(g.matches("BOB TELLS YOU HI"), "case-insensitive");
        assert!(!g.matches("Bob says hi"));
        assert!(Grep::new("a.c", false).matches("xa.cy") && !Grep::new("a.c", false).matches("abc"), "a glob's dot is a dot");
        assert!(Grep::new("^Bob", true).matches("Bob hi"));
        assert!(Grep::new("([", true).matches("anything"), "a bad regex shows everything");
    }

    #[test]
    fn grep_windows_parse_as_app_js_parses_them() {
        assert_eq!(parse_grep_window("--grep foo"), Some(("foo".into(), None, false)));
        assert_eq!(parse_grep_window("--grep \"two words\" -w Mud --regexp"), Some(("two words".into(), Some("Mud".into()), true)));
        assert_eq!(parse_grep_window("--grep '*x*'"), Some(("*x*".into(), None, false)));
        assert_eq!(parse_grep_window("Mud"), None);
    }
}
