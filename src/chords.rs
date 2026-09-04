//! Multi-keystroke chord input (`^X^R`, `Esc-^N`, `Esc-Left`, ...), shared by
//! the console (`input_handler::handle_key_event`) and the SSH remote-console
//! client (`remote_client::handle_remote_client_key`) so the two lookup paths
//! Job 17 found duplicated can no longer drift (TinyFugue-parity plan,
//! finding A / Phase 2 step P2.2).
//!
//! `App.last_escape` (a bare `Option<Instant>` that only ever recognised one
//! specific two-keystroke shape - Escape then one more key) is replaced by
//! [`ChordState`], which generalises the same idea to TF's real chord model:
//! *any* key can be the first half of a multi-keystroke binding as long as
//! some longer binding starts with it (`is_prefix`), not just Escape.
//!
//! Escape itself is not special-cased by [`ChordState`] at all - it is
//! simply the token `Named(Escape)`, and [`ChordState::push`] folds it into
//! an `Esc(...)` *compound* `KeyToken` (see `keynames.rs`'s module doc: an
//! `Esc-b` binding is ONE grammar token, not two chord elements) the moment
//! the next keystroke arrives, exactly the way `^X` combines with `^R` as
//! two separate elements of the same [`KeySeq`]. Both shapes are just
//! "extend the buffered prefix with one more token, then ask whether that's
//! a real binding, a real (still-ambiguous) prefix, or a dead end" -
//! [`ChordState::push`] doesn't need to know which case it's in.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::keybindings::{escape_key_to_token, key_event_to_token};
use crate::keynames::{self, KeySeq, KeyToken, NamedKey};
use crate::App;

/// TF's own chord/kbnum cancel key: `^G` always aborts a buffered chord
/// without dispatching anything (plan finding A).
const CANCEL_TOKEN: KeyToken = KeyToken::Ctrl('G');

/// Default chord timeout - matches the old hardcoded bare-Escape window
/// exactly (`Duration::from_millis(500)` in the pre-Job-18
/// `input_handler.rs`/`remote_client.rs`). `App.chord_window` overrides this
/// so tests don't need to sleep for real.
pub const DEFAULT_CHORD_WINDOW: Duration = Duration::from_millis(500);

/// Outcome of [`ChordState::push`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordResult {
    /// `token` extended a real, still-ambiguous prefix - buffered, dispatch
    /// nothing yet.
    Pending,
    /// The sequence built so far (including `token`) is ready to dispatch:
    /// either it matched a binding outright, or it's an ordinary first
    /// keystroke that isn't a chord at all (the common case - the caller's
    /// own binding lookup finds nothing and falls through exactly as if
    /// chords didn't exist).
    Complete(KeySeq),
    /// The prefix buffered *before* `replay` arrived turned out to match no
    /// binding once `replay` broke it out of every candidate chord.
    /// Dispatch `prefix`'s own binding if it has one, then process `replay`
    /// exactly as if no chord had been in progress.
    Abandon { prefix: KeySeq, replay: KeyToken },
}

/// Buffered, not-yet-resolved chord prefix, plus when it was last extended.
#[derive(Debug, Clone, Default)]
pub struct ChordState {
    pending: Vec<KeyToken>,
    armed_at: Option<Instant>,
}

impl ChordState {
    pub fn new() -> Self {
        Self { pending: Vec::new(), armed_at: None }
    }

    /// Whether a chord is currently buffered, waiting for its next
    /// keystroke.
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Discard any buffered prefix without dispatching anything - TF's `^G`
    /// chord/kbnum cancel key.
    pub fn cancel(&mut self) {
        self.pending.clear();
        self.armed_at = None;
    }

