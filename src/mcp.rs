//! MCP (MUD Client Protocol) 2.1 — plan Job 15 of
//! investigate-differences-between-tinyfugu-fluffy-stallman.md.
//!
//! # Why this is not part of `TelnetSession`
//!
//! Every other Phase 4 protocol (MTTS, MSSP, MSDP, CHARSET, MSP) lives in `telnet.rs`
//! because each is either a real telnet option or a byte-level marker inside the text
//! stream, and `TelnetSession` is deliberately pure/synchronous/App-free (design
//! commitment 1). MCP does not fit that shape:
//!
//! - It is **in-band**: ordinary lines of text that happen to start with the literal
//!   `#$#`. Detecting it needs no telnet option state, no subnegotiation buffer, no
//!   escaping of IAC — it needs *complete decoded lines*, which only exist once telnet
//!   framing, MCCP2 decompression and encoding have already happened.
//! - Its highest-value package (`dns-org-mud-moo-simpleedit`) has to open the note
//!   editor, which is `App` state. A pure session has no way to reach that, and
//!   widening `TelnetSession` to touch `App` would break the "session is pure telnet"
//!   commitment for every protocol, not just this one.
//!
//! So this module is a **line filter**: callers hand it one complete, already-decoded
//! server line at a time (never partial — see `McpState::process_line`'s doc comment)
//! and get back either "this is not MCP, display it" or "consumed, here are zero or
//! more high-level events and zero or more reply lines to send back down the wire".
//! `App::process_server_data` is the hook (see its own doc comment for why that call
//! site, not `add_output`, is the right one) and `App::handle_mcp_event` turns
//! `McpEvent::SimpleEditContent` into an actual editor session.
//!
//! # Security model
//!
//! Every message after the initial handshake must carry Clay's own per-connection
//! authentication key (generated with the same fail-closed `getrandom` path as
//! `App::generate_auth_key`, compared with `util::constant_time_eq`). A message with a
//! wrong or missing key is **inert**: not acted on, and — unlike a line that merely
//! fails to parse as MCP at all — not displayed either, since it *is* well-formed
//! out-of-band syntax, just unauthenticated. This is what stops another player's text
//! (echoed to the client in-band by the MUD, e.g. a `say` containing `#$#...`) from
//! forging protocol messages: only whoever is on the authenticated side of this exact
//! connection ever sees the key. See `McpState::check_auth` and the
//! `wrong_or_missing_key_is_inert_and_hidden` test, which the plan calls out as the
//! most important test in the job.
//!
//! Genuinely malformed input (fails to parse as MCP grammar at all) takes the opposite
//! path: it displays as ordinary — if ugly — text. Silently eating real MUD output
//! because it happened to start with three particular characters is a worse failure
//! than showing something ugly, so the display fallback is the default and hiding is
//! reserved for text that really did parse as an authenticated (or exempt) MCP
//! message. The real spec is stricter than this — literally any `#$#`-prefixed line is
//! "out of band" and hidden regardless of whether it parses further — but Clay
//! deliberately relaxes that in the direction of not losing text.
//!
//! # Multiline accumulation is unbounded on the wire
//!
//! The spec explicitly disclaims any limit ("There is no limit on the number of lines
//! in a multiline value"), so a hostile or buggy server can send continuation lines
//! forever. `MAX_MCP_MULTILINE_BYTES` caps accumulated bytes per open datatag; past it,
//! the buffered lines are dropped and further continuation lines for that tag are
//! silently ignored until the closing `:` line arrives (which removes the pending
//! state but does not dispatch anything) — mirroring `MAX_SUBNEG_BYTES`'s "abandon,
//! don't grow forever, don't corrupt state either" contract for telnet
//! subnegotiations.
//!
//! That per-tag byte cap does nothing about the *number* of simultaneously open
//! tags (T1.3): a server that opens N datatags and never closes any of them grows
//! `McpState::pending` without bound regardless of how small each one stays.
//! `MAX_MCP_OPEN_MULTILINE` bounds the map itself — opening one past the cap
//! evicts the oldest open tag (by insertion order) with a `ProtocolError` naming
//! it, rather than growing further. A later continuation or end line for an
//! evicted tag falls into the ordinary "unknown datatag" path (silently consumed,
//! nothing to dispatch) — there is nothing left to abandon a second time.
//!
//! # Scope
//!
//! Core MCP 2.1 framing (handshake, per-session key, quoted/unquoted values, multiline
//! continuation), `mcp-negotiate` version negotiation, and
//! `dns-org-mud-moo-simpleedit`. Everything else (any other package) is authenticated
//! and consumed like any other out-of-band traffic Clay doesn't implement, but no event
//! is produced for it.
//!
//! Character-class leniency: the formal grammar restricts quoted-value and
//! multiline-payload content to a specific mostly-ASCII character set inherited from
//! MCP's 1990s design. Real MOO content (player names, room descriptions, verb code)
//! is not restricted that way, and rejecting perfectly ordinary Unicode text here would
//! make `@edit` unusable on any character outside that set — far worse than being more
//! permissive than the letter of the spec. Only token boundaries (identifiers, bare
//! unquoted values) enforce the strict `simple-char` set, because that is where the
//! grammar's character-class restriction is actually load-bearing for disambiguation;
//! free-text content (inside quotes, and multiline continuation payloads) accepts any
//! character.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Every MCP message line begins with this.
pub const MCP_PREFIX: &str = "#$#";
/// The "this line isn't really MCP" escape: strip this prefix, display the rest
/// literally, and do not interpret it as MCP even if it then starts with `#$#`.
pub const MCP_ESCAPE_PREFIX: &str = "#$\"";

/// Clay implements exactly this one version of core MCP. Negotiation is therefore
/// always "does the server's [version, to] range include (2, 1)".
const OUR_VERSION: (u32, u32) = (2, 1);

const NEGOTIATE_PACKAGE: &str = "mcp-negotiate";
const NEGOTIATE_MIN: (u32, u32) = (1, 0);
const NEGOTIATE_MAX: (u32, u32) = (1, 0);

const SIMPLEEDIT_PACKAGE: &str = "dns-org-mud-moo-simpleedit";
const SIMPLEEDIT_VERSION: (u32, u32) = (1, 0);

/// Cap on accumulated bytes for one open multiline value (see the module doc comment).
/// Deliberately much larger than telnet's `MAX_SUBNEG_BYTES` (8 KiB): a real `@edit`
/// target is MOO verb code or a description, routinely tens of KB, and simpleedit is
/// the entire payoff of this job — a cap so tight it clips ordinary verbs would defeat
/// the point. 256 KiB is still a hard, bounded limit against a malicious/buggy sender.
pub const MAX_MCP_MULTILINE_BYTES: usize = 262_144;

/// `dns-org-mud-moo-simpleedit`'s `type` field: what kind of text is being edited.
/// Purely informational for Clay (the note editor doesn't render MOO code specially),
/// but round-tripped verbatim into the `-set` reply since the server may care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleEditType {
    Str,
    StringList,
    MooCode,
}

