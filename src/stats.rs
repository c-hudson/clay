//! Per-world status model fed by GMCP `Char.*` packages and MSDP variables — plan
//! Job 1 of mud-status-display.md.
//!
//! # Why this exists
//!
//! Clay already negotiates GMCP and MSDP and mirrors whatever arrives into
//! `World::gmcp_data`/`World::msdp_variables`, but nothing reads those maps back out
//! into anything a user could see. This module is the model half of that: it turns
//! the raw wire data into an ordered, paired view a status display can render
//! directly. It deliberately adds no rendering of its own (see Jobs 3/4 of the
//! plan) — `WorldStats` is inert until something calls [`WorldStats::entries`].
//!
//! # Normalizing by shape, not by a schema (plan D2)
//!
//! There is no universal field naming across MUD codebases (Achaea sends
//! `hp`/`maxhp`, MSDP conventionally uses `HEALTH`/`HEALTH_MAX`, others differ
//! again), so this does not attempt a fixed mapping table. Instead:
//!
//! 1. Both sources flatten into ordered `(key, value)` pairs, each tagged with its
//!    origin ([`update_from_gmcp`](WorldStats::update_from_gmcp) /
//!    [`update_from_msdp`](WorldStats::update_from_msdp)).
//! 2. [`WorldStats::entries`] pairs a key with its maximum by name shape,
//!    case-insensitively: `max<k>`, `<k>max`, `<k>_max`, `MAX_<k>`.
//! 3. A key that finds its maximum becomes a [`StatValue::Gauge`]; everything else
//!    stays a [`StatValue::Plain`] value.
//! 4. Nothing is ever dropped for being unrecognized — an unknown MUD degrades to a
//!    tidy key/value list rather than an empty model, mirroring the principle the
//!    MCP work (`src/mcp.rs`) already established: silently eating MUD data is
//!    worse than showing something plain.
//!
//! # Scope: only `Char.*` GMCP packages, MSDP variables minus protocol bookkeeping
//!
//! GMCP package names are case-insensitive per spec, which Clay's own
//! `package == "Client.Media.Default"` / `package.starts_with("Client.Media.")`
//! checks in `main.rs` do not honor (both compare case-sensitively — a pre-existing
//! gap this job does not touch, since Job 1's scope is only the `Char.` match).
//! Fixed here for `Char.*` via [`is_char_package`], which is intentionally the only
//! place this module compares a package name at all, so there is exactly one
//! case-insensitive comparison to keep correct rather than a second hand-rolled one
//! drifting out of sync with it. `Room.*`/`Comm.*`/`Client.*` (and everything else)
//! never reach this model. MSDP has no package concept — every variable the MUD
//! chooses to report feeds the model, *except* a variable that is itself MSDP
//! protocol bookkeeping rather than character state (`REPORTABLE_VARIABLES` and its
//! siblings — see [`is_msdp_meta_variable`], added for mud-status-display.md Job 6
//! alongside the auto-REPORT flow in `main.rs` that reads exactly these replies).
//!
//! # Malformed input
//!
//! A GMCP payload that fails to parse as JSON, or an MSDP value that fails to parse
//! as JSON (both are expected to always be valid JSON by the time they reach this
//! module — `telnet.rs` builds `msdp_variables`' JSON encoding itself — but a buggy
//! or hostile server is not trusted to hold up that end), contributes no entries
//! and leaves every existing entry untouched. Nothing here panics on attacker- or
//! bug-controlled input.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Which protocol reported a stat entry's current value. For a [`StatValue::Gauge`]
/// this describes the *current* value's source — the maximum may in principle have
/// arrived via the other protocol (pairing matches by name shape across the whole
/// flattened set, regardless of origin), which is rare enough in practice not to
/// need its own field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatOrigin {
    Gmcp,
    Msdp,
}

/// A stat's shape once paired (plan D2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StatValue {
    /// No maximum was found for this key — rendered as a plain labelled value.
    Plain(String),
    /// A maximum was found by name shape — rendered as a gauge (current/maximum/bar).
    Gauge { current: String, maximum: String },
}

/// One row of the derived, ordered view ([`WorldStats::entries`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatEntry {
    /// Display key, exactly as received from the MUD (whichever spelling arrived
    /// for the *current* value — a paired maximum's own spelling is not surfaced
    /// separately once consumed).
    pub key: String,
    pub value: StatValue,
    pub origin: StatOrigin,
}

/// One raw flattened `(key, value)` pair as stored before pairing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RawStat {
    value: String,
    origin: StatOrigin,
}

/// Recognized vital groups, most important first (plan D2 point 5). Membership is
/// an exact, case-insensitive match against the whole key — not a substring test,
/// so e.g. a MUD's unrelated `hp_regen` field does not get pulled to the front.
const VITAL_GROUPS: &[&[&str]] = &[
    &["health", "hp"],
    &["mana", "mp"],
    &["movement", "moves", "move", "mv"],
    &["experience", "exp", "xp"],
    &["level", "lvl", "lv"],
];