    /// Push one more keystroke's token onto the buffered prefix (or start a
    /// fresh one). `is_prefix`/`has_binding` are asked about the *combined*
    /// candidate sequence built so far; callers pass closures over whatever
    /// binding tables they have (TF `/bind`, action `KeyBindings`, ...) -
    /// this module knows nothing about either.
    pub fn push(
        &mut self,
        token: KeyToken,
        now: Instant,
        is_prefix: impl Fn(&KeySeq) -> bool,
        has_binding: impl Fn(&KeySeq) -> bool,
    ) -> ChordResult {
        let had_prefix = !self.pending.is_empty();

        // A bare Escape waiting for its target folds the next token into ONE
        // compound `Esc(...)` KeyToken rather than appending a sibling
        // element - see keynames.rs's module doc: `Esc-b` is a single
        // grammar token, not two chord elements.
        if matches!(self.pending.last(), Some(KeyToken::Named(NamedKey::Escape))) {
            self.pending.pop();
            self.pending.push(KeyToken::Esc(Box::new(token.clone())));
        } else {
            self.pending.push(token.clone());
        }

        let candidate = KeySeq(self.pending.clone());

        // A still-bare, not-yet-wrapped Escape is ALWAYS open-ended: it has
        // no canonical text of its own to look up yet (its meaning depends
        // entirely on whatever key comes next), so it's never resolved via
        // `is_prefix`/`has_binding` at all - it just always buffers and
        // waits, exactly like the pre-Job-18 `last_escape` mechanism did
        // regardless of whether any Esc-* binding was even registered.
        // Comparing `candidate`'s *token vector* against a bound key's own
        // parsed tokens (below) can't see this relationship on its own:
        // `Esc-b` parses to ONE compound `Esc(Char('b'))` token, the same
        // length as the bare `Named(Escape)` token alone - not a longer
        // sequence starting with it - so a plain vector-prefix check would
        // never consider bare Escape a prefix of anything.
        if matches!(candidate.0.last(), Some(KeyToken::Named(NamedKey::Escape))) {
            self.armed_at = Some(now);
            return ChordResult::Pending;
        }

        if is_prefix(&candidate) {
            // Still ambiguous - some longer binding could still be
            // completed. Wait, even if `candidate` itself already has a
            // direct binding too (that ambiguity is resolved on `expired`,
            // by firing the shorter binding, or by a later `push` that
            // breaks the chord, via `Abandon` below).
            self.armed_at = Some(now);
            return ChordResult::Pending;
        }

        self.pending.clear();
        self.armed_at = None;

        if has_binding(&candidate) {
            return ChordResult::Complete(candidate);
        }

        if had_prefix {
            // `candidate` is everything buffered before `token` plus `token`
            // itself; `prefix` is just the part that was already buffered.
            let prefix = KeySeq(candidate.0[..candidate.0.len() - 1].to_vec());
            ChordResult::Abandon { prefix, replay: token }
        } else {
            // No prefix was buffered - this is just an ordinary, non-chord
            // key (bound or not; the caller's own lookup handles "not").
            ChordResult::Complete(candidate)
        }
    }

    /// If the buffered prefix has gone stale (`window` elapsed since it was
    /// last extended), drop it and return its own binding's name if it has
    /// one worth firing on timeout. Returns `None` either while still
    /// within the window (nothing changes - still pending) or once dropped
    /// with nothing to fire.
    ///
    /// A bare, still-unfollowed `Escape` is a hardcoded exception: it has
    /// always been a silent no-op on timeout (the pre-Job-18 code never
    /// dispatched anything for a lone Escape press, regardless of whether a
    /// binding for plain "Escape" existed) - so it is never fired here even
    /// if `has_binding` would say yes, preserving that behaviour exactly
    /// rather than teaching a previously-inert binding to suddenly fire.
    pub fn expired(
        &mut self,
        now: Instant,
        window: Duration,
        has_binding: impl Fn(&KeySeq) -> bool,
    ) -> Option<KeySeq> {
        let armed_at = self.armed_at?;
        if now.duration_since(armed_at) < window {
            return None;
        }
        let seq = KeySeq(std::mem::take(&mut self.pending));
        self.armed_at = None;
        if seq.0 == [KeyToken::Named(NamedKey::Escape)] {
            return None;
        }
        has_binding(&seq).then_some(seq)
    }
}

/// Result of resolving one physical keystroke through [`resolve_key_name`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyResolution {
    /// Buffered as part of a chord - dispatch nothing this keystroke.
    Pending,
    /// Look this canonical name up (TF `/bind` first, then `KeyBindings`)
    /// and dispatch whatever it's bound to.
    Dispatch(String),
    /// No chord was in progress and this keystroke isn't a candidate key
    /// name either (typically a plain unmodified character) - handle it
    /// exactly as if chords didn't exist (fall through to character input,
    /// the Enter check, etc).
    NotAKey,
}