impl SimpleEditType {
    pub fn as_str(self) -> &'static str {
        match self {
            SimpleEditType::Str => "string",
            SimpleEditType::StringList => "string-list",
            SimpleEditType::MooCode => "moo-code",
        }
    }

    /// Unrecognised values fall back to `StringList` (the least structured, safest
    /// interpretation) rather than rejecting the whole message — permissive, matching
    /// this module's general stance (see the module doc comment).
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "string" => SimpleEditType::Str,
            "moo-code" => SimpleEditType::MooCode,
            _ => SimpleEditType::StringList,
        }
    }
}

/// A high-level, actionable outcome of a completed (possibly multiline) MCP message.
#[derive(Debug, Clone, PartialEq)]
pub enum McpEvent {
    /// `dns-org-mud-moo-simpleedit-content`: the server pushed text for the user to
    /// edit. `reference` is opaque and must be echoed back verbatim in the eventual
    /// `-set` reply (`McpState::build_simpleedit_set`) — never interpreted, never used
    /// to build a filesystem path (it is server-supplied and untrusted; the note
    /// editor it reuses never touches the filesystem for this content at all, so there
    /// is no path-containment question the way MSP's local filenames had one).
    SimpleEditContent {
        reference: String,
        name: String,
        edit_type: SimpleEditType,
        content: String,
    },
    /// Non-fatal diagnostic (e.g. a multiline value abandoned past
    /// `MAX_MCP_MULTILINE_BYTES`) — callers log it (`debug_log`), same convention as
    /// `TelnetEvent::ProtocolError`.
    ProtocolError(String),
}

/// Result of feeding one complete decoded line to `McpState::process_line`.
#[derive(Debug, Clone, PartialEq)]
pub enum LineOutcome {
    /// Not valid MCP (or the `#$"` escape form) — display this text. For the escape
    /// form this is the line with the `#$"` prefix stripped; otherwise it is the
    /// original line unchanged.
    Display(String),
    /// Consumed as MCP framing/negotiation/package traffic: never display it.
    /// `replies` are complete wire lines (already correctly quoted/escaped) the caller
    /// should send back down this world's connection, in order. `events` are zero or
    /// more high-level events for the caller to act on.
    Consumed {
        replies: Vec<String>,
        events: Vec<McpEvent>,
    },
}

impl LineOutcome {
    fn consumed() -> Self {
        LineOutcome::Consumed { replies: Vec::new(), events: Vec::new() }
    }
}

/// One parsed `key[*]: value` pair from a message-start line.
struct Field {
    /// Lowercased (keywords are case-insensitive per spec).
    key: String,
    /// True if the key was written `key*` (declares a multiline value).
    multiline: bool,
    /// Resolved value text (case preserved). Meaningless when `multiline` is true —
    /// the spec's own convention is the placeholder is always `""`.
    value: String,
}

/// State for one open multiline value, keyed by its datatag. The datatag "is used in
/// place of the authentication key" for continuation/end lines (spec section 2.2.3):
/// only reachable at all by first authenticating the message-start line that declared
/// it, so no separate per-line auth check is needed here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PendingMultiline {
    message_name: String,
    /// Every non-multiline field from the original message-start line, `_data-tag`
    /// itself excluded (it is plumbing, never part of the dispatched field set).
    fields: Vec<(String, String)>,
    multiline_key: String,
    lines: Vec<String>,
    byte_len: usize,
    /// Past `MAX_MCP_MULTILINE_BYTES`: stop accumulating, stop dispatching on close,
    /// but keep the entry so continuation/end lines for this tag keep being silently
    /// consumed (never fall through to display) until the sender's own `:` closes it.
    abandoned: bool,
    /// T1.3: insertion order, from `McpState::next_open_seq`. `MAX_MCP_MULTILINE_BYTES`
    /// caps bytes *per open tag*, but nothing capped how many tags could be open at
    /// once — a server opening tags and never closing them grows `McpState::pending`
    /// without bound. This is what lets `finish_message_start` find "the oldest still
    /// open" in O(n) over the (small, capped) map to evict when a new tag would push
    /// past `MAX_MCP_OPEN_MULTILINE`.
    opened_seq: u64,
}

/// Per-world, per-connection MCP state. Lives on `World` (see `World::mcp`), reset by
/// `clear_connection_state` on every disconnect/reconnect — a stale key or half-open
/// multiline value from a previous connection must never be trusted against a new one.
///
/// Persisted across a hot reload (T3.4/job 5), `#[derive(Serialize, Deserialize)]`
/// as one `mcp_json=` line in the reload state file: the socket survives an `exec`,
/// and unlike a real reconnect the server has no reason to resend its one-time "mcp
/// version:" invite mid-session, so there is no natural recovery path at all — Clay
/// is purely reactive and never prompts the server side of MCP (or any other telnet
/// option) to redo its own negotiation. Left unpersisted, MCP silently and
/// permanently stopped working on that connection — unlike MCCP2, this does not
/// corrupt the byte stream, it just went quiet with no way back short of a real
/// reconnect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpState {
    /// `None` until a satisfiable version handshake completes. `Some(_)` is
    /// "authenticated": every subsequent message must carry this exact key.
    key: Option<String>,
    /// Recorded from an authenticated `mcp-negotiate-can` advertising an overlapping
    /// range for `dns-org-mud-moo-simpleedit`. Not currently gated on (see
    /// `dispatch`'s doc comment on `simpleedit-content` for why receiving does not
    /// require it) — kept for diagnostics and tests, and so a future package that
    /// *does* need to check it has the state already threaded through.
    pub(crate) simpleedit_negotiated: bool,
    pending: HashMap<String, PendingMultiline>,
    /// Counter for datatags Clay itself generates when sending a multiline value
    /// (`build_simpleedit_set`). Does not need to be unpredictable — the datatag isn't
    /// protecting anything on Clay's outbound side, it only has to be unique for the
    /// life of this connection so multiple saves can't collide.
    next_out_tag: u64,
    /// Monotonic counter stamped onto each `PendingMultiline::opened_seq` as it's
    /// inserted (T1.3) — an incoming-datatag analogue of `next_out_tag`, used purely
    /// to find "the oldest open tag" for eviction, never sent on the wire.
    next_open_seq: u64,
}

/// Cap on the number of simultaneously open multiline values (T1.3; see the module
/// doc comment's "Multiline accumulation is unbounded on the wire" section, and
/// `MAX_MCP_MULTILINE_BYTES` just above, which only bounds bytes *per tag*). A real
/// `@edit` session opens one tag at a time; 16 leaves generous headroom for a client
/// with several edit windows in flight while still bounding a server that opens tags
/// and never closes them.
pub const MAX_MCP_OPEN_MULTILINE: usize = 16;

impl McpState {
    /// Feed one complete, already-decoded server line. Must be a full line (no
    /// trailing partial) — the caller is expected to have already reassembled a line
    /// split across TCP reads before this is called (`App::process_server_data`'s
    /// existing partial-line carry-over does this for free, since MCP detection runs
    /// on the same fully-reassembled `&str` lines that action triggers see).
    ///
    /// Callers should only call this for a line that starts with [`MCP_PREFIX`] or
    /// [`MCP_ESCAPE_PREFIX`] — anything else is definitionally not MCP and does not
    /// need the round trip through here. It is still safe (a no-op passthrough) to
    /// call on any other line.
    pub fn process_line(&mut self, line: &str) -> LineOutcome {
        if let Some(rest) = line.strip_prefix(MCP_ESCAPE_PREFIX) {
            // Spec section 2.1: this is the one escape hatch for in-band text that
            // would otherwise be misread as out-of-band. Pure passthrough — never
            // re-examined for MCP syntax even if the unescaped text itself starts
            // with "#$#".
            return LineOutcome::Display(rest.to_string());
        }
        let Some(rest) = line.strip_prefix(MCP_PREFIX) else {
            return LineOutcome::Display(line.to_string());
        };
        if let Some(cont) = rest.strip_prefix('*') {
            return self.handle_continuation(cont, line);
        }
        if let Some(end) = rest.strip_prefix(':') {
            return self.handle_end(end, line);
        }
        self.handle_message_start_line(rest, line)
    }