/// Per-world status model (plan D2/D5). Lives on `World`, populated from the
/// existing GMCP/MSDP handlers in `main.rs`, and cleared by
/// `World::clear_connection_state` alongside the other reactive protocol mirrors —
/// see that function's own comment for why this is not persisted to settings.dat or
/// the hot-reload state file.
///
/// Backed by a flat map of raw values, with [`entries`](Self::entries) lazily
/// recomputed and cached (T1.4 — see `entries_cache` below): a single incoming
/// GMCP message or MSDP variable only ever touches one or a few keys, but pairing
/// and ordering depend on the *whole* current set (a maximum arriving after its
/// current value, or vice versa, must still pair correctly), so there is no way to
/// maintain the derived view incrementally — it must be recomputed from this map
/// whenever the map has actually changed since the last computation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldStats {
    raw: HashMap<String, RawStat>,
    /// Cache of the last computed [`entries`](Self::entries) view (T1.4).
    /// `entries()` clones, lowercases and sorts the whole map on every call, and
    /// `rendering.rs` calls it up to three times per frame — this makes every call
    /// in between two real mutations free. Invalidated with `.take()` by every
    /// method that can change `raw` (`update_from_gmcp`, `update_from_msdp`,
    /// `clear`) so the next `entries()` call after a genuine change always
    /// recomputes rather than returning stale data.
    ///
    /// Skipped by serde (job 5, hot-reload persistence): a `OnceLock` has no
    /// `Serialize`/`Deserialize` impl, and there is no need for one — it is exactly
    /// as stale as `raw` was at save time, so restoring it verbatim would either
    /// require re-validating it against `raw` anyway or risk serving a cache that
    /// silently disagrees with the map it is supposed to be derived from. Left
    /// empty (`OnceLock`'s `Default`), it simply recomputes itself from `raw` on
    /// the first post-restore call, same as any other mutation.
    #[serde(skip)]
    entries_cache: std::sync::OnceLock<Vec<StatEntry>>,
}

impl WorldStats {
    /// True once any data has arrived — the signal the UI (Jobs 3/4) uses to decide
    /// whether to show anything at all (plan D1: visibility is derived from data).
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Drop every stat. Called by `World::clear_connection_state` on disconnect.
    pub fn clear(&mut self) {
        self.raw.clear();
        self.entries_cache.take();
    }

    /// Ingest one GMCP message. Only `Char.*` packages (matched case-insensitively)
    /// feed the model; anything else is a no-op. A payload that fails to parse as
    /// JSON is also a no-op — existing entries are left exactly as they were.
    pub fn update_from_gmcp(&mut self, package: &str, json_data: &str) {
        // T1.4: invalidate unconditionally, up front — simpler and safer than
        // tracking whether this particular call actually changed `raw`, and a
        // spurious recompute on a no-op call is cheap next to the cost of ever
        // serving a stale cache.
        self.entries_cache.take();
        if !is_char_package(package) {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json_data) else {
            return;
        };
        // A Char.* payload is always a JSON object in practice (its own fields are
        // the entries, unprefixed by the package name — Char.Vitals's "hp" becomes
        // the key "hp", not "Vitals.hp"). A non-object top level has no field name
        // to hang a value on and is skipped rather than inventing one.
        let serde_json::Value::Object(map) = &value else {
            return;
        };
        for (key, v) in map {
            flatten_into(key, v, StatOrigin::Gmcp, &mut self.raw);
        }
    }