/// Resolve one physical keystroke (`code`/`modifiers`) against `app.chord`,
/// `app.tf_engine.keybindings`, `app.keybindings` and (Job 22c)
/// `app.tf_bound_keys` - a remote console client's own mirror of what the
/// SERVER has bound, so a chord bound only there still completes instead of
/// being abandoned before the caller's own `tf_bound_keys` check runs. This
/// is the single shared replacement for the old duplicated
/// `last_escape`/`recent_escape` block in `input_handler::handle_key_event`
/// and `remote_client::handle_remote_client_key` - both call sites now just
/// do:
///
/// ```ignore
/// let key_name = match chords::resolve_key_name(app, key.code, key.modifiers) {
///     KeyResolution::Pending => return KeyAction::None,
///     KeyResolution::NotAKey => None,
///     KeyResolution::Dispatch(name) => Some(name),
/// };
/// ```
///
/// and then carry on with their existing (identical) TF-bind lookup,
/// action-binding lookup, Enter check, and character-input fallthrough,
/// unchanged - all of that already worked purely off an `Option<String>`
/// key name, and this returns the same shape.
pub fn resolve_key_name(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> KeyResolution {
    let now = Instant::now();
    let window = app.chord_window;

    // Snapshot both binding tables' key strings up front. Every closure
    // below then owns these plain `Vec<String>`s instead of borrowing any
    // part of `app`, so the `app.chord.*` calls further down (which need a
    // fresh `&mut` each time) never have to fight the borrow checker over
    // disjoint fields of the same `&mut App` - a few dozen short-string
    // clones once per keystroke is immaterial next to a terminal's own
    // input latency.
    let tf_keys: Vec<String> = app.tf_engine.keybindings.keys().cloned().collect();
    let action_keys: Vec<String> = app.keybindings.bindings.keys().cloned().collect();
    // A remote console client's own local mirror of the SERVER's bound keys
    // (`GlobalSettingsMsg::tf_bound_keys_json` -> `App::tf_bound_keys`, TF-parity plan Job
    // 22c). Always empty for the master console's own in-process `App` (nothing ever
    // populates it there), so this is a no-op addition for that caller. Folded into
    // `is_prefix`/`has_binding` here too so e.g. an `Esc-x` bound ONLY on the server (never
    // locally, since this client's own `tf_engine.keybindings` above is always empty) still
    // completes as a genuine chord instead of being `Abandon`ed before
    // `remote_client::handle_remote_client_key`'s own `app.tf_bound_keys` check ever runs.
    let server_bound_keys: Vec<String> = app.tf_bound_keys.iter().cloned().collect();
    // Macro names (lower-cased), for the `key_<name>` check below - TF-parity
    // plan Job 21/P2.5. A macro named `key_<tfname>` makes an otherwise-unbound
    // named-key chord (`Esc-Left`, say) a "real" binding for chord-resolution
    // purposes too, exactly like a `/bind`/action-table entry does - without
    // this, `push` would see no binding for the completed candidate and
    // `Abandon` it (re-resolving just the trailing keystroke alone) before
    // `input_handler.rs`'s own `key_<name>` lookup ever got a chance to run.
    let macro_names: std::collections::HashSet<String> =
        app.tf_engine.macros.iter().map(|m| m.name.to_ascii_lowercase()).collect();

    let is_prefix = |candidate: &KeySeq| {
        keynames::is_prefix_of_any(tf_keys.iter().map(String::as_str), candidate)
            || keynames::is_prefix_of_any(action_keys.iter().map(String::as_str), candidate)
            || keynames::is_prefix_of_any(server_bound_keys.iter().map(String::as_str), candidate)
    };
    let has_binding = |candidate: &KeySeq| {
        let name = candidate.canonical();
        if tf_keys.contains(&name) || action_keys.contains(&name) || server_bound_keys.contains(&name) {
            return true;
        }
        keynames::key_macro_names(&name)
            .into_iter()
            .any(|macro_name| macro_names.contains(&macro_name))
    };

    if app.chord.is_pending() {
        if let Some(fired) = app.chord.expired(now, window, has_binding) {
            return KeyResolution::Dispatch(fired.canonical());
        }
    }

    let mid_chord = app.chord.is_pending();
    let token = if mid_chord {
        escape_key_to_token(code, modifiers)
    } else {
        key_event_to_token(code, modifiers)
    };

    let Some(token) = token else {
        // An exotic code neither converter recognizes: if it arrived
        // mid-chord, the buffered prefix can't be extended by it - drop the
        // prefix rather than leave it stranded for some unrelated later key
        // to (mis)combine with.
        if mid_chord {
            app.chord.cancel();
        }
        return KeyResolution::NotAKey;
    };

    if mid_chord && token == CANCEL_TOKEN {
        app.chord.cancel();
        return KeyResolution::NotAKey;
    }

    match app.chord.push(token, now, is_prefix, has_binding) {
        ChordResult::Pending => KeyResolution::Pending,
        ChordResult::Complete(seq) => KeyResolution::Dispatch(seq.canonical()),
        ChordResult::Abandon { prefix, replay: _ } => {
            if has_binding(&prefix) {
                // Rare (no default binding is both a chord prefix and
                // directly bound itself): fire the prefix's own binding for
                // *this* keystroke. The keystroke that broke the chord is
                // not replayed in the same call - `handle_key_event` only
                // ever returns one `KeyAction` per physical keystroke - so
                // it's effectively dropped; the user's next keystroke is
                // unaffected (the chord state is already clear).
                return KeyResolution::Dispatch(prefix.canonical());
            }
            // The common case (`^X` then an unbound follow-up): re-resolve
            // this exact keystroke as if no chord had been in progress at
            // all. `app.chord` is already empty (`push` cleared it before
            // returning `Abandon`), so this is a plain top-level lookup -
            // it can produce `Pending` (the replay itself starts a new
            // chord) or `Complete`, but never another `Abandon` (that needs
            // a *previously buffered* prefix, and there now isn't one).
            match key_event_to_token(code, modifiers) {
                None => KeyResolution::NotAKey,
                Some(token) => match app.chord.push(token, now, is_prefix, has_binding) {
                    ChordResult::Pending => KeyResolution::Pending,
                    ChordResult::Complete(seq) => KeyResolution::Dispatch(seq.canonical()),
                    ChordResult::Abandon { .. } => unreachable!(
                        "a fresh top-level push on an empty ChordState can never itself Abandon"
                    ),
                },
            }
        }
    }
}

/// Resolve a canonical key name (`crate::keynames` grammar) to the TF command it runs,
/// checking exactly the two layers finding A / Phase 2 step P2.5's dispatch order puts
/// ahead of the built-in action table: a `/bind`/`/def -b`/`-B` binding
/// (`app.tf_engine.keybindings`, a real - possibly nameless - TF macro; see
/// `hooks::get_binding`'s own doc comment), then a `key_<name>` macro
/// (`app.tf_engine.macros`, TF's own two-level named-key mapping - `keynames::
/// key_macro_names`). `None` means neither layer has an opinion on this key; the caller's
/// own action-table lookup (step 3) is a separate call, not folded in here, since a
/// `RunKeyBinding` sender (Job 22a/P2.7) never even reaches this function for a key it
/// already knows is an action-table binding (see `GlobalSettingsMsg::tf_bound_keys_json`).
///
/// Shared by `input_handler::handle_key_event` (the master console's live keypress path)
/// and `App::handle_ws_client_msg`'s `WsMessage::RunKeyBinding` handler (the server-side
/// counterpart a web/GUI/SSH-remote-console client's keypress reaches over the wire), so
/// "what does this bound key actually run" can never drift between the two. A `--console`
/// remote-attach client (`remote_client::handle_remote_client_key`) never calls this
/// directly - its own `app.tf_engine` never receives `/bind`/`/def` locally (typed
/// commands go straight to `WsMessage::SendCommand`, executed by the SERVER's own TF
/// engine) - it instead checks `app.tf_bound_keys` (its local mirror of
/// `GlobalSettingsMsg::tf_bound_keys_json`, Job 22c) and sends `WsMessage::RunKeyBinding`
/// for a hit, which is what reaches this function on the server side.
pub(crate) fn resolve_bound_command(app: &App, canonical_key: &str) -> Option<String> {
    if let Some(cmd) = app.tf_engine.keybindings.get(canonical_key) {
        return Some(cmd.clone());
    }
    for macro_name in keynames::key_macro_names(canonical_key) {
        if app.tf_engine.macros.iter().any(|m| m.name.eq_ignore_ascii_case(&macro_name)) {
            return Some(format!("/{macro_name}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(name: &str) -> KeySeq {
        keynames::parse_key_name(name).unwrap()
    }

    /// `is_prefix`/`has_binding` over a fixed, small binding table - enough
    /// to exercise `ChordState` on its own, with no `App` involved.
    fn lookup(bound: &'static [&'static str]) -> (impl Fn(&KeySeq) -> bool, impl Fn(&KeySeq) -> bool) {
        let is_prefix = move |candidate: &KeySeq| {
            keynames::is_prefix_of_any(bound.iter().copied(), candidate)
        };
        let has_binding = move |candidate: &KeySeq| {
            let name = candidate.canonical();
            bound.iter().any(|k| *k == name)
        };
        (is_prefix, has_binding)
    }

    #[test]
    fn test_chord_two_token_completes() {
        let (is_prefix, has_binding) = lookup(&["^X^R"]);
        let mut chord = ChordState::new();
        let now = Instant::now();

        let r1 = chord.push(KeyToken::Ctrl('X'), now, &is_prefix, &has_binding);
        assert_eq!(r1, ChordResult::Pending);
        assert!(chord.is_pending());

        let r2 = chord.push(KeyToken::Ctrl('R'), now, &is_prefix, &has_binding);
        assert_eq!(r2, ChordResult::Complete(seq("^X^R")));
        assert!(!chord.is_pending(), "completing a chord must clear the buffered prefix");
    }

    #[test]
    fn test_chord_unbound_followup_abandons_with_replay() {
        // ^X is only ever a prefix (of ^X^R), never bound on its own.
        let (is_prefix, has_binding) = lookup(&["^X^R"]);
        let mut chord = ChordState::new();
        let now = Instant::now();

        assert_eq!(chord.push(KeyToken::Ctrl('X'), now, &is_prefix, &has_binding), ChordResult::Pending);

        let r2 = chord.push(KeyToken::Char('q'), now, &is_prefix, &has_binding);
        match r2 {
            ChordResult::Abandon { prefix, replay } => {
                assert_eq!(prefix.canonical(), "^X");
                assert_eq!(replay, KeyToken::Char('q'));
            }
            other => panic!("expected Abandon, got {other:?}"),
        }
        assert!(!chord.is_pending(), "an abandoned chord must not leave a stale prefix");
    }

    #[test]
    fn test_chord_abandon_fires_prefix_own_binding() {
        // The rare ambiguous case: ^X is directly bound AND a prefix of ^X^R.
        let (is_prefix, has_binding) = lookup(&["^X", "^X^R"]);
        let mut chord = ChordState::new();
        let now = Instant::now();

        assert_eq!(chord.push(KeyToken::Ctrl('X'), now, &is_prefix, &has_binding), ChordResult::Pending);

        let r2 = chord.push(KeyToken::Char('q'), now, &is_prefix, &has_binding);
        match r2 {
            ChordResult::Abandon { prefix, replay } => {
                assert_eq!(prefix.canonical(), "^X");
                assert!(has_binding(&prefix), "^X has its own binding in this table");
                assert_eq!(replay, KeyToken::Char('q'));
            }
            other => panic!("expected Abandon, got {other:?}"),
        }
    }

    #[test]
    fn test_chord_esc_folds_into_compound_token() {
        // Esc, then ^N -> ONE compound Esc(^N) token, not two chord
        // elements (keynames.rs: Esc-<x> is a single grammar token).
        let (is_prefix, has_binding) = lookup(&["Esc-^N"]);
        let mut chord = ChordState::new();
        let now = Instant::now();

        let r1 = chord.push(KeyToken::Named(NamedKey::Escape), now, &is_prefix, &has_binding);
        assert_eq!(r1, ChordResult::Pending);

        let r2 = chord.push(KeyToken::Ctrl('N'), now, &is_prefix, &has_binding);
        assert_eq!(r2, ChordResult::Complete(seq("Esc-^N")));
    }

    #[test]
    fn test_chord_esc_left_named_key() {
        let (is_prefix, has_binding) = lookup(&["Esc-Left"]);
        let mut chord = ChordState::new();
        let now = Instant::now();

        assert_eq!(
            chord.push(KeyToken::Named(NamedKey::Escape), now, &is_prefix, &has_binding),
            ChordResult::Pending
        );
        assert_eq!(
            chord.push(KeyToken::Named(NamedKey::Left), now, &is_prefix, &has_binding),
            ChordResult::Complete(seq("Esc-Left"))
        );
    }

    #[test]
    fn test_chord_cancel_clears_pending() {
        let (is_prefix, has_binding) = lookup(&["^X^R"]);
        let mut chord = ChordState::new();
        let now = Instant::now();

        assert_eq!(chord.push(KeyToken::Ctrl('X'), now, &is_prefix, &has_binding), ChordResult::Pending);
        assert!(chord.is_pending());
        chord.cancel();
        assert!(!chord.is_pending());

        // A fresh ^X after cancelling must behave exactly like the first
        // ever keystroke - no leftover state.
        assert_eq!(chord.push(KeyToken::Ctrl('X'), now, &is_prefix, &has_binding), ChordResult::Pending);
        assert_eq!(chord.push(KeyToken::Ctrl('R'), now, &is_prefix, &has_binding), ChordResult::Complete(seq("^X^R")));
    }

    #[test]
    fn test_chord_expiry_drops_prefix_with_no_binding() {
        let (is_prefix, has_binding) = lookup(&["^X^R"]);
        let mut chord = ChordState::new();
        let t0 = Instant::now();

        assert_eq!(chord.push(KeyToken::Ctrl('X'), t0, &is_prefix, &has_binding), ChordResult::Pending);

        // Zero window: any later Instant counts as expired.
        let t1 = t0 + Duration::from_millis(1);
        let fired = chord.expired(t1, Duration::ZERO, &has_binding);
        assert_eq!(fired, None, "^X has no direct binding of its own, so expiry fires nothing");
        assert!(!chord.is_pending(), "expiry must still drop the stale prefix");
    }

    #[test]
    fn test_chord_expiry_fires_prefix_own_binding() {
        let (is_prefix, has_binding) = lookup(&["^X", "^X^R"]);
        let mut chord = ChordState::new();
        let t0 = Instant::now();

        assert_eq!(chord.push(KeyToken::Ctrl('X'), t0, &is_prefix, &has_binding), ChordResult::Pending);

        let t1 = t0 + Duration::from_millis(1);
        let fired = chord.expired(t1, Duration::ZERO, &has_binding);
        assert_eq!(fired, Some(seq("^X")));
        assert!(!chord.is_pending());
    }

    #[test]
    fn test_chord_expiry_never_fires_bare_escape() {
        // Hardcoded carve-out: even if "Escape" itself somehow had a direct
        // binding, a lone unfollowed Escape must stay a silent no-op on
        // timeout, matching the pre-Job-18 behaviour exactly.
        let (is_prefix, has_binding) = lookup(&["Escape", "Esc-b"]);
        let mut chord = ChordState::new();
        let t0 = Instant::now();

        assert_eq!(
            chord.push(KeyToken::Named(NamedKey::Escape), t0, &is_prefix, &has_binding),
            ChordResult::Pending
        );
        let t1 = t0 + Duration::from_millis(1);
        let fired = chord.expired(t1, Duration::ZERO, &has_binding);
        assert_eq!(fired, None, "bare Escape must never fire on expiry, even if \"Escape\" is bound");
        assert!(!chord.is_pending());
    }

    #[test]
    fn test_chord_not_yet_expired_stays_pending() {
        let (is_prefix, has_binding) = lookup(&["^X^R"]);
        let mut chord = ChordState::new();
        let t0 = Instant::now();

        assert_eq!(chord.push(KeyToken::Ctrl('X'), t0, &is_prefix, &has_binding), ChordResult::Pending);

        let fired = chord.expired(t0, Duration::from_secs(500), &has_binding);
        assert_eq!(fired, None);
        assert!(chord.is_pending(), "well within the window - must still be buffered");
    }

    #[test]
    fn test_is_prefix_of_any_does_not_confuse_function_key_text() {
        // "F1" must never be mistaken for a prefix of the unrelated,
        // single-token "F10" just because their canonical text happens to
        // share a leading character - see keynames::is_prefix_of_any's doc.
        let bound = ["F10"];
        assert!(!keynames::is_prefix_of_any(bound.iter().copied(), &seq("F1")));
    }

    #[test]
    fn test_is_prefix_of_any_recognizes_real_chord_prefix() {
        let bound = ["^X^R"];
        assert!(keynames::is_prefix_of_any(bound.iter().copied(), &seq("^X")));
        assert!(!keynames::is_prefix_of_any(bound.iter().copied(), &seq("^X^R")));
    }
}