    /// Filter a chunk of decoded server text (`App::process_server_data`'s
    /// `combined_data` — already reassembled across any partial-line carry-over from a
    /// previous read) line by line, replacing/removing MCP traffic and leaving
    /// everything else untouched byte-for-byte, including whether the text ends with a
    /// trailing newline. The final line is treated as a still-incomplete partial (and
    /// left completely untouched, not fed to `process_line`) exactly when `text` does
    /// not itself end with `\n` — matching the caller's own definition of "partial" so
    /// a `#$#` line split across two reads is never examined until it is whole. This
    /// is the integration point real callers use; `process_line` (single, already
    /// complete line) is the lower-level primitive the tests exercise directly.
    pub fn filter_lines(&mut self, text: &str) -> (String, Vec<String>, Vec<McpEvent>) {
        let ends_with_newline = text.ends_with('\n');
        let mut out = String::with_capacity(text.len());
        let mut replies = Vec::new();
        let mut events = Vec::new();
        let lines: Vec<&str> = text.lines().collect();
        let last_idx = lines.len().checked_sub(1);
        for (i, line) in lines.iter().enumerate() {
            let is_partial = last_idx == Some(i) && !ends_with_newline;
            if !is_partial && (line.starts_with(MCP_PREFIX) || line.starts_with(MCP_ESCAPE_PREFIX)) {
                match self.process_line(line) {
                    LineOutcome::Display(displayed) => {
                        out.push_str(&displayed);
                        out.push('\n');
                    }
                    LineOutcome::Consumed { replies: r, events: e } => {
                        replies.extend(r);
                        events.extend(e);
                    }
                }
            } else {
                out.push_str(line);
                if !is_partial {
                    out.push('\n');
                }
            }
        }
        (out, replies, events)
    }

    fn handle_message_start_line(&mut self, rest: &str, original: &str) -> LineOutcome {
        let name_end = rest.find(' ').unwrap_or(rest.len());
        let name_raw = &rest[..name_end];
        if !is_valid_ident(name_raw) {
            return LineOutcome::Display(original.to_string());
        }
        let name = name_raw.to_ascii_lowercase();
        let after_name = if name_end < rest.len() { &rest[name_end + 1..] } else { "" };
        match parse_keyvals_and_authkey(after_name) {
            None => LineOutcome::Display(original.to_string()),
            Some((auth_key, fields)) => self.finish_message_start(&name, auth_key.as_deref(), fields),
        }
    }

    fn finish_message_start(&mut self, name: &str, auth_key: Option<&str>, fields: Vec<Field>) -> LineOutcome {
        if name == "mcp" {
            return self.handle_mcp_bootstrap(&fields);
        }
        if !self.check_auth(auth_key) {
            // Well-formed MCP syntax, wrong/missing key: inert AND hidden. See the
            // module doc comment's security section - this is the load-bearing case.
            return LineOutcome::consumed();
        }
        let multiline_field = fields.iter().find(|f| f.multiline).map(|f| f.key.clone());
        let simple_fields: Vec<(String, String)> = fields.iter()
            .filter(|f| !f.multiline)
            .map(|f| (f.key.clone(), f.value.clone()))
            .collect();
        match multiline_field {
            None => {
                let (replies, events) = self.dispatch(name, &simple_fields);
                LineOutcome::Consumed { replies, events }
            }
            Some(multiline_key) => {
                let datatag = simple_fields.iter().find(|(k, _)| k == "_data-tag").map(|(_, v)| v.clone());
                match datatag {
                    // Declared a multiline field but gave no way to correlate its
                    // continuation lines - can't do anything with this message, but it
                    // was still authenticated real MCP syntax, so still hidden.
                    None => LineOutcome::consumed(),
                    Some(tag) => {
                        // T1.3: only an actually-new tag grows the map — re-using an
                        // already-open tag (overwriting its entry, same as before)
                        // must not trigger an eviction of something else.
                        let mut events = Vec::new();
                        if !self.pending.contains_key(&tag) && self.pending.len() >= MAX_MCP_OPEN_MULTILINE {
                            if let Some(oldest_tag) = self.pending.iter()
                                .min_by_key(|(_, pm)| pm.opened_seq)
                                .map(|(k, _)| k.clone())
                            {
                                self.pending.remove(&oldest_tag);
                                events.push(McpEvent::ProtocolError(format!(
                                    "mcp: too many open multiline values (limit {MAX_MCP_OPEN_MULTILINE}), abandoned oldest datatag '{oldest_tag}'",
                                )));
                            }
                        }
                        let opened_seq = self.next_open_seq;
                        self.next_open_seq += 1;
                        self.pending.insert(tag, PendingMultiline {
                            message_name: name.to_string(),
                            fields: simple_fields.into_iter().filter(|(k, _)| k != "_data-tag").collect(),
                            multiline_key,
                            lines: Vec::new(),
                            byte_len: 0,
                            abandoned: false,
                            opened_seq,
                        });
                        LineOutcome::Consumed { replies: Vec::new(), events }
                    }
                }
            }
        }
    }

    /// The one pair of messages exempt from the authentication-key check: the
    /// server's initial `mcp version: X to: Y` invite. On a satisfiable overlap,
    /// generates our key (fail-closed - see `generate_session_key`), stores it, and
    /// returns our reply plus an immediate `mcp-negotiate-can`/`mcp-negotiate-end`
    /// batch advertising `mcp-negotiate` and `dns-org-mud-moo-simpleedit` (spec:
    /// implementations need not wait for the peer's own `mcp-negotiate-end` before
    /// advertising their own capabilities).
    ///
    /// T1.6: a bootstrap is only ever legitimate once, at the start of the
    /// connection. Because this is the one message exempt from `check_auth`, nothing
    /// else stops a replay — from the server itself, or echoed in-band from another
    /// player's text at start-of-line — from regenerating the session key. Every
    /// later genuine message is signed with the *old* key and would then fail
    /// `check_auth` and be silently consumed and hidden (the same fate as any
    /// wrong-key message — see the module doc comment's security section), killing
    /// MCP for the rest of the connection with no diagnostic. Once a key exists, a
    /// second bootstrap is ignored outright: same posture as a wrong/missing key
    /// elsewhere in this module — well-formed MCP, but inert and hidden.
    fn handle_mcp_bootstrap(&mut self, fields: &[Field]) -> LineOutcome {
        if self.key.is_some() {
            return LineOutcome::consumed();
        }
        let version = fields.iter().find(|f| f.key == "version").and_then(|f| parse_version(&f.value));
        let to = fields.iter().find(|f| f.key == "to").and_then(|f| parse_version(&f.value));
        let (Some(v), Some(t)) = (version, to) else {
            // Grammatically a valid message-start, but we can't read its version
            // range - can't negotiate, so treat like an unsatisfiable range: hidden,
            // no reply, MCP stays unavailable this connection.
            return LineOutcome::consumed();
        };
        if v > t || !(v <= OUR_VERSION && OUR_VERSION <= t) {
            return LineOutcome::consumed();
        }
        let Some(key) = generate_session_key() else {
            // Fail closed (C2 posture, same as App::generate_auth_key): no weak key,
            // no MCP this connection, rather than proceeding without real entropy.
            return LineOutcome::consumed();
        };
        self.key = Some(key.clone());
        let replies = vec![
            message_line("mcp", None, &[
                kv("authentication-key", &key),
                kv("version", "2.1"),
                kv("to", "2.1"),
            ]),
            message_line("mcp-negotiate-can", Some(&key), &[
                kv("package", NEGOTIATE_PACKAGE),
                kv("min-version", &version_str(NEGOTIATE_MIN)),
                kv("max-version", &version_str(NEGOTIATE_MAX)),
            ]),
            message_line("mcp-negotiate-can", Some(&key), &[
                kv("package", SIMPLEEDIT_PACKAGE),
                kv("min-version", &version_str(SIMPLEEDIT_VERSION)),
                kv("max-version", &version_str(SIMPLEEDIT_VERSION)),
            ]),
            message_line("mcp-negotiate-end", Some(&key), &[]),
        ];
        LineOutcome::Consumed { replies, events: Vec::new() }
    }