    /// Ingest one MSDP variable. MSDP is flat by nature — every variable the MUD
    /// reports feeds the model, with no package-style filtering, *except* a variable
    /// that is itself MSDP protocol bookkeeping (plan Job 6) — see
    /// [`is_msdp_meta_variable`]. A value that fails to parse as JSON is a no-op,
    /// same as the GMCP path.
    pub fn update_from_msdp(&mut self, variable: &str, value_json: &str) {
        self.entries_cache.take(); // T1.4 — see update_from_gmcp's comment
        if is_msdp_meta_variable(variable) {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(value_json) else {
            return;
        };
        flatten_into(variable, &value, StatOrigin::Msdp, &mut self.raw);
    }

    /// The four case-insensitive name shapes (already-lowercased) that mark
    /// `key_lower` as *someone else's* maximum, in priority order (plan D2 point 2).
    fn max_shapes(key_lower: &str) -> [String; 4] {
        [
            format!("max{key_lower}"),
            format!("{key_lower}max"),
            format!("{key_lower}_max"),
            format!("max_{key_lower}"),
        ]
    }

    /// The ordered, paired view a status display renders (plan D2). Recomputed from
    /// the raw map lazily and cached (T1.4 — see `entries_cache`'s doc comment):
    /// the first call after a mutation recomputes and fills the cache, every call
    /// after that until the next mutation just returns the cached slice.
    ///
    /// Nothing is ever dropped for being unrecognized: every raw key ends up in the
    /// result as either the current side of a gauge or a plain entry, except a key
    /// that was consumed as *someone else's* maximum, which is folded into that
    /// gauge instead of also appearing on its own.
    pub fn entries(&self) -> &[StatEntry] {
        self.entries_cache.get_or_init(|| {
            // Deterministic processing order regardless of the HashMap's own
            // iteration order, so pairing and the final sort never depend on hash
            // iteration order varying between runs.
            let mut keys: Vec<String> = self.raw.keys().cloned().collect();
            keys.sort_by_key(|a| a.to_lowercase());

            // Lowercase -> original spelling, first occurrence wins. Two keys that
            // collide only by case is a pathological input this doesn't try to be
            // clever about beyond not panicking.
            let mut lower_to_orig: HashMap<String, String> = HashMap::new();
            for k in &keys {
                lower_to_orig.entry(k.to_lowercase()).or_insert_with(|| k.clone());
            }

            // Once a key is claimed as either side of a pairing it is fully spoken
            // for and cannot be reused - stops a maximum from also being treated as
            // a base looking for its own maximum, and stops the same key pairing
            // twice.
            let mut assigned: HashSet<String> = HashSet::new(); // lowercased
            let mut gauge_of: HashMap<String, String> = HashMap::new(); // base(orig) -> maximum(orig)
            let mut consumed_max: HashSet<String> = HashSet::new(); // maximum(orig), exact case

            for k in &keys {
                let k_lower = k.to_lowercase();
                if assigned.contains(&k_lower) {
                    continue;
                }
                for shape in Self::max_shapes(&k_lower) {
                    let Some(max_orig) = lower_to_orig.get(&shape) else {
                        continue;
                    };
                    if max_orig == k || assigned.contains(&shape) {
                        continue;
                    }
                    assigned.insert(k_lower.clone());
                    assigned.insert(shape.clone());
                    gauge_of.insert(k.clone(), max_orig.clone());
                    consumed_max.insert(max_orig.clone());
                    break;
                }
            }

            let mut result: Vec<StatEntry> = Vec::with_capacity(keys.len());
            for k in &keys {
                if consumed_max.contains(k) {
                    continue; // folded into another entry's gauge - never shown twice
                }
                let raw = &self.raw[k];
                let value = match gauge_of.get(k) {
                    Some(max_key) => StatValue::Gauge {
                        current: raw.value.clone(),
                        maximum: self.raw[max_key].value.clone(),
                    },
                    None => StatValue::Plain(raw.value.clone()),
                };
                result.push(StatEntry { key: k.clone(), value, origin: raw.origin });
            }

            result.sort_by(|a, b| {
                vital_priority(&a.key)
                    .cmp(&vital_priority(&b.key))
                    .then_with(|| a.key.to_lowercase().cmp(&b.key.to_lowercase()))
            });
            result
        })
    }

    /// Test-only: number of distinct raw keys currently stored (T1.4) — lets a test
    /// observe `MAX_STAT_KEYS` capping without reaching into the private `raw` map.
    #[cfg(test)]
    pub(crate) fn key_count(&self) -> usize {
        self.raw.len()
    }

    /// Test-only: whether `entries()` has been called since the last mutation
    /// (T1.4) — observes the cache from outside without exposing its contents.
    #[cfg(test)]
    pub(crate) fn entries_cache_is_populated(&self) -> bool {
        self.entries_cache.get().is_some()
    }
}

/// Is `package` in the `Char.*` GMCP family? Case-insensitive per the GMCP spec
/// (package names are case-insensitive) — see the module doc comment for why this
/// is the one place a package prefix is compared, rather than adding a second
/// case-sensitive check alongside the existing `Client.Media.*` ones in `main.rs`.
/// Case-insensitive GMCP package-name prefix test. The GMCP spec says package
/// names are case-insensitive; this is the single place that rule lives, so a
/// later comparison cannot quietly drift back to a case-sensitive `starts_with`.
pub fn package_has_prefix(package: &str, prefix: &str) -> bool {
    package.len() >= prefix.len()
        && package.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn is_char_package(package: &str) -> bool {
    package_has_prefix(package, "char.")
}

/// MSDP's "special variables" (mud-status-display.md Job 6): the valid targets of a
/// `LIST` request per the MSDP spec, always answered with an array rather than
/// character state. `REPORTABLE_VARIABLES` is the one Clay itself requests (see
/// `App::handle_msdp_negotiated`/`App::request_msdp_reports` in `main.rs`), but a
/// server is free to volunteer any of these unprompted, and none of them belong in
/// the status model — a row titled `REPORTABLE_VARIABLES` listing every stat name
/// the MUD supports is protocol noise, not a stat. Matched case-insensitively, same
/// convention as [`is_char_package`]/[`package_has_prefix`]: MSDP variable names are
/// conventionally all-uppercase but nothing enforces that on the wire.
const MSDP_META_VARIABLES: &[&str] = &[
    "COMMANDS",
    "LISTS",
    "CONFIGURABLE_VARIABLES",
    "REPORTABLE_VARIABLES",
    "REPORTED_VARIABLES",
    "SENDABLE_VARIABLES",
];

/// Is `variable` one of MSDP's own protocol-bookkeeping names ([`MSDP_META_VARIABLES`])
/// rather than character state? See that constant's doc comment.
pub fn is_msdp_meta_variable(variable: &str) -> bool {
    MSDP_META_VARIABLES.iter().any(|m| m.eq_ignore_ascii_case(variable))
}

/// Recognized-vital sort priority for `key` (plan D2 point 5): index into
/// [`VITAL_GROUPS`], or one past the end for everything else (sorted alphabetically
/// among themselves afterwards).
fn vital_priority(key: &str) -> usize {
    let lower = key.to_lowercase();
    VITAL_GROUPS
        .iter()
        .position(|group| group.contains(&lower.as_str()))
        .unwrap_or(VITAL_GROUPS.len())
}

/// Cap on the number of distinct keys [`WorldStats::raw`](WorldStats) may hold
/// (T1.4). Nothing previously bounded this: a server that varies its GMCP/MSDP key
/// names across messages (deliberately, or via a buggy per-message-unique field
/// name) grows the map without bound, and [`WorldStats::entries`] clones,
/// lowercases and sorts the *whole* map on every call — a redraw-path cost that
/// scales with attacker-controlled input. Real MUDs report tens of distinct
/// fields; 256 is generous headroom above that while still being a hard limit.
pub const MAX_STAT_KEYS: usize = 256;

/// Insert `key -> value` into `out`, refusing a **new** key once `out` already
/// holds [`MAX_STAT_KEYS`] distinct keys (T1.4). An update to a key that is
/// already present always succeeds, capped or not — it never grows the map, and a
/// server that is actively reporting real state (as opposed to spamming new key
/// names) must keep working normally even once the cap is reached.
fn insert_capped(out: &mut HashMap<String, RawStat>, key: String, value: RawStat) {
    if out.len() >= MAX_STAT_KEYS && !out.contains_key(&key) {
        return;
    }
    out.insert(key, value);
}

/// Flatten one JSON value under `key` into `out`, applying the plan's normalization
/// rule: a scalar (string/number/bool) becomes `key -> value` directly; a nested
/// object is unwrapped exactly one level, each of *its* scalar fields becoming
/// `key.subkey -> value` (an array or a further-nested object inside that is more
/// than one level deep and is skipped); an array or null at the top level is
/// skipped entirely. Shared by both the GMCP (per top-level field) and MSDP (per
/// variable) ingestion paths — see their doc comments for how `key` differs between
/// the two callers. Every insertion goes through [`insert_capped`] so this can
/// never grow [`WorldStats::raw`] past [`MAX_STAT_KEYS`] distinct keys.
fn flatten_into(key: &str, v: &serde_json::Value, origin: StatOrigin, out: &mut HashMap<String, RawStat>) {
    match v {
        serde_json::Value::String(s) => {
            insert_capped(out, key.to_string(), RawStat { value: s.clone(), origin });
        }
        serde_json::Value::Number(n) => {
            insert_capped(out, key.to_string(), RawStat { value: n.to_string(), origin });
        }
        serde_json::Value::Bool(b) => {
            insert_capped(out, key.to_string(), RawStat { value: b.to_string(), origin });
        }
        serde_json::Value::Object(nested) => {
            for (nk, nv) in nested {
                let dotted = format!("{key}.{nk}");
                match nv {
                    serde_json::Value::String(s) => {
                        insert_capped(out, dotted, RawStat { value: s.clone(), origin });
                    }
                    serde_json::Value::Number(n) => {
                        insert_capped(out, dotted, RawStat { value: n.to_string(), origin });
                    }
                    serde_json::Value::Bool(b) => {
                        insert_capped(out, dotted, RawStat { value: b.to_string(), origin });
                    }
                    // Array or a further-nested object: more than one level deep,
                    // skip per the flatten-one-level rule.
                    _ => {}
                }
            }
        }
        // Array or null at the top level: skip.
        _ => {}
    }
}

/// Maximum rendered width, in terminal columns (measured via
/// [`crate::display_width`], never `.len()`/`.chars().count()` — see
/// [`sanitize_and_cap`]), allowed for a single stat key or value once sanitized (plan
/// Job 4 security requirement). Every key and value here is MUD-supplied text (a
/// `Char.*` GMCP field or an MSDP variable), so an unbounded one must not be able to
/// crowd out every other entry. Applies to both the full `/stats` listing
/// (`commands::execute_stats_command`) and the console status line
/// (`rendering::render_stats_line`, which additionally applies its own much tighter
/// per-entry budget on top of this since it must fit several entries on one row).
pub const MAX_STAT_FIELD_WIDTH: usize = 200;

/// Strip every control character from MUD-supplied stat text before it ever reaches a
/// terminal — a `Char.*` GMCP field or an MSDP variable is attacker-controlled the
/// same as any other server text, and unlike ordinary output lines this one is never
/// run through the normal `strip_ansi_codes`/telnet decode pipeline first.
///
/// Two passes, deliberately kept as two calls rather than one hand-rolled filter:
/// - [`crate::encoding::strip_c1_controls`] removes C1 controls (U+0080-U+009F) — the
///   hazard CLAUDE.md documents in detail: a terminal in a UTF-8 locale decodes an
///   unstripped one as APC, which swallows every byte that follows (including the
///   cursor positioning and separator/input redraw queued right after this line)
///   until a String Terminator arrives, corrupting the display on every repaint from
///   then on. Reused rather than reimplemented, per that module's own doc comment.
/// - `char::is_ascii_control` removes C0 controls (0x00-0x1F, which includes ESC —
///   the start of an arbitrary escape sequence — and CR/LF/TAB) plus DEL (0x7F).
///   `strip_c1_controls` doesn't touch this range at all, and a raw newline or ESC
///   here is exactly as unsafe to this line as a C1 byte: either desyncs the fixed
///   single row this text is rendered into, or hands the MUD an open escape sequence
///   straight to the terminal.
pub fn sanitize_stat_text(s: &str) -> String {
    let c1_stripped = crate::encoding::strip_c1_controls(s.to_string());
    c1_stripped.chars().filter(|c| !c.is_ascii_control()).collect()
}

/// [`sanitize_stat_text`], then truncate to at most `max_width` display columns
/// (measured with [`crate::display_width`]/[`crate::chars_for_display_width`] — never
/// `.len()` or `.chars().count()`, both of which a MUD-supplied string full of wide or
/// zero-width characters would silently disagree with), appending a single `…` marker
/// when truncation happened so a reader can tell the value was cut rather than
/// assuming it was simply short.
pub fn sanitize_and_cap(s: &str, max_width: usize) -> String {
    let clean = sanitize_stat_text(s);
    if crate::display_width(&clean) <= max_width {
        return clean;
    }
    if max_width == 0 {
        return String::new();
    }
    let chars: Vec<char> = clean.chars().collect();
    // Reserve one column for the trailing ellipsis marker.
    let (cut, _) = crate::chars_for_display_width(&chars, max_width - 1);
    let mut truncated: String = chars[..cut].iter().collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry<'a>(entries: &'a [StatEntry], key: &str) -> Option<&'a StatEntry> {
        entries.iter().find(|e| e.key == key)
    }