    fn check_auth(&self, auth_key: Option<&str>) -> bool {
        match (&self.key, auth_key) {
            (Some(k), Some(a)) => crate::util::constant_time_eq(k.as_bytes(), a.as_bytes()),
            _ => false,
        }
    }

    /// Act on a fully-resolved message (either an immediate one-line message, or one
    /// whose multiline value just closed). `fields` never includes `_data-tag`.
    fn dispatch(&mut self, name: &str, fields: &[(String, String)]) -> (Vec<String>, Vec<McpEvent>) {
        let get = |k: &str| fields.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.clone());
        match name {
            "mcp-negotiate-can" => {
                let package = get("package");
                let lo = get("min-version").and_then(|v| parse_version(&v));
                let hi = get("max-version").and_then(|v| parse_version(&v));
                if let (Some(pkg), Some(lo), Some(hi)) = (package, lo, hi) {
                    if pkg.eq_ignore_ascii_case(SIMPLEEDIT_PACKAGE) && lo <= SIMPLEEDIT_VERSION && SIMPLEEDIT_VERSION <= hi {
                        self.simpleedit_negotiated = true;
                    }
                }
                (Vec::new(), Vec::new())
            }
            // No argument, no action needed - Clay never waits for this before using a
            // package it has already advertised, per spec.
            "mcp-negotiate-end" => (Vec::new(), Vec::new()),
            "dns-org-mud-moo-simpleedit-content" => {
                // Receiving is not gated on `simpleedit_negotiated` - that flag tracks
                // whether the SERVER also claims to support this package, which the
                // spec never makes a precondition for the server sending it once Clay
                // has advertised support (which happens unconditionally, right after
                // the handshake, in `handle_mcp_bootstrap`).
                let edit_type = get("type").map(|v| SimpleEditType::parse(&v)).unwrap_or(SimpleEditType::StringList);
                // Defensive fallback: accept a plain non-multiline `content` value too,
                // even though the spec always declares it `content*`. Real-world
                // robustness over strict conformance, same stance as the module doc
                // comment's character-class leniency.
                let content = get("content").unwrap_or_default();
                match (get("reference"), get("name")) {
                    (Some(reference), Some(name)) => {
                        (Vec::new(), vec![McpEvent::SimpleEditContent { reference, name, edit_type, content }])
                    }
                    _ => (Vec::new(), vec![McpEvent::ProtocolError(
                        "dns-org-mud-moo-simpleedit-content missing reference or name".to_string(),
                    )]),
                }
            }
            // A message name we don't implement, from a package we may or may not
            // have even advertised. Authenticated real MCP traffic all the same -
            // consumed, but nothing to act on.
            _ => (Vec::new(), Vec::new()),
        }
    }

    fn handle_continuation(&mut self, cont: &str, original: &str) -> LineOutcome {
        // `<message-continue> ::= '*' <space> <datatag> <space> <simple-key> ':' ' ' <line>`
        let Some(rest) = cont.strip_prefix(' ') else {
            return LineOutcome::Display(original.to_string());
        };
        let tag_end = match rest.find(' ') {
            Some(i) => i,
            None => return LineOutcome::Display(original.to_string()),
        };
        let tag = &rest[..tag_end];
        if tag.is_empty() || !tag.chars().all(is_simple_char) {
            return LineOutcome::Display(original.to_string());
        }
        let after_tag = &rest[tag_end + 1..];
        let key_end = match after_tag.find(':') {
            Some(i) => i,
            None => return LineOutcome::Display(original.to_string()),
        };
        let key = &after_tag[..key_end];
        if !is_valid_ident(key) {
            return LineOutcome::Display(original.to_string());
        }
        let after_key = &after_tag[key_end + 1..];
        let Some(data) = after_key.strip_prefix(' ') else {
            return LineOutcome::Display(original.to_string());
        };
        if let Some(pm) = self.pending.get_mut(tag) {
            if !pm.abandoned {
                pm.byte_len += data.len() + 1;
                if pm.byte_len > MAX_MCP_MULTILINE_BYTES {
                    pm.abandoned = true;
                    pm.lines = Vec::new(); // free the buffer, clean abandon
                } else {
                    pm.lines.push(data.to_string());
                }
            }
        }
        // Unknown tag: still genuinely-shaped MCP continuation syntax - consumed, not
        // displayed, even though there's nothing to correlate it to.
        LineOutcome::consumed()
    }

    fn handle_end(&mut self, end: &str, original: &str) -> LineOutcome {
        // `<message-end> ::= ':' <space> <datatag>`
        let Some(rest) = end.strip_prefix(' ') else {
            return LineOutcome::Display(original.to_string());
        };
        let tag = rest.trim_end();
        if tag.is_empty() || !tag.chars().all(is_simple_char) {
            return LineOutcome::Display(original.to_string());
        }
        match self.pending.remove(tag) {
            None => LineOutcome::consumed(),
            Some(pm) if pm.abandoned => LineOutcome::Consumed {
                replies: Vec::new(),
                events: vec![McpEvent::ProtocolError(format!(
                    "mcp '{}' multiline value exceeded {} bytes, abandoned",
                    pm.message_name, MAX_MCP_MULTILINE_BYTES,
                ))],
            },
            Some(pm) => {
                let mut fields = pm.fields;
                fields.push((pm.multiline_key, pm.lines.join("\n")));
                let (replies, events) = self.dispatch(&pm.message_name, &fields);
                LineOutcome::Consumed { replies, events }
            }
        }
    }

    /// Test/diagnostic-only: whether a session key has been established. Production
    /// code never needs to ask this (`build_simpleedit_set` already fails closed when
    /// there is no key), but integration tests in `tests.rs` exercising the real
    /// `App::process_server_data` pipeline need a way to observe it from outside this
    /// module.
    #[cfg(test)]
    pub(crate) fn has_key(&self) -> bool {
        self.key.is_some()
    }

    /// Test-only: the session key itself, for a test outside this module (job 5's
    /// hot-reload roundtrip in `persistence.rs`) that needs to sign a follow-up
    /// message after restoring a serialized `McpState` it never negotiated by hand.
    #[cfg(test)]
    pub(crate) fn key_for_test(&self) -> Option<String> {
        self.key.clone()
    }

    /// Test-only: number of currently-open multiline datatags (T1.3). Lets a test
    /// observe `MAX_MCP_OPEN_MULTILINE` eviction without reaching into the private
    /// `pending` map directly.
    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Build the wire lines for a `dns-org-mud-moo-simpleedit-set` reply: the
    /// message-start line (reference/type/multiline placeholder/datatag), one
    /// continuation line per line of `content` (verbatim — see the module doc comment
    /// on why multiline payload data needs no escaping), then the closing line.
    /// `content` is untrusted round-tripped user-edited text and is never interpreted
    /// here, only framed.
    ///
    /// Returns `None` if MCP was never authenticated on this connection (no key to
    /// sign the message with) — the caller (the note editor's save path) should tell
    /// the user the edit could not be sent rather than emitting an unauthenticated
    /// line the server would just discard.
    pub fn build_simpleedit_set(&mut self, reference: &str, edit_type: SimpleEditType, content: &str) -> Option<Vec<String>> {
        let key = self.key.clone()?;
        self.next_out_tag += 1;
        let tag = format!("out{}", self.next_out_tag);
        let mut lines = vec![message_line("dns-org-mud-moo-simpleedit-set", Some(&key), &[
            kv("reference", reference),
            kv("type", edit_type.as_str()),
            format!("content*: {}", quote_if_needed("")),
            kv("_data-tag", &tag),
        ])];
        for line in content.split('\n') {
            lines.push(format!("{MCP_PREFIX}* {tag} content: {line}"));
        }
        lines.push(format!("{MCP_PREFIX}: {tag}"));
        Some(lines)
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_ident_start(c) => chars.all(is_ident_char),
        _ => false,
    }
}

/// The grammar's `<simple-char>` set: alphanumerics plus a fixed punctuation list.
/// Notably excludes space, quote, backslash, colon and asterisk — those five are
/// exactly what forces a value to be quoted.
fn is_simple_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '-' | '~' | '`' | '!' | '@' | '#' | '$' | '%' | '^' | '&' | '('
            | ')' | '=' | '+' | '{' | '}' | '[' | ']' | '|' | '\'' | ';' | '?' | '/' | '>'
            | '<' | '.' | ',')
}

/// Parse everything after a message-start line's name and its single mandatory
/// separating space (already stripped by the caller) into an optional positional
/// auth-key token and the keyval list. Returns `None` on any grammar violation — the
/// whole line must then be displayed, not partially trusted.
fn parse_keyvals_and_authkey(rest: &str) -> Option<(Option<String>, Vec<Field>)> {
    let rest = rest.trim_start_matches(' ');
    if rest.is_empty() {
        return Some((None, Vec::new()));
    }
    // The auth-key token (if present) can never contain ':' (it's built from
    // simple-char, which excludes it), while a key token always ends in exactly one
    // ':' with nothing else before it - so peeking at whether the first raw
    // whitespace-delimited token ends with ':' unambiguously tells them apart.
    let first_token_end = rest.find(' ').unwrap_or(rest.len());
    let first_token = &rest[..first_token_end];
    let (auth_key, kv_rest) = if first_token.ends_with(':') {
        (None, rest)
    } else {
        if first_token.is_empty() || !first_token.chars().all(is_simple_char) {
            return None;
        }
        (Some(first_token.to_string()), rest[first_token_end..].trim_start_matches(' '))
    };
    let fields = parse_fields(kv_rest)?;
    Some((auth_key, fields))
}

fn parse_fields(mut s: &str) -> Option<Vec<Field>> {
    let mut fields = Vec::new();
    s = s.trim_start_matches(' ');
    while !s.is_empty() {
        let key_end = s.find(|c: char| !is_ident_char(c)).unwrap_or(s.len());
        if key_end == 0 || !is_valid_ident(&s[..key_end]) {
            return None;
        }
        let key = s[..key_end].to_ascii_lowercase();
        let mut rest = &s[key_end..];
        let multiline = rest.starts_with('*');
        if multiline {
            rest = &rest[1..];
        }
        rest = rest.strip_prefix(':')?;
        rest = rest.strip_prefix(' ')?;
        let (value, after) = if let Some(after_quote) = rest.strip_prefix('"') {
            parse_quoted(after_quote)?
        } else {
            let end = rest.find(' ').unwrap_or(rest.len());
            let raw = &rest[..end];
            if raw.is_empty() || !raw.chars().all(is_simple_char) {
                return None;
            }
            (raw.to_string(), &rest[end..])
        };
        if !after.is_empty() && !after.starts_with(' ') {
            // Garbage stuck immediately after a value with no separating space.
            return None;
        }
        fields.push(Field { key, multiline, value });
        s = after.trim_start_matches(' ');
    }
    Some(fields)
}

/// Parse a quoted value's content, `s` being everything after the opening `"`. Returns
/// the resolved text and the remainder of the string starting right after the closing
/// `"`. `None` on an unterminated string or an escape sequence other than `\"`/`\\`.
fn parse_quoted(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    loop {
        let (i, c) = chars.next()?;
        match c {
            '"' => return Some((out, &s[i + c.len_utf8()..])),
            '\\' => {
                let (_, esc) = chars.next()?;
                match esc {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    _ => return None,
                }
            }
            other => out.push(other),
        }
    }
}

fn parse_version(s: &str) -> Option<(u32, u32)> {
    let (maj, min) = s.split_once('.')?;
    Some((maj.parse().ok()?, min.parse().ok()?))
}

fn version_str((maj, min): (u32, u32)) -> String {
    format!("{maj}.{min}")
}