    // ------------------------------------------------------------------
    // The four maximum-naming shapes (plan D2 point 2), each case-insensitive.
    // ------------------------------------------------------------------

    #[test]
    fn pairs_max_prefix_shape() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"hp":"80","maxhp":"100"}"#);
        let entries = s.entries();
        assert_eq!(
            entry(entries, "hp").unwrap().value,
            StatValue::Gauge { current: "80".to_string(), maximum: "100".to_string() }
        );
        assert!(entry(entries, "maxhp").is_none(), "maxhp must not also appear as its own entry");
    }

    #[test]
    fn pairs_suffix_max_shape() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"mp":"30","mpmax":"50"}"#);
        let entries = s.entries();
        assert_eq!(
            entry(entries, "mp").unwrap().value,
            StatValue::Gauge { current: "30".to_string(), maximum: "50".to_string() }
        );
        assert!(entry(entries, "mpmax").is_none());
    }

    #[test]
    fn pairs_underscore_suffix_shape() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"mv":"12","mv_max":"20"}"#);
        let entries = s.entries();
        assert_eq!(
            entry(entries, "mv").unwrap().value,
            StatValue::Gauge { current: "12".to_string(), maximum: "20".to_string() }
        );
        assert!(entry(entries, "mv_max").is_none());
    }

    #[test]
    fn pairs_max_underscore_prefix_shape() {
        let mut s = WorldStats::default();
        s.update_from_msdp("HEALTH", "\"100\"");
        s.update_from_msdp("HEALTH_MAX", "\"150\"");
        let entries = s.entries();
        assert_eq!(
            entry(entries, "HEALTH").unwrap().value,
            StatValue::Gauge { current: "100".to_string(), maximum: "150".to_string() }
        );
        assert!(entry(entries, "HEALTH_MAX").is_none());
    }

    #[test]
    fn pairing_is_case_insensitive_across_mixed_casing() {
        let mut s = WorldStats::default();
        // Base and maximum arrive in unrelated casing - real MUDs are internally
        // consistent, but the pairing rule itself must not care.
        s.update_from_gmcp("Char.Vitals", r#"{"Hp":"80"}"#);
        s.update_from_gmcp("Char.MaxStats", r#"{"MAXHP":"100"}"#);
        let entries = s.entries();
        assert_eq!(entries.len(), 1, "Hp/MAXHP must pair despite mismatched case");
        assert_eq!(
            entry(entries, "Hp").unwrap().value,
            StatValue::Gauge { current: "80".to_string(), maximum: "100".to_string() }
        );
    }

    // ------------------------------------------------------------------
    // Plain values, survival, and hiding a consumed maximum.
    // ------------------------------------------------------------------

    #[test]
    fn value_with_no_maximum_stays_plain() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Status", r#"{"name":"Bob"}"#);
        let entries = s.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, StatValue::Plain("Bob".to_string()));
    }

    #[test]
    fn consumed_maximum_does_not_also_appear_as_its_own_entry() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"hp":"80","maxhp":"100"}"#);
        let entries = s.entries();
        assert_eq!(entries.len(), 1, "maxhp must be folded into hp's gauge, not counted separately");
    }

    #[test]
    fn unknown_keys_survive_rather_than_being_dropped() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Status", r#"{"favorite_toad_count":"3"}"#);
        let entries = s.entries();
        assert_eq!(
            entry(entries, "favorite_toad_count").unwrap().value,
            StatValue::Plain("3".to_string()),
            "an unrecognized field must still show up plainly, never be dropped"
        );
    }

    // ------------------------------------------------------------------
    // Ordering: recognized vitals first, then the rest alphabetically.
    // ------------------------------------------------------------------

    #[test]
    fn orders_recognized_vitals_first_then_alphabetical() {
        let mut s = WorldStats::default();
        s.update_from_gmcp(
            "Char.Vitals",
            r#"{"zebra":"1","level":"10","xp":"500","mana":"40","health":"90","apple":"1"}"#,
        );
        let entries = s.entries();
        let order: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(order, vec!["health", "mana", "xp", "level", "apple", "zebra"]);
    }

    // ------------------------------------------------------------------
    // Malformed / nested payloads.
    // ------------------------------------------------------------------

    #[test]
    fn nested_gmcp_object_flattens_one_level_with_dotted_key() {
        let mut s = WorldStats::default();
        s.update_from_gmcp(
            "Char.Status",
            r#"{"name":"Bob","race":{"name":"Human","id":3,"tags":["a","b"]}}"#,
        );
        let entries = s.entries();
        assert_eq!(entry(entries, "name").unwrap().value, StatValue::Plain("Bob".to_string()));
        assert_eq!(entry(entries, "race.name").unwrap().value, StatValue::Plain("Human".to_string()));
        assert_eq!(entry(entries, "race.id").unwrap().value, StatValue::Plain("3".to_string()));
        assert!(entry(entries, "race.tags").is_none(), "an array nested inside the flattened object must be skipped");
        assert!(entry(entries, "race").is_none(), "the intermediate object key itself is not an entry");
    }

    #[test]
    fn top_level_array_field_is_skipped() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Status", r#"{"name":"Bob","tags":["a","b"]}"#);
        let entries = s.entries();
        assert_eq!(entries.len(), 1);
        assert!(entry(entries, "tags").is_none());
    }

    #[test]
    fn malformed_gmcp_payload_does_not_panic_and_does_not_corrupt_existing_entries() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Status", r#"{"name":"Bob"}"#);
        // Not valid JSON at all.
        s.update_from_gmcp("Char.Vitals", "{not json");
        // Valid JSON but not an object - also must not disturb anything.
        s.update_from_gmcp("Char.Vitals", "[1,2,3]");
        s.update_from_gmcp("Char.Vitals", "42");
        let entries = s.entries();
        assert_eq!(entries.len(), 1, "malformed/non-object payloads must add nothing");
        assert_eq!(entry(entries, "name").unwrap().value, StatValue::Plain("Bob".to_string()));
    }

    #[test]
    fn malformed_msdp_value_does_not_panic_and_does_not_corrupt_existing_entries() {
        let mut s = WorldStats::default();
        s.update_from_msdp("HEALTH", "\"100\"");
        s.update_from_msdp("MANA", "not valid json at all {{{");
        let entries = s.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entry(entries, "HEALTH").unwrap().value, StatValue::Plain("100".to_string()));
    }

    // ------------------------------------------------------------------
    // Char.* package gating and case-insensitivity.
    // ------------------------------------------------------------------

    #[test]
    fn char_package_matching_is_case_insensitive() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("cHAR.vitals", r#"{"hp":"80"}"#);
        assert!(!s.is_empty(), "Char.* must match regardless of case");
    }

    #[test]
    fn non_char_packages_are_excluded() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Room.Info", r#"{"name":"The Temple"}"#);
        s.update_from_gmcp("Comm.Channel", r#"{"name":"gossip"}"#);
        s.update_from_gmcp("Client.Media.Default", r#"{"url":"http://example.com"}"#);
        assert!(s.is_empty(), "Room./Comm./Client. packages must never feed the model");
    }

    #[test]
    fn char_prefixed_but_unrelated_package_name_is_excluded() {
        let mut s = WorldStats::default();
        // "Chart.Foo" starts with "Char" but is not in the Char.* family.
        s.update_from_gmcp("Chart.Foo", r#"{"x":"1"}"#);
        assert!(s.is_empty());
    }

    // ------------------------------------------------------------------
    // MSDP.
    // ------------------------------------------------------------------

    #[test]
    fn msdp_variables_feed_the_model() {
        let mut s = WorldStats::default();
        s.update_from_msdp("HEALTH", "\"100\"");
        let entries = s.entries();
        assert_eq!(entries.len(), 1);
        let e = entry(entries, "HEALTH").unwrap();
        assert_eq!(e.value, StatValue::Plain("100".to_string()));
        assert_eq!(e.origin, StatOrigin::Msdp);
    }

    #[test]
    fn msdp_table_value_flattens_one_level_under_the_variable_name() {
        let mut s = WorldStats::default();
        s.update_from_msdp("ROOM", r#"{"VNUM":"1001","NAME":"The Temple"}"#);
        let entries = s.entries();
        assert_eq!(
            entry(entries, "ROOM.VNUM").unwrap().value,
            StatValue::Plain("1001".to_string())
        );
        assert_eq!(
            entry(entries, "ROOM.NAME").unwrap().value,
            StatValue::Plain("The Temple".to_string())
        );
    }

    #[test]
    fn msdp_array_value_is_skipped() {
        let mut s = WorldStats::default();
        s.update_from_msdp("EXITS", r#"["north","south"]"#);
        assert!(s.is_empty());
    }

    // ------------------------------------------------------------------
    // MSDP meta variables (plan Job 6) — protocol bookkeeping, never a stat.
    // ------------------------------------------------------------------

    #[test]
    fn msdp_reportable_variables_reply_never_becomes_a_stat() {
        let mut s = WorldStats::default();
        // The real shape a server sends back: an array of variable names. Already
        // excluded incidentally by the array-skip rule in flatten_into, but this
        // pins the actually-required behavior down explicitly.
        s.update_from_msdp("REPORTABLE_VARIABLES", r#"["HEALTH","HEALTH_MAX","MANA"]"#);
        assert!(s.is_empty(), "REPORTABLE_VARIABLES must never appear as a stat");
    }

    #[test]
    fn msdp_meta_variable_sent_as_a_scalar_is_still_excluded() {
        let mut s = WorldStats::default();
        // The case the array-skip rule above does NOT catch on its own: a
        // non-conforming server sending a meta variable's value as a plain string
        // rather than a proper MSDP array. Without an explicit name-based exclusion
        // this would otherwise flatten straight into a Plain stat.
        s.update_from_msdp("REPORTABLE_VARIABLES", "\"HEALTH, HEALTH_MAX, MANA\"");
        assert!(s.is_empty(), "a meta variable must be excluded by name, not just by shape");
    }

    #[test]
    fn msdp_meta_variable_matching_is_case_insensitive() {
        let mut s = WorldStats::default();
        s.update_from_msdp("reportable_variables", "\"x\"");
        assert!(s.is_empty());
    }

    #[test]
    fn all_msdp_meta_variables_are_excluded() {
        for name in ["COMMANDS", "LISTS", "CONFIGURABLE_VARIABLES",
                      "REPORTABLE_VARIABLES", "REPORTED_VARIABLES", "SENDABLE_VARIABLES"] {
            let mut s = WorldStats::default();
            s.update_from_msdp(name, "\"whatever\"");
            assert!(s.is_empty(), "{name} must be excluded from stats");
        }
    }

    #[test]
    fn msdp_meta_variable_does_not_shadow_a_real_variable_of_a_similar_name() {
        let mut s = WorldStats::default();
        // Sanity: only an exact (case-insensitive) match against the reserved names
        // is excluded - an ordinary variable that merely contains one as a substring
        // still feeds the model normally.
        s.update_from_msdp("MY_COMMANDS", "\"5\"");
        assert!(!s.is_empty(), "a variable that isn't itself a reserved name must still feed the model");
    }

    // ------------------------------------------------------------------
    // Lifecycle and serialization.
    // ------------------------------------------------------------------

    #[test]
    fn clear_empties_the_model() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"hp":"80"}"#);
        assert!(!s.is_empty());
        s.clear();
        assert!(s.is_empty());
        assert!(s.entries().is_empty());
    }

    #[test]
    fn stat_entries_round_trip_through_serde_json() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"hp":"80","maxhp":"100"}"#);
        s.update_from_gmcp("Char.Status", r#"{"name":"Bob"}"#);
        let entries = s.entries().to_vec();
        let json = serde_json::to_string(&entries).expect("StatEntry must serialize");
        let round_tripped: Vec<StatEntry> = serde_json::from_str(&json).expect("StatEntry must deserialize");
        assert_eq!(entries, round_tripped);
    }

    // ------------------------------------------------------------------
    // T1.4: bounding the number of distinct keys, and the entries() cache.
    // ------------------------------------------------------------------

    #[test]
    fn distinct_key_count_is_capped_but_existing_keys_still_update() {
        let mut s = WorldStats::default();
        // 300 distinct MSDP variables, each its own key - well past MAX_STAT_KEYS.
        for i in 0..300 {
            s.update_from_msdp(&format!("VAR{i}"), &format!("\"{i}\""));
        }
        assert_eq!(s.key_count(), MAX_STAT_KEYS, "must never hold more than the cap");

        // A key that was already stored before the cap was reached must keep
        // updating normally - the cap only refuses brand-new keys.
        s.update_from_msdp("VAR0", "\"updated\"");
        assert_eq!(s.key_count(), MAX_STAT_KEYS, "updating an existing key must not change the count");
        let entries = s.entries();
        let var0 = entries.iter().find(|e| e.key == "VAR0").expect("VAR0 must still be present");
        assert_eq!(var0.value, StatValue::Plain("updated".to_string()));
    }

    #[test]
    fn entries_cache_is_populated_by_a_call_and_cleared_by_an_update() {
        let mut s = WorldStats::default();
        s.update_from_gmcp("Char.Vitals", r#"{"hp":"80"}"#);
        assert!(!s.entries_cache_is_populated(), "must start uncached after a mutation");
        let _ = s.entries();
        assert!(s.entries_cache_is_populated(), "a call to entries() must populate the cache");
        s.update_from_gmcp("Char.Vitals", r#"{"hp":"90"}"#);
        assert!(!s.entries_cache_is_populated(), "a later update must invalidate the cache");
        // And the recomputed value reflects the update, not stale cached data.
        let entries = s.entries();
        assert_eq!(entries.iter().find(|e| e.key == "hp").unwrap().value, StatValue::Plain("90".to_string()));
    }

    // ------------------------------------------------------------------
    // Sanitization (plan Job 4 security requirement): MUD-supplied stat text must
    // never reach a terminal with a control character still in it.
    // ------------------------------------------------------------------

    #[test]
    fn sanitize_strips_c1_control() {
        // U+009F (APC) - the exact hazard CLAUDE.md documents: read by a terminal in a
        // UTF-8 locale as an Application Program Command that swallows everything
        // until a String Terminator, which would eat this line's own redraw.
        let hostile = "HP:80\u{9f}more";
        let clean = sanitize_stat_text(hostile);
        assert!(!clean.contains('\u{9f}'), "C1 control must be stripped");
        assert_eq!(clean, "HP:80more");
    }

    #[test]
    fn sanitize_strips_esc_and_newline_and_carriage_return() {
        // A real ESC sequence (color change) plus an embedded newline and CR - none of
        // these are C1, so strip_c1_controls alone would leave them all in place.
        let hostile = "\x1b[31mRED\x1b[0m\nsecond line\rthird";
        let clean = sanitize_stat_text(hostile);
        assert!(!clean.contains('\x1b'), "ESC must be stripped");
        assert!(!clean.contains('\n'), "newline must be stripped");
        assert!(!clean.contains('\r'), "carriage return must be stripped");
        // The bracket/digit/letter payload of the escape sequence is plain printable
        // text once ESC itself is gone - harmless, and not this function's job to
        // detect as "was part of an escape sequence".
        assert_eq!(clean, "[31mRED[0msecond linethird");
    }

    #[test]
    fn sanitize_combined_hostile_value_is_fully_safe() {
        // A single value carrying all three hazards named in the plan: a C1 byte, a
        // real ESC sequence, and an embedded newline.
        let hostile = "80\u{9f}\x1b[2J\nmore text\rand more";
        let clean = sanitize_stat_text(hostile);
        for c in clean.chars() {
            assert!(!c.is_ascii_control(), "no ASCII control character may survive: {c:?}");
            assert!(!crate::encoding::is_c1_control(c), "no C1 control character may survive: {c:?}");
        }
    }

    #[test]
    fn sanitize_leaves_ordinary_text_untouched() {
        assert_eq!(sanitize_stat_text("100"), "100");
        assert_eq!(sanitize_stat_text("Bob the Brave"), "Bob the Brave");
    }

    #[test]
    fn cap_truncates_long_value_with_ellipsis_and_respects_display_width() {
        let long = "x".repeat(50);
        let capped = sanitize_and_cap(&long, 10);
        assert_eq!(crate::display_width(&capped), 10, "capped text must measure exactly the budget");
        assert!(capped.ends_with('…'), "truncated text must be marked with an ellipsis");
    }

    #[test]
    fn cap_leaves_short_value_unchanged() {
        assert_eq!(sanitize_and_cap("80", 10), "80");
    }

    #[test]
    fn cap_sanitizes_before_measuring_width() {
        // A hostile value that is short once sanitized must not be truncated just
        // because the raw (unsanitized) string was long.
        let hostile = format!("80{}", "\u{9f}".repeat(100));
        let capped = sanitize_and_cap(&hostile, 10);
        assert_eq!(capped, "80");
    }

    #[test]
    fn cap_at_zero_width_returns_empty() {
        assert_eq!(sanitize_and_cap("anything", 0), "");
    }
}