/// Quote `v` iff it is empty or contains a character outside `simple-char` — the exact
/// condition the spec gives for when quoting is required, applied symmetrically to
/// everything Clay itself sends.
fn quote_if_needed(v: &str) -> String {
    if !v.is_empty() && v.chars().all(is_simple_char) {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn kv(key: &str, value: &str) -> String {
    format!("{key}: {}", quote_if_needed(value))
}

fn message_line(name: &str, auth_key: Option<&str>, parts: &[String]) -> String {
    let mut s = format!("{MCP_PREFIX}{name}");
    if let Some(a) = auth_key {
        s.push(' ');
        s.push_str(a);
    }
    for p in parts {
        s.push(' ');
        s.push_str(p);
    }
    s
}

/// Fail-closed per-session key generation (C2 posture, mirroring
/// `App::generate_auth_key` exactly): a `getrandom` failure must not silently degrade
/// to a time/pid-derived guessable key, since this key is the only thing standing
/// between a real MOO `@edit` push and a hostile third party forging the same
/// messages in-band.
fn generate_session_key() -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut random_bytes = [0u8; 16];
    if getrandom::getrandom(&mut random_bytes).is_err() {
        crate::debug_log(true, "MCP SESSION KEY GENERATION FAILED: getrandom error, refusing to enable MCP for this connection");
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(random_bytes);
    Some(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handshake(state: &mut McpState, version: &str, to: &str) -> LineOutcome {
        state.process_line(&format!("{MCP_PREFIX}mcp version: {version} to: {to}"))
    }

    fn assert_hidden_no_events(outcome: &LineOutcome) {
        match outcome {
            LineOutcome::Consumed { events, .. } => assert!(events.is_empty(), "expected no events, got {events:?}"),
            LineOutcome::Display(text) => panic!("expected the message to be hidden, but it displayed: {text:?}"),
        }
    }

    // ==================================================================
    // Handshake
    // ==================================================================

    #[test]
    fn handshake_negotiates_and_replies_with_our_key() {
        let mut state = McpState::default();
        let outcome = handshake(&mut state, "1.0", "2.1");
        let LineOutcome::Consumed { replies, events } = outcome else { panic!("expected Consumed") };
        assert!(events.is_empty());
        assert_eq!(replies.len(), 4, "reply, 2 negotiate-can, negotiate-end: {replies:?}");
        let key = state.key.clone().expect("key must be established");
        assert!(replies[0].starts_with(&format!("{MCP_PREFIX}mcp authentication-key: {key} version: 2.1 to: 2.1")));
        assert!(replies[1].contains(&format!("{MCP_PREFIX}mcp-negotiate-can {key} package: mcp-negotiate")));
        assert!(replies[2].contains(&format!("package: {SIMPLEEDIT_PACKAGE} min-version: 1.0 max-version: 1.0")));
        assert_eq!(replies[3], format!("{MCP_PREFIX}mcp-negotiate-end {key}"));
    }

    #[test]
    fn handshake_version_range_we_cannot_satisfy_is_silently_consumed_with_no_reply() {
        let mut state = McpState::default();
        let outcome = handshake(&mut state, "1.0", "1.0");
        assert_eq!(outcome, LineOutcome::Consumed { replies: Vec::new(), events: Vec::new() });
        assert!(state.key.is_none(), "no overlap must mean no key is ever established");

        // Once negotiation has failed, every subsequent #$# line lacks a key to check
        // against - matches the "missing key" bucket, still hidden, never displayed.
        let follow_up = state.process_line(&format!("{MCP_PREFIX}mcp-negotiate-can somekey package: {SIMPLEEDIT_PACKAGE} min-version: 1.0 max-version: 1.0"));
        assert_hidden_no_events(&follow_up);
    }

    #[test]
    fn handshake_reversed_range_is_treated_as_unsatisfiable() {
        let mut state = McpState::default();
        let outcome = handshake(&mut state, "3.0", "1.0");
        assert_eq!(outcome, LineOutcome::Consumed { replies: Vec::new(), events: Vec::new() });
        assert!(state.key.is_none());
    }

    // T1.6: a replayed bootstrap must not reset the session key - either the server
    // resends its own invite mid-session, or another player's in-band text happens
    // to echo one back verbatim.
    #[test]
    fn bootstrap_replay_does_not_reset_key() {
        let (mut state, original_key) = authenticated_state();

        let replay = handshake(&mut state, "2.1", "2.1");
        assert_eq!(replay, LineOutcome::Consumed { replies: Vec::new(), events: Vec::new() },
            "a replayed bootstrap must be consumed with no reply, not renegotiate");
        assert_eq!(state.key.as_deref(), Some(original_key.as_str()), "the key must not change");

        // A message signed with the ORIGINAL key must still authenticate and
        // dispatch - proves the replay didn't quietly invalidate every later
        // genuine message (the bug: a regenerated key made every subsequent real
        // message fail check_auth and vanish with no diagnostic).
        let outcome = state.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {original_key} reference: r name: n type: string content: \"hi\""
        ));
        let LineOutcome::Consumed { events, .. } = outcome else { panic!("expected Consumed") };
        assert_eq!(events.len(), 1, "message signed with the original key must still dispatch after a replayed bootstrap");
    }

    // ==================================================================
    // Argument parsing: quoted, unquoted, malformed
    // ==================================================================

    #[test]
    fn quoted_value_with_embedded_space_and_escaped_quote() {
        let fields = parse_fields(r#"name: "Joe says \"hi\" to you""#).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].key, "name");
        assert_eq!(fields[0].value, "Joe says \"hi\" to you");
    }

    #[test]
    fn unquoted_argument_parses() {
        let fields = parse_fields("version: 2.1 to: 2.1").unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, "version");
        assert_eq!(fields[0].value, "2.1");
        assert_eq!(fields[1].key, "to");
        assert_eq!(fields[1].value, "2.1");
    }

    #[test]
    fn unquoted_value_cannot_contain_a_colon_or_quote() {
        // ':' and '"' are excluded from simple-char, so an unquoted token containing
        // them must fail to parse as a bare value.
        assert!(parse_fields("k: va:lue").is_none());
    }

    #[test]
    fn malformed_message_displays_rather_than_vanishing() {
        let mut state = McpState::default();
        // Looks MCP-shaped but the key token has no trailing colon anywhere and the
        // "keyval" that follows has no colon either - not valid MCP grammar at all.
        let line = format!("{MCP_PREFIX}mcp-negotiate-can key package simpleedit");
        let outcome = state.process_line(&line);
        assert_eq!(outcome, LineOutcome::Display(line));
    }

    #[test]
    fn malformed_unterminated_quote_displays() {
        let mut state = McpState::default();
        let line = format!(r#"{MCP_PREFIX}mcp-negotiate-can key package: "unterminated"#);
        let outcome = state.process_line(&line);
        assert_eq!(outcome, LineOutcome::Display(line));
    }

    #[test]
    fn ordinary_text_is_untouched() {
        let mut state = McpState::default();
        let line = "You see a sword here.";
        assert_eq!(state.process_line(line), LineOutcome::Display(line.to_string()));
    }

    // ==================================================================
    // #$" escaping
    // ==================================================================

    #[test]
    fn escape_prefix_unescapes_and_bypasses_mcp_interpretation() {
        let mut state = McpState::default();
        let line = format!("{MCP_ESCAPE_PREFIX}{MCP_PREFIX}this isn't: really an: \"out-of-band message\"");
        let outcome = state.process_line(&line);
        assert_eq!(outcome, LineOutcome::Display(format!("{MCP_PREFIX}this isn't: really an: \"out-of-band message\"")));
        // Must not have been interpreted as MCP even though the unescaped text starts
        // with "#$#" and would otherwise parse as a (malformed) message-start.
        assert!(state.key.is_none());
        assert!(state.pending.is_empty());
    }

    // ==================================================================
    // Multiline accumulation, negotiate, and simpleedit dispatch
    // ==================================================================

    fn authenticated_state() -> (McpState, String) {
        let mut state = McpState::default();
        handshake(&mut state, "2.1", "2.1");
        let key = state.key.clone().unwrap();
        (state, key)
    }

    #[test]
    fn mcp_negotiate_can_records_simpleedit_support_only_on_overlap() {
        let (mut state, key) = authenticated_state();
        assert!(!state.simpleedit_negotiated);
        let no_overlap = state.process_line(&format!(
            "{MCP_PREFIX}mcp-negotiate-can {key} package: {SIMPLEEDIT_PACKAGE} min-version: 2.0 max-version: 3.0"
        ));
        assert_hidden_no_events(&no_overlap);
        assert!(!state.simpleedit_negotiated, "1.0 is outside [2.0, 3.0], must not set the flag");

        let overlap = state.process_line(&format!(
            "{MCP_PREFIX}mcp-negotiate-can {key} package: {SIMPLEEDIT_PACKAGE} min-version: 1.0 max-version: 1.0"
        ));
        assert_hidden_no_events(&overlap);
        assert!(state.simpleedit_negotiated);
    }

    #[test]
    fn multiline_accumulation_and_dispatch_produces_simpleedit_content() {
        let (mut state, key) = authenticated_state();
        let start = state.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {key} reference: \"#72.name\" name: \"Joe's name\" type: string content*: \"\" _data-tag: t1"
        ));
        assert_hidden_no_events(&start);

        let c1 = state.process_line(&format!("{MCP_PREFIX}* t1 content: first line"));
        assert_hidden_no_events(&c1);
        let c2 = state.process_line(&format!("{MCP_PREFIX}* t1 content: "));
        assert_hidden_no_events(&c2); // an empty continuation line is a real blank line
        let c3 = state.process_line(&format!("{MCP_PREFIX}* t1 content: third line"));
        assert_hidden_no_events(&c3);

        let end = state.process_line(&format!("{MCP_PREFIX}: t1"));
        let LineOutcome::Consumed { events, .. } = end else { panic!("expected Consumed") };
        assert_eq!(events.len(), 1);
        match &events[0] {
            McpEvent::SimpleEditContent { reference, name, edit_type, content } => {
                assert_eq!(reference, "#72.name");
                assert_eq!(name, "Joe's name");
                assert_eq!(*edit_type, SimpleEditType::Str);
                assert_eq!(content, "first line\n\nthird line");
            }
            other => panic!("expected SimpleEditContent, got {other:?}"),
        }
    }

    #[test]
    fn simpleedit_content_accepts_non_multiline_fallback() {
        let (mut state, key) = authenticated_state();
        let outcome = state.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {key} reference: r1 name: r1 type: moo-code content: \"one liner\""
        ));
        let LineOutcome::Consumed { events, .. } = outcome else { panic!("expected Consumed") };
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], McpEvent::SimpleEditContent { content, edit_type: SimpleEditType::MooCode, .. } if content == "one liner"));
    }

    #[test]
    fn multiline_size_cap_abandons_cleanly() {
        let (mut state, key) = authenticated_state();
        state.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {key} reference: r1 name: r1 type: string-list content*: \"\" _data-tag: big"
        ));
        // One 1KiB line per continuation; comfortably exceed MAX_MCP_MULTILINE_BYTES.
        let chunk = "x".repeat(1024);
        let needed = MAX_MCP_MULTILINE_BYTES / chunk.len() + 4;
        for _ in 0..needed {
            let outcome = state.process_line(&format!("{MCP_PREFIX}* big content: {chunk}"));
            assert_hidden_no_events(&outcome); // never displays, even mid-abandon
        }
        let end = state.process_line(&format!("{MCP_PREFIX}: big"));
        let LineOutcome::Consumed { replies, events } = end else { panic!("expected Consumed") };
        assert!(replies.is_empty());
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], McpEvent::ProtocolError(msg) if msg.contains("abandoned")),
            "expected a ProtocolError, got {events:?}");
    }

    // ==================================================================
    // T1.3: capping the number of simultaneously open multiline datatags.
    // ==================================================================

    #[test]
    fn open_multiline_count_is_capped() {
        let (mut state, key) = authenticated_state();
        // Open MAX_MCP_OPEN_MULTILINE + 1 tags ("tag0".."tag16"), each declaring the
        // multiline field but never closing it - the unbounded-entry-count DoS.
        for i in 0..(MAX_MCP_OPEN_MULTILINE + 1) {
            let tag = format!("tag{i}");
            let outcome = state.process_line(&format!(
                "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {key} reference: r name: n type: string content*: \"\" _data-tag: {tag}"
            ));
            if i < MAX_MCP_OPEN_MULTILINE {
                assert_hidden_no_events(&outcome);
            } else {
                // The 17th open evicts the oldest ("tag0") with a ProtocolError
                // naming it, rather than growing the map past the cap.
                let LineOutcome::Consumed { events, .. } = outcome else { panic!("expected Consumed") };
                assert_eq!(events.len(), 1);
                assert!(matches!(&events[0], McpEvent::ProtocolError(msg) if msg.contains("tag0")),
                    "expected a ProtocolError naming the evicted tag0, got {events:?}");
            }
        }
        assert_eq!(state.pending_count(), MAX_MCP_OPEN_MULTILINE, "must never hold more than the cap");

        // A later continuation/end line for the evicted tag falls into the ordinary
        // "unknown tag" path: consumed, hidden, no dispatch - not a second error.
        let closed = state.process_line(&format!("{MCP_PREFIX}: tag0"));
        assert_eq!(closed, LineOutcome::Consumed { replies: Vec::new(), events: Vec::new() });
    }

    // ==================================================================
    // Wrong/missing authentication key: THE most important test in the job.
    // ==================================================================

    #[test]
    fn wrong_or_missing_key_makes_a_message_inert_and_hidden() {
        // Case 1: no handshake has ever happened - there is no key to check against
        // at all, so any authenticated-package message is inert.
        let mut never_authed = McpState::default();
        let before_handshake = never_authed.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content anykey reference: r name: n type: string content: \"secret\""
        ));
        assert_hidden_no_events(&before_handshake);

        // Case 2: a real handshake happened, but the message carries the WRONG key.
        let (mut state, real_key) = authenticated_state();
        let wrong_key = format!("{real_key}x"); // definitely not equal
        let wrong = state.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {wrong_key} reference: r name: n type: string content: \"secret\""
        ));
        assert_hidden_no_events(&wrong);

        // Contrast: the SAME message with the CORRECT key does produce the event -
        // proves the rejection above was really about the key, not something else
        // wrong with the message.
        let right = state.process_line(&format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {real_key} reference: r name: n type: string content: \"secret\""
        ));
        let LineOutcome::Consumed { events, .. } = right else { panic!("expected Consumed") };
        assert_eq!(events.len(), 1, "the correctly-keyed message must be actionable");

        // Case 3: missing key entirely (no token before the keyvals at all).
        let mut state2 = McpState::default();
        handshake(&mut state2, "2.1", "2.1");
        let missing = state2.process_line(&format!(
            "{MCP_PREFIX}mcp-negotiate-can package: {SIMPLEEDIT_PACKAGE} min-version: 1.0 max-version: 1.0"
        ));
        assert_hidden_no_events(&missing);
        assert!(!state2.simpleedit_negotiated, "an unauthenticated negotiate-can must not take effect");
    }

    #[test]
    fn unknown_datatag_is_consumed_not_displayed() {
        let mut state = McpState::default();
        let line = format!("{MCP_PREFIX}* nosuchtag content: whatever");
        assert_hidden_no_events(&state.process_line(&line));
        let end = format!("{MCP_PREFIX}: nosuchtag");
        assert_hidden_no_events(&state.process_line(&end));
    }

    // ==================================================================
    // build_simpleedit_set
    // ==================================================================

    #[test]
    fn build_simpleedit_set_without_a_key_returns_none() {
        let mut state = McpState::default();
        assert!(state.build_simpleedit_set("r1", SimpleEditType::Str, "hi").is_none());
    }

    #[test]
    fn build_simpleedit_set_produces_well_formed_multiline_wire_lines() {
        let (mut state, key) = authenticated_state();
        let lines = state.build_simpleedit_set("#72.name", SimpleEditType::Str, "line one\nline two").unwrap();
        assert_eq!(lines.len(), 4, "start + 2 content lines + end: {lines:?}");
        assert!(lines[0].starts_with(&format!("{MCP_PREFIX}dns-org-mud-moo-simpleedit-set {key} reference: #72.name type: string content*: \"\" _data-tag: ")));
        let tag = lines[0].rsplit("_data-tag: ").next().unwrap();
        assert_eq!(lines[1], format!("{MCP_PREFIX}* {tag} content: line one"));
        assert_eq!(lines[2], format!("{MCP_PREFIX}* {tag} content: line two"));
        assert_eq!(lines[3], format!("{MCP_PREFIX}: {tag}"));

        // Feeding the SAME continuation/end lines this call produced, plus a
        // hand-built `-content`-shaped start line reusing the same tag/reference,
        // through a fresh receiver proves the continuation/end wire format this
        // module writes is exactly the one it reads (dispatch only differs on the
        // message name and needing a `name` field, which `-set` doesn't carry).
        let mut receiver = McpState { key: Some(key.clone()), ..McpState::default() };
        let start = format!(
            "{MCP_PREFIX}dns-org-mud-moo-simpleedit-content {key} reference: #72.name name: n type: string content*: \"\" _data-tag: {tag}"
        );
        let mut outcome = receiver.process_line(&start);
        assert_hidden_no_events(&outcome);
        for line in &lines[1..lines.len() - 1] {
            outcome = receiver.process_line(line);
            assert_hidden_no_events(&outcome);
        }
        outcome = receiver.process_line(&lines[lines.len() - 1]);
        let LineOutcome::Consumed { events, .. } = outcome else { panic!("expected Consumed") };
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], McpEvent::SimpleEditContent { content, .. } if content == "line one\nline two"));
    }

    #[test]
    fn quote_if_needed_round_trips_special_characters() {
        assert_eq!(quote_if_needed("plain"), "plain");
        assert_eq!(quote_if_needed(""), "\"\"");
        assert_eq!(quote_if_needed("has space"), "\"has space\"");
        assert_eq!(quote_if_needed("has\"quote"), "\"has\\\"quote\"");
        assert_eq!(quote_if_needed("has\\backslash"), "\"has\\\\backslash\"");
    }

    // ==================================================================
    // filter_lines: the blob-level entry point App::process_server_data actually
    // calls, exercised separately from the per-line primitive above.
    // ==================================================================

    #[test]
    fn filter_lines_hides_mcp_and_leaves_everything_else_untouched() {
        let mut state = McpState::default();
        let input = format!(
            "You see a sword here.\r\n{MCP_PREFIX}mcp version: 2.1 to: 2.1\r\nA rat scurries by.\r\n"
        );
        let (out, replies, events) = state.filter_lines(&input);
        // `str::lines()` treats \r\n as one terminator, and reassembly (matching how
        // the trigger-processing loop this feeds already rejoins lines with a bare
        // '\n' — see add_visible_run's lines_only.join("\n")) always emits a bare
        // '\n' regardless of the original line ending, for MCP-untouched lines too.
        // Not a new behavior this introduces: the existing pipeline already loses the
        // '\r' at this same point today.
        assert_eq!(out, "You see a sword here.\nA rat scurries by.\n");
        assert_eq!(replies.len(), 4, "the handshake reply batch: {replies:?}");
        assert!(events.is_empty());
    }

    #[test]
    fn filter_lines_leaves_a_trailing_partial_line_completely_untouched() {
        let mut state = McpState::default();
        // No trailing newline: the last line is a partial fragment, even though it
        // happens to start with the MCP prefix - must not be examined at all yet.
        let input = format!("first line\r\n{MCP_PREFIX}mcp version");
        let (out, replies, events) = state.filter_lines(&input);
        // The complete first line is rejoined with a bare '\n' (see the comment on
        // the test above); the trailing partial fragment is preserved byte-for-byte,
        // whitespace and all, since the caller's own partial-line carry-over depends
        // on getting exactly what it handed in back for anything not yet examined.
        assert_eq!(out, format!("first line\n{MCP_PREFIX}mcp version"));
        assert!(replies.is_empty());
        assert!(events.is_empty());
        assert!(state.key.is_none(), "an untouched partial must not have been acted on");
    }

    #[test]
    fn filter_lines_honours_the_escape_prefix_inline() {
        let mut state = McpState::default();
        let input = format!("{MCP_ESCAPE_PREFIX}{MCP_PREFIX}not really mcp\r\nnormal line\r\n");
        let (out, replies, events) = state.filter_lines(&input);
        assert_eq!(out, format!("{MCP_PREFIX}not really mcp\nnormal line\n"));
        assert!(replies.is_empty());
        assert!(events.is_empty());
    }

    /// A `#$#` message split across two `filter_lines` calls (mirroring two TCP
    /// reads) must behave identically to the same bytes arriving in one call - the
    /// caller (`App::process_server_data`) is responsible for the actual
    /// reassembly (`trigger_partial_line`), so this pins the contract `filter_lines`
    /// promises it: a still-incomplete final line is never touched.
    #[test]
    fn split_across_two_reads_is_invariant_once_reassembled() {
        let whole = format!("{MCP_PREFIX}mcp version: 2.1 to: 2.1\r\n");
        let mut one_shot = McpState::default();
        let (out_one_shot, replies_one_shot, _) = one_shot.filter_lines(&whole);

        // Split mid-line: first call sees an incomplete trailing fragment (no
        // trailing '\n'), gets nothing back for it, then the caller's own
        // partial-line carry-over (simulated here directly) hands the reassembled
        // whole line to a second call.
        let split_at = whole.find("version").unwrap();
        let (first, second) = whole.split_at(split_at);
        let mut split = McpState::default();
        let (out_first, replies_first, events_first) = split.filter_lines(first);
        // The incomplete fragment is preserved as-is (not acted on, not dropped) so
        // the caller's own partial-line carry-over can find it - see the
        // "leaves a trailing partial line completely untouched" test above.
        assert_eq!(out_first, first, "the incomplete fragment must be preserved untouched, not acted on");
        assert!(replies_first.is_empty());
        assert!(events_first.is_empty());
        let reassembled = format!("{first}{second}");
        let (out_second, replies_second, _) = split.filter_lines(&reassembled);

        assert_eq!(out_second, out_one_shot);
        // Reply *content* differs (each McpState generates its own random session
        // key), but the negotiation outcome - a real key established, the same
        // 4-reply handshake batch - must be identical either way.
        assert_eq!(replies_second.len(), replies_one_shot.len());
        assert_eq!(replies_second.len(), 4);
        assert!(split.key.is_some());
    }

    #[test]
    fn simple_edit_type_round_trips_and_falls_back() {
        assert_eq!(SimpleEditType::parse("string"), SimpleEditType::Str);
        assert_eq!(SimpleEditType::parse("MOO-CODE"), SimpleEditType::MooCode);
        assert_eq!(SimpleEditType::parse("string-list"), SimpleEditType::StringList);
        assert_eq!(SimpleEditType::parse("nonsense"), SimpleEditType::StringList);
    }
}
