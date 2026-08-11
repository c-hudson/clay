# Inter-instance protocol hardening roadmap

Design record for making Clay's WebSocket session layer reliably ordered,
lower-bandwidth, and free of silent drops — without replacing the transport.
Full analysis and the decision not to replace WebSocket: see the assistant
turn that produced this doc (2026-07-31) and the plan file it was built from.
User-facing summary belongs in this file's "Status" sections as phases land;
no separate NOTES doc unless one is requested.

**Why not a new transport:** TCP+WebSocket already gives reliable, in-order,
framed delivery. Every ordering/loss bug Clay has hit (the "D-Termux-lines"
investigation, commit `17fdf5b`) originated in the app layer above or below
that guarantee — lock strategy, keepalive timers, an Android bridge that
reorders, unbounded queues — not in the wire protocol. Replacing WebSocket
would also have to re-derive the CLAY-KNOCK/TOFU/D8 security stack (all built
on the single-port first-byte peek) and would kill SSH tunneling (TCP-only).
See `CLAUDE.md`'s Connection Security section for what stays untouched.

**Root idea:** `seq` on `ServerData` already exists but is only used as
telemetry to *detect* damage (`ReportSeqMismatch`/`ReportDuplicate`/
`ReportOutOfOrder`). Make it the delivery *contract*: client acks its highest
contiguous seq per world, server replays exactly the gap on reconnect using
the scrollback ring it already has. This lets every client delete its
ad-hoc gap-guessing heuristics.

## Progress checklist

**On resume:** find the first unchecked box below, re-read its step
description, verify against the current tree whether it's actually done
(a halted session may have completed the code but not checked the box, or
vice versa), then continue from there. Run one step at a time — do not
parallelize implementation steps, even independent-looking ones. Verify
`cargo build --target x86_64-unknown-linux-musl --no-default-features
--features rustls-backend,ssh-transport` and `cargo test` after every step
before checking it off.

- [x] **Step 1 — Wire protocol schema only.** Add ack/resume fields with no
      behavior change: a way for the client to report highest contiguous
      acked seq per world (piggyback on `PongCheck`/`PingCheck`,
      `src/websocket.rs:459-464`, rather than a new message type), a
      `resume: Vec<(usize, u64)>` field on `AuthRequest`
      (`src/websocket.rs:42-54`), and a new `ResyncRequired { world_index:
      usize, from_seq: u64 }` variant. All new fields `#[serde(default)]` so
      old/new clients and servers stay wire-compatible. No behavior wired up
      yet — this step is pure schema + `cargo build` + existing tests
      passing unchanged.
- [x] **Step 2 — Server: resume-driven replay on (re)connect.** When
      `AuthRequest.resume` is non-empty, after `InitialState` the server
      replays each named world's missing lines via the existing
      `App::handle_request_scrollback(.., after_seq)`
      (`src/main.rs:5456`, `after_seq` handling at `:5487`) instead of
      relying on the client to notice a gap. Track each client's last-acked
      seq per world (new field on `WsClientInfo`,
      `src/websocket.rs:722-745`). Add a test exercising: connect, receive
      N lines, disconnect mid-stream, reconnect with `resume`, assert exact
      replay with no gap and no duplicate.
- [x] **Step 3 — Bounded channel + `ResyncRequired` on overflow.** Replace
      `mpsc::unbounded_channel::<WsMessage>()` (`src/websocket.rs:1558`)
      with a bounded channel. On a full queue, don't silently drop: mark the
      client desynced and send exactly one `ResyncRequired` for the
      affected world(s) once the channel has room, which Step 2's replay
      path already knows how to service. Add a test that fills the channel
      and asserts `ResyncRequired` is observed exactly once.
- [x] **Step 4 — Close remaining silent-drop sites.** Route the two
      swallowed-error sites to `log_remote_event` (`src/http.rs:67`) the
      same way `WS-SEND-FAIL` already is: the `if let Ok(json) = ...` with
      no `else` at `src/websocket.rs:1652`, and the `let _ =
      client.tx.send(...)` in `broadcast_to_owner` (`:890`),
      `broadcast_to_all` (`:912`), `broadcast_to_world_viewers` (`:1031`),
      `send_to_client` (`:933`). Low-risk, no behavior change beyond
      logging.
- [x] **Step 5 — Web client: send resume + ack, remove gap-guessing.**
      In `src/web/app.js`: on connect/reconnect, send `resume` built from
      per-world last-contiguous-seq state; periodically ack via
      `PongCheck`. Once Steps 2–3 guarantee exact replay, delete the
      heuristic machinery it replaces: `MAX_TRACKED_SEQ_GAPS`, `_seqGaps`,
      `recordSeqGapIfAny`/`findOverlappingSeqGap`/`shrinkSeqGap`/
      `insertLinesBySeq` (`:67-110`), and simplify the `ServerData`/
      `ScrollbackLines` handlers (`:2521-2680`, `:3355-3371`) to a
      straightforward ordered-append trusting the contract, with
      `ReportSeqMismatch` kept only as a should-never-happen safety net.
- [x] **Step 6 — Rust remote console: same fix.** In `src/main.rs`
      (`:4447`, `:4558`) and `src/remote_client.rs` (`:398-411`): replace
      permanent-drop-on-dedup with the resume contract from Steps 2–3 —
      send `resume` on connect, ack periodically, drop the
      sort_by_key+dedup_by_key post-hoc patch in the grep client's
      favor of trusting in-order delivery.
- [x] **Step 6a — Owner-scoped resume for multiuser (security gap found in
      Step 2).** `handle_multiuser_ws_message`'s `AuthRequest` arm
      (`src/daemon.rs:3216`) does not wire up resume replay: multiuser never
      handled `RequestScrollback`/`PongCheck` at all (pre-existing gap), and
      `App::handle_request_scrollback` (`src/main.rs:5456`) has no
      per-world owner check — calling it directly from the multiuser path
      would let one user's `resume` list pull scrollback from a world they
      don't own (the same class of bug `CLAUDE.md`'s D7 invariants call out
      for `ConnectWorld`/`SwitchWorld`: must check `world.owner ==
      username`). Add an owner-scoped wrapper (or an owner check inside
      `handle_request_scrollback` itself, gated on whether the caller is in
      multiuser mode) before wiring `PongCheck`/resume replay into
      `handle_multiuser_ws_message`. Blocks nothing upstream (single-user
      Steps continue independent of this), but must land before Step 7
      (Android) or Step 11 (docs) claim multiuser parity.
- [x] **Step 7 — Android WebView bridge: ordered delivery.** Replace the
      base64-through-`evaluateJavascript` fire-and-forget relay
      (`android/app/src/main/java/com/clay/mudclient/MainActivity.java:520-528`)
      with an ordered queue: Java appends to a `ConcurrentLinkedQueue` and
      signals JS once; JS drains via a `@JavascriptInterface`
      `drainWsQueue()` pull instead of receiving a push per message. Removes
      both the reordering risk and the ~33% base64 size inflation on the
      hot path. Independent of Steps 1–6's wire changes; safe to verify on
      a running Android build against a Termux-hosted daemon per the plan's
      verification section.
- [x] **Step 8 — Serialize broadcasts once, not per client.** Change the
      per-client channel item so a broadcast is JSON-serialized once and
      shared (`Arc<str>`) across all recipients instead of
      `serde_json::to_string` running once per client
      (`src/websocket.rs:1652`). `InitialState`/auth replies stay
      per-client. Pure perf change — no wire format difference, should not
      need new tests beyond existing ones passing.
- [x] **Step 9 — REDEFINED, not the originally-planned behavior change.**
      The original plan (filter `broadcast_to_world_viewers` by
      `client.current_world`, since Step 2's resume covers the resulting
      reconnect race) was investigated and **rejected**: `app.js`'s
      `ServerData` handler indexes `worlds[msg.world_index]`
      unconditionally, meaning every connected client (every browser tab,
      GUI, Android) already buffers *every* world's output locally
      regardless of which tab is focused — that's what makes tab-switching
      instant and keeps unseen-line badges live for background worlds.
      Filtering server-side by focused world would silently starve every
      world a client isn't actively looking at — a regression, not a
      bandwidth win. `reference/networking.md:78-79` was the one actually
      wrong artifact (documented the filtering as real when the `_world_index`
      param was always intentionally unused) — fixed to describe the real,
      deliberate design, and `broadcast_to_world_viewers`'s doc comment in
      `src/websocket.rs` was expanded to explain why, so a future reader
      doesn't "fix" this again. No wire/bandwidth behavior changed.
- [x] **Step 10 — Trim the `ServerData` envelope.** Extend
      `skip_serializing_if` to the remaining defaultable fields on
      `ServerData` (`src/websocket.rs:107`), matching the treatment
      `flush`/`gagged` already get.
- [x] **Step 11 — Rewrite `websockets.readme`.** Bring the protocol spec of
      record up to date with the post-Step-10 design: `seq`, `flush`,
      `resume`/ack, `ResyncRequired`, `RequestScrollback`, challenge-response
      auth. It currently documents none of these.
- [x] **Step 12 — Measured, deliberately not implemented.** Per this step's
      own gate ("do not start without first measuring"), measured
      representative MUD traffic (ANSI-colored combat-spam lines) against
      three shapes: envelope trim alone (Step 10, already shipped — 2-36%
      smaller depending on batch size, best on tiny messages), naive
      one-shot-per-message `flate2` deflate (the shape this step originally
      sketched: `Binary` frame + one-byte tag) — 30-54% smaller on
      multi-line batches but can *hurt* single short-line messages (8%
      smaller vs. 36% from trim alone, due to per-message dictionary-priming
      overhead with no shared context), and a persistent per-connection
      compression stream (dictionary shared across the life of the
      connection, the same idea as standard WebSocket `permessage-deflate`)
      — 93% smaller across 30 consecutive small messages, since MUD traffic
      repeats constantly (ANSI codes, JSON keys) *across* messages, not just
      within one. Conclusion: the naive version this step originally scoped
      is a modest, implementation-costly win with a real downside case;
      the version that actually delivers a large win requires persistent
      codec state negotiated and maintained on the server and all three
      clients (web/Android/Rust console) — `tungstenite`/`tokio-tungstenite`
      don't ship `permessage-deflate` support, so that would mean hand-rolling
      it, a project comparable in size to Step 7, not a quick add-on. Decided
      not to pursue either shape now: Phase A (reliability/ordering/security,
      the stated priority) is complete, and the cheap bandwidth win (Step
      10's envelope trim) already shipped. Revisit as its own scoped project
      if a specific bandwidth problem shows up in practice.

## Status

Step 1 complete as of 2026-07-31: `AuthRequest` gained `resume: Vec<(usize,
u64)>` (`src/websocket.rs:55`), `PongCheck` gained `acked: Vec<(usize, u64)>`
(`src/websocket.rs:474`), and a new `ResyncRequired { world_index: usize,
from_seq: u64 }` variant was added (`src/websocket.rs:481`) — all
`#[serde(default)]`, schema-only, no behavior wired up. `cargo build` clean,
`cargo test` 656/656 passing (same count as before the change).

Step 2 complete as of 2026-07-31: resume-driven replay is wired up on the
single-user paths (master-WS/console and the `-D` headless daemon).
`WsClientInfo` gained `acked_seq: HashMap<usize, u64>` (`src/websocket.rs:766`),
populated by the new `WebSocketServer::record_acked_seq()` (`src/websocket.rs:979`),
which is called from both `PongCheck { acked, .. }` handlers
(`src/main.rs:9600`, `src/daemon.rs:1736`) and from the `AuthRequest.resume` seed
step below. `App::handle_ws_auth_initial_state` (`src/main.rs:8095`) now takes a
`resume: Vec<(usize, u64)>` param and, when non-empty, seeds `acked_seq` then calls
the existing `handle_request_scrollback(.., after_seq: Some(last_seq))` once per
named world to replay exactly the gap — its two call sites
(`src/main.rs:9129`, `src/main.rs:14019`) were updated to pass `resume` through.
The `-D` headless daemon's separate inline `AuthRequest` handler in
`run_daemon_server` (`src/daemon.rs:336` on) got the same treatment (it doesn't
go through `handle_ws_auth_initial_state`, so was extended in parallel to keep
the two paths consistent per CLAUDE.md). Multiuser's `AuthRequest` handler
(`handle_multiuser_ws_message`, `src/daemon.rs:3216`) deliberately does **not**
get resume replay in this step: `RequestScrollback`/`PongCheck` aren't handled
in multiuser mode at all yet (pre-existing gap), and `handle_request_scrollback`
has no per-world owner check — wiring resume through it as-is would let one
user's `resume` list pull another user's world scrollback. Left for a later
step with an owner-scoped variant.

Two tests added in `src/tests.rs`
(`test_resume_replay_on_reconnect_sends_exact_gap_no_duplicate`,
`test_empty_resume_sends_no_scrollback_replay`), exercising
`handle_ws_auth_initial_state` directly against a real registered
`WsClientInfo`/channel: the first drives a 10-line world with
`resume: vec![(0, 7)]` and asserts exactly `[8, 9, 10]` comes back as one
`ScrollbackLines` reply with no duplicate and `acked_seq` seeded to 7; the
second asserts an empty `resume` produces `InitialState` only, unchanged from
pre-Step-2 behavior. `cargo build` clean (only the pre-existing russh
future-incompat notice), `cargo test` 658/658 passing (656 baseline + 2 new).

Step 3 complete as of 2026-07-31: the per-client outbound `WsMessage` channel
is now bounded instead of unbounded. `handle_ws_client`'s channel creation
(`src/websocket.rs`, `mpsc::channel::<WsMessage>(WS_CLIENT_CHANNEL_CAPACITY)`)
replaced `mpsc::unbounded_channel`; `WsClientInfo.tx` is now
`mpsc::Sender<WsMessage>` (was `UnboundedSender`). Capacity chosen:
`WS_CLIENT_CHANNEL_CAPACITY = 256` — sized against realistic bursts
(`ServerData` fanned out per socket read across every connected client, up to
several dozen/sec across a few simultaneously-busy worlds) while still
bounding worst-case per-client memory to a few hundred queued `WsMessage`
clones, well under the existing 2 MiB `max_message_size` frame cap; a client
that can't drain 256 messages' worth of backlog is meaningfully behind, which
is exactly the condition Step 3 exists to surface rather than mask. Full
rationale in the doc comment at the constant's definition.

Every send site that touches `WsClientInfo.tx` was converted from the old
infallible sync `.send()` to non-blocking `try_send()`, since a bounded
`Sender::send()` is async and calling it from these sync functions (or
`.await`-ing it from inside `handle_ws_client`'s own select loop — the same
task that drains the channel — which would deadlock if the channel were
ever full) isn't an option. Converted: `WebSocketServer::broadcast_to_owner`,
`broadcast_to_all`, `broadcast_to_world_viewers`, `send_to_client`,
`send_initial_state_and_mark` (`src/websocket.rs`); `handle_ws_client`'s own
local response sends (ServerHello/AuthResponse/Pong) via a new
`try_send_local` helper; and three call sites that bypass the
`WebSocketServer` methods and touch `WsClientInfo.tx` directly —
`App::broadcast_activity`, `App::ws_broadcast`, `App::ws_send_to_client`,
`App::ws_send_initial_state_and_mark` (`src/main.rs`) and the `/l` command's
`PingCheck` fan-out (`src/commands.rs`).

On `TrySendError::Full`, the message is no longer silently dropped: logged
via `log_remote_event("WS-CHANNEL-FULL", ..)` (`src/http.rs:67`), and if the
dropped message carries a `world_index` (via the new
`message_world_index()` helper, `src/websocket.rs`, matched over every
`WsMessage` variant that has one), the affected world is recorded in a new
`WsClientInfo.needs_resync: HashSet<usize>` field. A shared
`reconcile_resync()` helper (`src/websocket.rs`, `pub(crate)`) does the actual
`ResyncRequired { world_index, from_seq }` delivery attempt, using
`WsClientInfo.acked_seq` (Step 2) for `from_seq`: on a fresh overflow it makes
one immediate best-effort `try_send` (usually still full, so this usually just
sets the flag); the flag is then retried — piggybacked — the next time a send
to that client either (a) succeeds while the flag is still set (checked
inline in the fan-out functions' read-locked loop, reconciled after the lock
is dropped) or (b) the client's own `handle_ws_client` select loop drains a
message off `rx` (added a `needs_resync` flush check right after every
successful `ws_sink.send()`, since that task is the one guaranteed not to be
blocked by the very overflow it's trying to relieve). This is best-effort by
design per the roadmap's scope (not a full flow-control redesign) — a client
that stops receiving traffic entirely before either path fires stays flagged
until the periodic ping/pong reaper (`WS-DEAD`) or a future message reopens
the opportunity.

One test added in `src/tests.rs`
(`test_channel_full_sends_resync_required_once`): registers a `WsClientInfo`
with a test-only capacity-4 channel, overflows it via five `broadcast_to_all`
calls and asserts the client stays registered (connection not torn down) and
world 0 gets flagged `needs_resync`; then drains part of the backlog
(simulating the client catching up) and asserts the next broadcast clears the
flag and delivers exactly one `ResyncRequired { world_index: 0, .. }` — no
more, no fewer. `cargo build` clean (only the pre-existing russh
future-incompat notice, no new warnings), `cargo test` 659/659 passing (658
baseline + 1 new).

Step 4 complete as of 2026-07-31: closed the one remaining confirmed silent
serialization-drop site and re-audited `src/websocket.rs`, `src/main.rs`, and
`src/daemon.rs` for stragglers Step 3 might have missed.

**Fixed:** `src/websocket.rs` (the `Some(msg) = rx.recv()` arm of
`handle_ws_client`'s combined select loop, originally reported around
`:1926`, now the `match serde_json::to_string(&msg) { .. }` a few lines
below where that `if let Ok(json) = ...` used to be) — a serialization
failure for an outbound message (e.g. a stray NaN/Infinity float in a
settings message) previously vanished with zero trace, right after being
successfully dequeued, as if delivered. Now logged via
`log_remote_event("WS-SERIALIZE-FAIL", &client_ip, "variant=<name>: <error>")`
plus a matching `debug_log(true, ..)` line. The variant name is extracted by
taking the `Debug`-formatted message and truncating at the first `{`/`(` —
deliberately not logging the full `Debug` dump, since several `WsMessage`
variants (`UpdateWorldSettings`, `AuthRequest`, …) carry a plaintext password
field per CLAUDE.md's password-handling rule. The message is dropped (not
retried — a value that fails to serialize now will fail identically later)
but the connection is left open; only a `ws_sink.send` failure (the
pre-existing `WS-SEND-FAIL` path just above it) breaks the loop.

**Re-audit result:** every other `let _ = ...send(...)` / `if let Ok(...) =
...` site touching an actual network WebSocket client's outbound channel
(`WsClientInfo.tx`) was already converted to `try_send()` +
`log_remote_event("WS-CHANNEL-FULL", ..)` in Step 3 — confirmed by grepping
all `.tx.send(`/`.tx.try_send(` call sites in `src/websocket.rs`,
`src/main.rs`, and `src/commands.rs`: none remain on the infallible-`send`
form. The multiuser broadcast path (`AppEvent::MultiuserServerData` /
`MultiuserDisconnected` / `MultiuserPrompt` handlers in `src/daemon.rs`,
around `:2182-2259`) routes through `WebSocketServer::broadcast_to_owner`,
already covered. GMCP/MSDP/notification/media sends (`ws_broadcast`,
`ws_send_to_client`, `ws_broadcast_to_world` call sites in `src/main.rs` and
`src/daemon.rs`) all route through the same already-converted fan-out
methods.

Two categories of `let _ = tx.send(...)` were found and deliberately left
alone as out of this step's scope ("server-side silent-drop logging only"
per the task):
- `self.gui_tx` sends (`src/main.rs:6911`, `:6967`, `:7025`, `:7125`) — an
  **unbounded**, in-process channel to the locally-embedded GUI (master
  `--gui` mode), not a network WebSocket client; its only failure mode is
  "receiver dropped" (GUI window closed), not backpressure, so it isn't the
  class of bug this roadmap targets.
- `self.ws_client_tx` sends (`src/main.rs:4456`, `:4603`, `:4653`, `:4802`)
  — Clay acting as a remote **client** connecting to another Clay's server
  (`--console`/remote-console mode). This is explicitly Step 6's territory
  ("Rust remote console: same fix") and the task instructions for this step
  excluded touching `src/remote_client.rs` / remote-console send paths.

No new tests added (logging-only change, per the task's guidance that a test
here is optional). `cargo build --target x86_64-unknown-linux-musl
--no-default-features --features rustls-backend,ssh-transport` clean (only
the pre-existing russh future-incompat notice, no new warnings). `cargo test`
(same flags) 659/659 passing — unchanged from the Step 3 baseline, as
expected for a logging-only step.

Step 5 complete as of 2026-07-31: `src/web/app.js` now sends `AuthRequest.resume`,
acks periodically via `PongCheck.acked`, and handles `ResyncRequired` — the
reconnect path no longer guesses what it lost.

**Added:**
- `lastContiguousSeq(world)` (`app.js:115-125`) — the highest seq such that every
  seq up to and including it has actually been received. Deliberately NOT the
  same as `world._max_seq`: if `world._seqGaps` (the existing mid-stream
  reordering tracker) has an open hole, `_max_seq` can already be past it (a
  later batch leapfrogged the missing one), so sending `_max_seq` as a resume
  anchor would silently tell the server "I have everything up to here" and hide
  the hole from the exact replay this step exists to provide. When gaps are
  open, returns `(earliest gap start) - 1`; otherwise falls back to `_max_seq`.
- `buildResumeAckList()` (`app.js:127-139`) — builds the shared
  `[[world_index, last_contiguous_seq], ...]` shape used by both
  `AuthRequest.resume` and `PongCheck.acked` (same wire shape per
  `websocket.rs`), from the live `worlds` array via `lastContiguousSeq()`.
  Worlds with nothing received yet (seq 0) are omitted.
- `resume: buildResumeAckList()` added to all four `AuthRequest` send sites:
  the `AUTO_PASSWORD` auto-login path (`:1481`), `tryAuthWithKey()` (`:4054`),
  and both branches (native `crypto.subtle` + `sha256Fallback`) of
  `authenticate()` (`:4123`, `:4136`). Built from the pre-reconnect `worlds`
  array at send time, so indices line up with what the server assigned in the
  same session — the same index-stability assumption the pre-existing
  `RequestScrollback{world_index}` calls already relied on.
- `case 'PingCheck'` (`app.js:3145`) now includes `acked: buildResumeAckList()`
  on its `PongCheck` reply (previously `nonce` only).
- The 30s keepalive `setInterval` (`app.js:10920` area) now also sends
  `{ type: 'PongCheck', nonce: 0, acked: buildResumeAckList() }` alongside the
  existing `Ping`, confirmed safe by reading the server's `PongCheck` handler
  (`main.rs:9664`, `daemon.rs:1736`): it processes `acked` unconditionally, not
  gated on the nonce matching an outstanding `/remote` `PingCheck`.
- `case 'ResyncRequired'` (`app.js:3403`) — new handler: on
  `{ world_index, from_seq }`, calls `requestGapFill(world_index, from_seq)`
  rather than duplicating the `RequestScrollback`/`_gapFillPending` logic.
  `requestGapFill()` (`app.js:3998`) gained an optional `fromSeq` param
  (distinguishing "not passed" from the legitimate explicit value `0`, which a
  falsy-check would have wrongly routed to a full normal backfill instead of a
  targeted resync) so the reconnect-continuation call (no arg, uses
  `world._max_seq`) and the live-resync call (explicit `from_seq` from the
  server) share one implementation.
- `world._resumedFromServer` (`app.js:2248`, set in the `InitialState`
  handler's per-world `forEach`) — true only for a world hydrated from the
  in-memory `priorWorld` (this session's own reconnect) with
  `lastContiguousSeq(priorWorld) > 0`, i.e. a world that was actually part of
  the `resume` list just sent. Drives two things: `world._gapFillPending` is
  seeded from it (`:2287`) so the server's unprompted resume-replay
  `ScrollbackLines` is handled as an append rather than misread as a
  backward/prepend response; and `startBackfill()`'s hydrated-world loop
  (`:3680`) now skips calling `requestGapFill()` for these worlds (the server
  is already about to push exactly that range unprompted — asking again would
  be a redundant round trip). Cache-hydrated worlds (`cachedWorld`, the
  cross-session IndexedDB path) are unaffected and still go through the old
  client-driven `requestGapFill()` call, because they structurally cannot be
  part of `resume` — their server-assigned index isn't known until the very
  `InitialState` that follows arrives.

**Removed / retired:** the *reconnect-time* use of `_max_seq`-only gap
recovery — before this step, `startBackfill()` called `requestGapFill(idx)`
unconditionally for every hydrated-from-local world, asking the server for
"everything after my `_max_seq`" with no way to express "and there was also an
older hole I never got." That guess is what `resume` + `lastContiguousSeq()`
replaces for the in-memory-reconnect case: the AuthRequest sent on the socket
reopening already carries the exact boundary (gap-aware), and the server (Step
2's `handle_ws_auth_initial_state`) replays exactly the missing range via its
own scrollback ring — unbounded and exact, not capped and best-effort. No
function was deleted outright; `requestGapFill()` was generalized (extra
`fromSeq` param) rather than removed, since it's still legitimately needed for
the cache-hydrated cold-start case and now also for live `ResyncRequired`.

**Deliberately kept — do not remove before Step 7:** `world._seqGaps`,
`recordSeqGapIfAny`/`findOverlappingSeqGap`/`shrinkSeqGap`/`insertLinesBySeq`
(`app.js:69-153`), and the `ServerData`/`ScrollbackLines` handlers' dedup/gap
logic are all still in place, untouched in behavior. Per the task constraint:
the Android WebView→JS bridge (`evaluateJavascript` in `MainActivity.java`) is
still fire-and-forget and can still reorder live frames on its own, independent
of anything the server does — that's tracked separately as the still-open Step
7. Deleting this mid-stream compensation now would turn that into an actual
live-UI line-loss/misordering regression on Android (the priority platform)
until Step 7 ships. Comments at `app.js:51-78` were extended (not rewritten) to
say this explicitly, and to note the one new wrinkle: `lastContiguousSeq()`
now also reads `_seqGaps` for the resume/ack contract, so a gap that ages out
of the bounded array is (in the pathological case of 2000+ simultaneously open
untracked holes) also invisible to resume/ack — `MAX_TRACKED_SEQ_GAPS` was
raised from 50 to 2000 (`app.js:79`) to make that corner case require an
absurd amount of concurrent unresolved reordering rather than a realistic one,
since each tracked gap is just two integers and 2000 of them is negligible
memory. This tradeoff is exercised explicitly in Test 7 of the verification
below rather than left implicit.

**Verification:** No live browser is available in this sandbox (same
constraint prior fixes in this project history hit), so this was verified the
same way: `node --check src/web/app.js` passes (syntax only — confirms the
`include_str!()`-embedded file, which `cargo build` can't validate as JS, is
still valid), and the pure gap/resume/ack functions
(`recordSeqGapIfAny`/`findOverlappingSeqGap`/`shrinkSeqGap`/`insertLinesBySeq`/
`lastContiguousSeq`/`buildResumeAckList`/the `requestGapFill` decision logic)
were copied verbatim into a standalone Node script and exercised against
constructed message sequences — 17/17 assertions passed:
- T1: fully in-order delivery — no gap recorded, `lastContiguousSeq` equals
  `_max_seq`.
- T2: a batch skips ahead (1..5 then 11..15, skipping 6..10) — a gap is
  recorded for 6..10, and `lastContiguousSeq` correctly reports 5, not the
  post-hole `_max_seq` of 15 (this is the exact bug class Step 5 fixes: a
  naive `_max_seq`-based resume anchor would have hidden the hole).
- T3: the missing batch (6..10) arrives late — `findOverlappingSeqGap`
  recognizes it as recoverable gap-fill rather than a duplicate, the lines are
  spliced into the correct seq-ordered position via `insertLinesBySeq`, the
  gap closes via `shrinkSeqGap`, and `lastContiguousSeq` then correctly
  advances all the way to 15.
- T4: a genuine duplicate (an old range that does NOT overlap any recorded
  gap) is still correctly identified as a duplicate (`findOverlappingSeqGap`
  returns -1) — confirms the existing dedup path is untouched.
- T5: `buildResumeAckList()` against a 3-world array (clean/no-history/open-gap)
  produces the exact expected `[[0, 42], [2, 50]]`, correctly omitting the
  empty world and correctly reporting the gapped world's pre-hole boundary
  rather than its raw `_max_seq` of 100.
- T6/T6b/T6c: `requestGapFill`'s `fromSeq` handling — an explicit `from_seq: 0`
  (a legitimate value `ResyncRequired` can send) still issues a targeted
  `RequestScrollback` rather than being treated as "no anchor"; no anchor and
  no explicit `fromSeq` still falls back to a normal backfill (unchanged
  behavior); the reconnect-continuation call with no `fromSeq` arg still uses
  `world._max_seq` (unchanged behavior).
- T7: confirms `MAX_TRACKED_SEQ_GAPS` actually bounds `_seqGaps` at 2000 and
  demonstrates (rather than just asserting in prose) the accepted corner-case
  tradeoff — the oldest gap ages out and is no longer visible to
  `lastContiguousSeq()`.

`cargo build --target x86_64-unknown-linux-musl --no-default-features
--features rustls-backend,ssh-transport` clean (only the pre-existing russh
future-incompat notice, no new warnings — expected, this step touches no Rust).
`cargo test` (same flags) 659/659 passing, unchanged from the Step 4 baseline.
No Rust files were modified in this step.

Step 6 complete as of 2026-07-31: the Rust `--console` remote client and the
`--grep` client now follow the same resume/ack contract as the web client,
adapted to how each one actually connects.

**`src/main.rs` — `--console` client's receiving side
(`App::handle_remote_ws_message`, ~`:4443-4934`):**
- **Dedup reframed, not just relabeled** (`:4460-4467`, `:4592-4596`): the
  `msg_seq <= max_received_seq` checks (`ServerData` and `WorldStateResponse`
  handlers) are unchanged in behavior but recommented — a match here now means
  "the server sent something we've already fully processed" (safe, recoverable
  overlap, e.g. a resume reply landing on top of live traffic), not the old
  "permanently lost, nothing to be done" framing.
- **New: mid-stream gap tracking** — `World` gained `pending_gap: Option<(usize,
  u64)>` (`:2664-2676`, initialized `None` at `:2760`): `(local output_lines
  index the gap starts at, last contiguous seq before it)`. Set in the
  `ServerData` handler (`:4478-4492`) when an incoming `msg_seq` jumps ahead of
  `max_received_seq + 1` — the signature of a server-side channel-overflow drop
  (Step 3) — recording exactly where in the local buffer the hole is, since more
  `ServerData` (and therefore more buffer growth) can legitimately arrive before
  the gap-fill does.
- **New: `WsMessage::ResyncRequired` handler** (`:4890-4919`) — previously fell
  into the catch-all `_ => {}` (silently ignored). On receipt, sends
  `RequestScrollback { world_index, count: 10_000, before_seq: None, after_seq:
  Some(after_seq) }` — reusing the existing scrollback-request mechanism, no new
  wire message. `after_seq` prefers the client's own `pending_gap` boundary over
  the server-supplied `from_seq` when both are known (the client's is exact,
  recorded the instant the jump was seen; `from_seq` reflects whatever was last
  acked, which can lag). If the client hadn't independently noticed a gap, it
  seeds `pending_gap` here instead, using `from_seq` and the current buffer
  length as a best-effort splice point.
- **`ScrollbackLines` handler reworked** (`:4785-4823`): now checks
  `world.pending_gap` first. If set, the reply is treated as the gap-fill: lines
  with `seq <= last_contiguous_seq` are filtered (defensive — the request may
  have used an older boundary than the client's own, so the reply can
  legitimately overlap what's already held), the rest are **spliced into
  `output_lines` at the recorded index** (not prepended to the front, which is
  what the old code did unconditionally and which would have put the gap
  content before the client's entire prior history), `scroll_offset` is only
  shifted if the splice landed at or before it, and the function returns early
  — deliberately bypassing the historical-backfill state machine
  (`backfill_queue`/`backfill_exhausted`/`backfill_advance_to_next`) below,
  which exists only for `before_seq` older-history requests and has nothing to
  do with a live resync. When `pending_gap` is `None`, behavior is byte-for-byte
  the pre-Step-6 prepend path (older-history backfill).
- **`PingCheck` handler now acks** (`:4881-4888`): replies with `PongCheck {
  nonce, acked: self.build_resume_ack_list() }` instead of `acked: Vec::new()`.
  New helper `build_resume_ack_list()` (`:4921-4934`) reports, per world with
  any data, `pending_gap`'s pre-gap boundary if a gap is outstanding, else
  `max_received_seq` — mirrors `app.js`'s `lastContiguousSeq()`/
  `buildResumeAckList()` for the exact same reason: acking `max_received_seq`
  while a gap is open would tell the server "I have everything up to here" and
  hide the hole from any future resume. Unlike `app.js`'s `_seqGaps` (an
  unbounded-count tracker, since the JS/Android bridge can reorder arbitrarily),
  this client tracks at most **one** outstanding gap per world — proportionate
  to its actual failure mode (a server-acknowledged drop, not client-side
  reordering) per PROTOCOL-ROADMAP.md's Step 6 scoping note. A second gap
  opening before the first resolves keeps the first gap's recorded position
  (documented limitation, same spirit as `app.js`'s documented
  `MAX_TRACKED_SEQ_GAPS` tradeoff).
- **`AuthRequest.resume` deliberately left empty** at every send site in
  `src/remote_client.rs` (commented in place at each): `run_console_client`
  has no in-process reconnect loop — a dropped connection ends the process
  (breaks its event loop and returns) rather than looping back to reauthenticate,
  and `App`/`World` state doesn't exist yet at the point `AuthRequest` is sent
  (before `InitialState`). So there is never prior per-world state to resume
  from at connect time for this client; live gap recovery for a connection that
  *stays* up is handled entirely by the `ResyncRequired` path above. Comments
  added at each `AuthRequest` construction site (console, grep, and the
  unrelated `/import` settings-fetch client) explaining this so a future reader
  doesn't wire up a pointless always-empty resume differently at each site.

**`src/remote_client.rs` — `--grep` client (`run_grep_client`):**
- **History-search mode** (the non-`--follow`, fetch-then-exit path,
  `:399-421` before this step): removed the post-hoc `sort_by_key(seq)` +
  `dedup_by_key(seq)` over each world's collected lines. Investigation found
  the dedup wasn't actually compensating for reordering (ordering is
  guaranteed, per the reasoning in this step's task) — it was papering over a
  **real overlap bug**: the first `RequestScrollback` per world used
  `before_seq: None`, which the server's `handle_request_scrollback` treats as
  "send the last N lines" — the *same* range `InitialState` already provided
  via `output_lines_ts`, guaranteed to duplicate on any world with more history
  than `remote_initial_lines` (default 100) but fewer than the request's
  `count` (10000). Fixed at the root instead of re-adding a different
  post-hoc patch: the "pre-populate from `InitialState`" step now runs
  *before* the request loop (reordered, `:345-364`), and the first
  `RequestScrollback` per world now anchors on the minimum seq already held
  (`before_seq: Some(min_seq)` when non-empty) instead of `None` — so every
  request, first and subsequent, only ever fetches strictly older,
  non-overlapping ranges. Combined with guaranteed in-order delivery and the
  fact that each reply's `lines` already arrives seq-ascending and is always
  prepended in oldest-first request order, the accumulated per-world `Vec` is
  provably sorted with no duplicates without any post-hoc pass. (The
  *separate*, still-present `all_lines.sort_by(ts, seq)` a few lines below
  merges *different worlds'* lines into one chronological stream for display —
  that's cross-world interleaving, not a reordering workaround, and was left
  untouched.)
- **No resume/ack wiring added** to either grep-client mode (`--follow` or
  history-search) — decided against, not overlooked:
  - `--follow` mode never buffers or tracks per-world seq state at all; it
    prints matches as an ephemeral stream and holds nothing to resume. Its
    `PingCheck` handler (`:309-315`) keeps `acked: Vec::new()`, commented in
    place explaining there's no contiguous-seq boundary to report. A dropped
    connection during `--follow` just ends the process (same as `--console`) —
    no reconnect loop exists to benefit from `resume` either.
  - History-search mode is a genuine one-shot: connect, backfill everything,
    print, exit, with no retry on a dropped connection
    (`Some(Ok(Message::Close(_))) | None => break;` just stops with whatever
    was collected so far). Its `PingCheck` handler (`:406-414`) also keeps
    `acked: Vec::new()`, commented for the same reason — no reconnect path
    exists to benefit from an anchor, and by the time any `PingCheck` could
    plausibly arrive the client is either still actively backfilling (no
    stable boundary yet) or about to exit.
  - `AuthRequest.resume` stays empty here too, for the same "no prior state at
    first connect" reason as `--console` (commented at its send site,
    `:179-190`). The unrelated `/import` settings-export client
    (`run_import_client`) also keeps `resume` empty at both its `AuthRequest`
    sites (`:571-597`) — it never touches world/scrollback state at all, so
    resume doesn't apply; commented there too.

**Test:** two new tests added to `src/tests.rs`
(`test_console_client_resync_gap_fill_restores_order_no_loss`,
`test_console_client_pong_check_acks_pre_gap_boundary`), both driving
`App::handle_remote_ws_message` directly — this step's test coverage is of the
*client's* receiving side, unlike Step 2's tests of the server's sending side.
The first drives: `ServerData` seq 1..5 (contiguous) → `ServerData` seq 11..15
(a jump, 6..10 missing) → asserts `pending_gap == Some((5, 5))` and that the
seq-11 batch was still displayed, not dropped → `ResyncRequired { from_seq: 5
}` → asserts exactly one `RequestScrollback { after_seq: Some(5), .. }` is
sent → `ScrollbackLines` with seq 6..10 → asserts `output_lines` has all 15
lines with `pending_gap` cleared, and — the specific regression this step's
splice logic exists to prevent — that reading the buffer front-to-back
reproduces `line1..line15` in exact order (a naive prepend would have put
6..10 before 1..5; a naive append would have put them after 11..15). The
second test asserts `PongCheck.acked` reports the pre-gap boundary (2), not
the post-jump `max_received_seq` (5), while a gap is outstanding — the
hidden-hole bug `app.js`'s Step 5 `lastContiguousSeq()` was written to avoid,
confirmed here for the Rust client's analogous `build_resume_ack_list()`.

`cargo build --target x86_64-unknown-linux-musl --no-default-features
--features rustls-backend,ssh-transport` clean (only the pre-existing russh
future-incompat notice, no new warnings). `cargo test` (same flags) 661/661
passing (659 baseline + 2 new).

Step 6a complete as of 2026-07-31: multiuser's `AuthRequest.resume` now gets
resume-driven replay, and `RequestScrollback`/`PongCheck` are now handled in
multiuser mode at all — all three owner-scoped so one user can never read
another user's world scrollback via a client-supplied `world_index`.

**`src/main.rs:5686-5707` — `App::handle_request_scrollback_owned`:** new
owner-checked wrapper around the existing `handle_request_scrollback`
(`:5592`), added immediately after it. Takes an extra `owner: &str`; looks up
`self.worlds.get(world_index)` and only delegates to
`handle_request_scrollback` if `world.owner.as_deref() == Some(owner)`,
otherwise silently no-ops (no reply at all — an error reply would itself leak
whether `world_index` exists). `handle_request_scrollback` itself is
untouched, so its existing single-user callers (master-WS, `-D` daemon) are
unaffected, per the task's constraint not to complicate them.

**`src/daemon.rs` — `handle_multiuser_ws_message`:**
- New private helper `owner_filtered_pairs(app, uname, pairs)`
  (`:3207-3212`, just above the function) — filters a `(world_index, seq)`
  list down to entries whose world is owned by `uname`. Shared by the
  `AuthRequest.resume` and `PongCheck.acked` arms so neither can seed
  `acked_seq` for, or replay from, a `world_index` belonging to another user.
- `AuthRequest` arm (`:3228-3252`, was `:3216`): now captures `ref resume`
  and, when non-empty, filters it through `owner_filtered_pairs` *before*
  seeding `acked_seq` or replaying, then calls
  `handle_request_scrollback_owned` once per surviving `(world_index,
  last_seq)` pair — mirroring Step 2's single-user
  `handle_ws_auth_initial_state` but owner-scoped at both the ack-seeding and
  replay steps.
- New `RequestScrollback` arm (`:3255-3262`) — previously absent entirely (a
  multiuser client could not scroll back at all; confirmed by grepping the
  match arms before this change, it fell into the trailing `_ => {}`).
  Delegates straight to `handle_request_scrollback_owned` with the
  connecting client's username.
- New `PongCheck` arm (`:3263-3282`) — also previously absent (fell into
  `_ => {}`, so a multiuser `/remote` liveness ping and any real ack traffic
  were silently swallowed). Records `acked_seq` via
  `WebSocketServer::record_acked_seq`, same as the single-user handlers, with
  entries owner-filtered as defense-in-depth (analysis in the code comment:
  not strictly required for the leak this step targets, since
  `ResyncRequired` delivery only ever targets worlds actually broadcast to a
  client via `broadcast_to_owner`, i.e. that client's own worlds — but cheap
  to filter anyway so a bogus `world_index` can't write into this client's
  per-world bookkeeping).

**Tests** (`src/daemon.rs`, new `#[cfg(test)] mod resume_owner_scoping_tests`
at end of file, after `change_password_tests`):
- `resume_and_request_scrollback_cannot_read_another_users_world` — the
  required security test. Builds a two-user, two-world `App` (`two_owner_app()`:
  world 0 owned by "alice", world 1 owned by "bob", each with output_lines seq
  1..=10). Registers only alice's WS client, then drives two attacks as her:
  (1) `AuthRequest { resume: vec![(1, 0)], .. }` naming bob's world_index, (2)
  a direct `RequestScrollback { world_index: 1, .. }`. Asserts alice's channel
  receives zero `ScrollbackLines` for world 1 in both cases, and that
  `acked_seq` was never seeded for world 1 in her `WsClientInfo` either.
- `resume_replays_own_world_scrollback_correctly` — the required positive
  companion (mirrors Step 2's single-user test through the multiuser path):
  alice resumes with her own world_index 0 at `last_seq=7`, asserts she gets
  exactly one `ScrollbackLines` reply with `seqs == [8, 9, 10]` and
  `acked_seq[0] == 7`.

**Confirmed the leak test actually exercises the vulnerability (not
vacuous):** temporarily reverted `handle_request_scrollback_owned` to skip
its owner check (delegate unconditionally) and `owner_filtered_pairs` to
return its input unfiltered, then reran the test suite. Result:
`resume_and_request_scrollback_cannot_read_another_users_world` **failed**
with `alice must never receive ScrollbackLines for bob's world_index via
AuthRequest.resume, got [(1, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10])]` — i.e.
without the fix, alice's own resume request pulls back all ten of bob's
lines verbatim, proving the test would have caught the original gap. The
positive test still passed in this reverted state (expected — it never
touches another user's world). Both temporary edits were then reverted
(files restored from pre-edit backups) and the full suite rerun clean before
reporting this step done.

`cargo build --target x86_64-unknown-linux-musl --no-default-features
--features rustls-backend,ssh-transport` clean (only the pre-existing russh
future-incompat notice, no new warnings). `cargo test` (same flags) 663/663
passing (661 baseline + 2 new).

Step 7 complete as of 2026-07-31: the Android WebView bridge's per-message
`evaluateJavascript` push was replaced with an ordered-queue pull model. No
Rust files touched (as expected — this step is Java/JS only).

**`android/app/src/main/java/com/clay/mudclient/MainActivity.java`:**
- New fields (added next to the existing `nativeWebSockets` map): a
  `ConcurrentLinkedQueue<WsQueueItem> wsMessageQueue` and a private static
  `WsQueueItem { int id; String message; }` pair class, carrying the
  connection id alongside each queued frame so JS can keep discarding
  messages from a non-winning racing connection attempt, same as before.
- `connectWebSocket()`'s `WebSocketCallback.onMessage(String message)`
  override (was the base64-encode-then-`evaluateJavascript` push, previously
  reported around `:520-528`): now `wsMessageQueue.add(new WsQueueItem(id,
  message))` followed by `webView.post(() -> webView.evaluateJavascript(
  "if (typeof onNativeWsQueueReady === 'function') onNativeWsQueueReady();",
  null))` — the evaluateJavascript call carries no data, only a "go pull"
  signal, so it no longer matters whether back-to-back calls execute in
  the order issued (the documented WebView race this step exists to close).
- New `@JavascriptInterface public String drainWsQueue()` on the
  `AndroidInterface` inner class (next to `hasNativeWebSocket()`): atomically
  polls every queued `WsQueueItem` off the FIFO queue and returns them as a
  JSON array of `[id, message]` pairs (built with `org.json.JSONArray`/
  `JSONObject` for correct escaping), oldest first. No base64 — `message` is
  the raw JSON text received from the WebSocket verbatim, since
  `evaluateJavascript` no longer carries the payload on this path.
- `NativeWebSocket.java` was deliberately left untouched, per the task's
  constraint: it already just forwards each OkHttp `onMessage(WebSocket,
  String)` callback to `WebSocketCallback.onMessage(String)` via
  `mainHandler.post(...)`, in order, per connection. The actual relay-to-JS
  logic (the part that needed fixing) lived in `MainActivity.java`'s
  `WebSocketCallback` implementation, not in `NativeWebSocket.java` itself.
- Checked `ClayForegroundService.java` and the rest of the Android sources
  for other `onNativeWebSocketMessageBase64`/push-per-message call sites:
  none found (`ClayForegroundService.java` only references the WebSocket
  connection for keeping it alive in the background, not message relay).
  The unrelated `window.onNativeWebSocketMessage` (non-base64, `app.js:1692`)
  and `onNativeWebSocketMessageBase64` (`app.js:1702`) handlers were left in
  place — `onNativeWebSocketMessageBase64` has no remaining Java caller after
  this change (dead code now, same as the already-dead non-base64 variant
  next to it) but removing either is a separate cleanup, out of this step's
  scope, and was left alone rather than risk an unrelated behavior change.

**`src/web/app.js`:**
- New `window.onNativeWsQueueReady` handler, added directly after the
  existing `onNativeWebSocketMessageBase64` definition in
  `setupNativeWebSocketCallbacks()` (`app.js:1702` area): calls
  `window.Android.drainWsQueue()`, `JSON.parse()`s the returned array, then
  for each `[id, data]` pair — in order, synchronously — applies the same
  `id !== winnerAttemptId` discard check the base64 handler uses and feeds
  `JSON.parse(data)` through the exact same `handleMessage(msg)` call used by
  every other transport path. No base64 decode needed on this path.
- No other `app.js` changes. Per the task's explicit instruction, the
  `_seqGaps`/`insertLinesBySeq` mid-stream compensation machinery
  (`app.js:51-156`) and its Step-5 doc comments were left untouched — it's
  now a no-op safety net on Android once this bridge fix is live/verified,
  not something to delete as part of this step.
- Per CLAUDE.md's "all UI changes must be reflected in all interfaces" rule:
  this is a transport/bridge-layer fix scoped entirely to the Android
  WebView↔Java bridge, with no new/changed UI surface, so console, web-
  standalone, and webview-desktop-GUI needed no changes and got none.

**Verification:**
- `cd android && ./gradlew assembleDebug` — **BUILD SUCCESSFUL**. Java
  compiled clean (`compileDebugJavaWithJavac` had only the pre-existing
  "source/target value 8 is obsolete" notices, unrelated to this change);
  the full debug APK assembled successfully, confirming both new/changed
  Java (`wsMessageQueue`, `WsQueueItem`, the rewritten `onMessage` override,
  `drainWsQueue()`) compiles and links against `org.json`/`webView.post`.
- `node --check src/web/app.js` — passed (using a Node 20.18.1 build staged
  in the scratchpad directory, same approach prior roadmap steps used since
  no system Node is on PATH in this sandbox).
- `cargo build --target x86_64-unknown-linux-musl --no-default-features
  --features rustls-backend,ssh-transport` — clean, only the pre-existing
  russh future-incompat notice, no new warnings (expected: no Rust files
  touched in this step).
- `cargo test` (same flags) — 663/663 passing, unchanged from the Step 6a
  baseline (expected: no Rust files touched).
- **Honest verification limits:** no live Android device or emulator was
  used — none was required or expected per the task's own verification
  section. This step's correctness rests on (a) a successful real compile of
  the changed Java against the actual Android SDK/NDK toolchain at
  `/home/adrick/Android/Sdk`, producing a working debug APK, and (b) careful
  code review of the threading/ordering argument (queue is FIFO and
  thread-safe; the `evaluateJavascript` signal call carries no data, so its
  documented lack of an execution-order guarantee can no longer reorder MUD
  output — at worst it can only delay or coalesce *when* a drain happens,
  never *what order* a drain returns) — not by observing the fix work
  end-to-end inside a running WebView against a live Clay server. That live
  check remains undone.

Step 8 complete as of 2026-07-31: broadcast fan-out now serializes a `WsMessage` to JSON
exactly once per call and shares the result across every recipient, instead of every
recipient's own receive loop independently running `serde_json::to_string` on its own
clone of the same message.

**`src/websocket.rs` — new `Outbound` channel item type (`:104-109`):**
```rust
#[derive(Clone, Debug)]
pub(crate) enum Outbound {
    Shared(std::sync::Arc<str>),   // pre-serialized JSON, for broadcasts
    Message(Box<WsMessage>),        // per-client message, serialized individually
}
```
`WsClientInfo.tx` (`:930-933`) changed from `pub mpsc::Sender<WsMessage>` to
`pub(crate) mpsc::Sender<Outbound>` (narrowed from `pub` to `pub(crate)` too, since
`Outbound` itself is `pub(crate)` — kept the compiler's `private_interfaces` warning from
firing rather than widening `Outbound`'s own visibility for no reason). `handle_ws_client`'s
channel creation (`:1903`) is now `mpsc::channel::<Outbound>(WS_CLIENT_CHANNEL_CAPACITY)`.

New `serialize_for_broadcast(&WsMessage) -> Option<Arc<str>>` helper (`:119-133`,
`pub(crate)` — also called from `main.rs`): does the one `serde_json::to_string` call for a
broadcast and wraps the result in `Arc<str>`; on failure logs `WS-SERIALIZE-FAIL` **once**
(tagged `"broadcast"` rather than a client IP, since there's no single recipient yet at this
point) and returns `None` so the caller sends to nobody — same net effect as the old
per-client failure (every recipient's independent `to_string` call failed identically and
was dropped), just one log line instead of N.

**Broadcast call sites — now serialize once, send `Outbound::Shared` clones:**
- `WebSocketServer::broadcast_to_owner` (`src/websocket.rs:1107-1145`)
- `WebSocketServer::broadcast_to_all` (`:1158-1188`)
- `WebSocketServer::broadcast_to_world_viewers` (`:1350-1382`)
- `App::ws_broadcast` (`src/main.rs:7065-7106`) — the single-user/master-GUI-mode
  equivalent of the above three; identical `msg` fanned out to every authenticated client
  regardless of world, so it's broadcast-shaped by the same test used for the
  `WebSocketServer` methods. (`App::ws_broadcast_to_world`, `src/main.rs:7278-7294`, was
  already covered for free — it just delegates to `broadcast_to_world_viewers`.)

Each of these computes `world_index` from the message first (unchanged), then calls
`serialize_for_broadcast(&msg)` once, then loops over eligible clients sending
`Outbound::Shared(shared_json.clone())` — an `Arc<str>` refcount bump per recipient, not a
JSON copy or a `WsMessage` clone.

**Per-client sends — audited, kept as `Outbound::Message` (serialized individually,
unchanged behavior):**
- `WebSocketServer::send_to_client` / `send_initial_state_and_mark` (`:1206-1252`) —
  single recipient by construction, nothing to share.
- `reconcile_resync`'s `ResyncRequired` send and `try_send_local` (`:110-198`) — both
  carry per-client content (`ResyncRequired.from_seq` is read from that specific client's
  own `acked_seq`; `try_send_local` covers pre-auth/one-shot sends like `ServerHello`/
  `AuthResponse`/`Pong`).
- `App::broadcast_activity` (`src/main.rs:5802-5836`) — checked and confirmed **not**
  broadcast-shaped despite the name: it calls `self.activity_count_excluding(exclude)`
  per client, where `exclude` depends on that client's own `paused`/`current_world` state,
  so the count genuinely differs per recipient. Left as `Outbound::Message`.
- `App::ws_send_to_client` / `ws_send_initial_state_and_mark` (`src/main.rs:7144-7212`) —
  single recipient, mirrors the `WebSocketServer` methods above.
- `/l`'s `PingCheck` fan-out (`src/commands.rs:1195-1204`) — same `nonce` to every client,
  so technically broadcast-shaped, but left sending `Outbound::Message` per client rather
  than adding a fourth serialize-once call site: it's a rare admin-triggered command (not
  a steady-state hot path like `ServerData`), so the perf win is negligible and the task
  scoped serialize-once specifically to `broadcast_to_owner`/`broadcast_to_all`/
  `broadcast_to_world_viewers` (plus `ws_broadcast`, its obvious main.rs analog).

**`handle_ws_client`'s receive loop** (`src/websocket.rs:1998-2059`, was the
`Some(msg) = rx.recv() => { match serde_json::to_string(&msg) { .. } }` block): now
matches on the received `Outbound` first. `Shared(json)` skips serialization entirely —
`json.to_string()` (an owned-`String` copy of the already-serialized JSON, not a
re-serialization) goes straight to `ws_sink.send(WsRawMessage::Text(..))`; a `Shared` value
can never hit the `WS-SERIALIZE-FAIL` path since it was only constructed after a successful
`serde_json::to_string` at the broadcast call site. `Message(msg)` runs
`serde_json::to_string(&msg)` exactly as before Step 8, including the existing
`WS-SERIALIZE-FAIL` logging (Step 4) on failure. Both arms funnel into the same
post-send logic (`WS-SEND-FAIL` handling, and the Step 3 `needs_resync` flush-on-drain
retry) so that behavior is identical regardless of which variant was received — refactored
into a shared `Result<(String, usize), ()>` produced by the match, consumed once below it,
rather than duplicating the send/flush logic in both arms.

**Call sites updated for the `WsClientInfo.tx` type change** (found via `grep -rn
'\.tx\.try_send(\|\.tx\.send('` across the repo, per the task's instruction not to trust
the roadmap's own call-site list as complete):
- `src/websocket.rs`: `reconcile_resync`, `try_send_local`, `broadcast_to_owner`,
  `broadcast_to_all`, `broadcast_to_world_viewers`, `send_to_client`,
  `send_initial_state_and_mark` (listed above).
- `src/main.rs`: `broadcast_activity` (`:5824`), `ws_broadcast` (`:7096`),
  `ws_send_to_client` (`:7150`), `ws_send_initial_state_and_mark` (`:7208`).
- `src/commands.rs`: `spawn_remote_ping_check`'s `PingCheck` fan-out (`:1199`).
- `src/daemon.rs`: two `#[cfg(test)]` `register_client` helpers
  (`change_password_tests::register_client` `:3605-3630`,
  `resume_owner_scoping_tests::register_client` `:3767-3786`) and
  `resume_owner_scoping_tests::drain_scrollback_replies` (`:3820-3830`) — all test-only,
  updated to construct/drain `mpsc::channel::<Outbound>` instead of `<WsMessage>`, with
  `drain_scrollback_replies` and the `PasswordChanged` assertion in
  `change_password_then_login_with_new_password_succeeds` (`:3667-3675`) unwrapping
  `Outbound::Message` before matching on the inner `WsMessage` (both are single-recipient
  sends, so always arrive as `Message`, never `Shared`).
- `src/tests.rs`: three tests construct a `WsClientInfo` directly with their own test
  channel — `test_resume_replay_on_reconnect_sends_exact_gap_no_duplicate` (`:5009`),
  `test_empty_resume_sends_no_scrollback_replay` (`:5078`), and
  `test_channel_full_sends_resync_required_once` (`:5131`, notable because this one drives
  real `ServerData` broadcasts via `server.broadcast_to_all()` — so its channel now
  actually receives a mix of `Outbound::Shared` `ServerData` items and `Outbound::Message`
  `ResyncRequired` items; its drain loop unwraps only `Message` and lets `Shared` items
  pass through unmatched, since it's only counting `ResyncRequired` occurrences). All three
  updated to receive `Outbound` and unwrap `Message` before matching on `WsMessage`
  variants.
- Confirmed out of scope and left untouched: `src/main.rs`'s `gui_tx`/`ws_client_tx`
  (`mpsc::UnboundedSender<WsMessage>`, a *different*, separate in-process channel to the
  embedded GUI — not `WsClientInfo.tx`) and `src/remote_client.rs`'s `ws_tx` (the
  remote-console *client's* outbound channel to its own event loop, also not
  `WsClientInfo.tx`) — grepped and confirmed neither touches the type that changed.

**Wire format:** byte-for-byte unchanged — `Outbound::Shared`'s `json.to_string()` is the
exact same `String` `serde_json::to_string` would have produced per-client before this
step (same input `WsMessage`, same serializer), just computed once and reused rather than
recomputed identically per recipient. Confirmed by code inspection (the `Shared` arm's
`WsRawMessage::Text(json.to_string())` call is structurally identical to the pre-Step-8
`WsRawMessage::Text(json)` call, modulo the `Arc<str>` source) and by the full test suite
passing unchanged (a wire-format regression would have broken message parsing in the
resume/scrollback/multiuser-owner-scoping tests, all of which exercise real serialized
round-trips).

**Ordering:** `Shared` and `Message` items share one `mpsc::channel<Outbound>` per client,
so FIFO delivery order between a broadcast and a per-client send to the same client is
unaffected by the item type change — confirmed both by inspection (no separate queues were
introduced) and by `test_channel_full_sends_resync_required_once`, which already asserts a
specific interleaving of `ServerData` broadcasts (`Shared`) and a `ResyncRequired`
(`Message`) on the same channel.

`cargo build --target x86_64-unknown-linux-musl --no-default-features --features
rustls-backend,ssh-transport` clean (only the pre-existing russh future-incompat notice; one
transient `private_interfaces` warning on `WsClientInfo.tx` was hit and fixed during this
step by narrowing the field to `pub(crate)`, not left in the final diff). `cargo test` (same
flags) 663/663 passing — unchanged from the Step 7 baseline, as expected for a pure
serialization-path change with no new tests required.

Step 10 complete as of 2026-07-31: `ServerData`'s two remaining defaultable boolean fields
now skip serialization on their common-case value, matching `flush`/`gagged`.

**`src/websocket.rs:293`** — the `ServerData` variant declaration:
- `from_server`: gained `skip_serializing_if = "is_true"` alongside its existing
  `default = "default_true"`. New helper `fn is_true(v: &bool) -> bool { *v }`
  (`src/websocket.rs:210`), added next to the pre-existing `is_false` (`:209`) in the same
  style — no other helper needed.
- `marked_new`: gained `skip_serializing_if = "is_false"` (reused the existing `is_false`
  helper `flush`/`gagged` already use — no new helper).
- `world_index`, `data`, `is_viewed`, `seq`, `ts` were **not** touched, per the task's
  explicit scope. `seq`/`ts` in particular carry load-bearing information for the Steps 1-6
  resume/replay contract (an omitted `seq` would be ambiguous between "seq 0, a real
  client-generated/pending-release marker" and "field omitted, defaults to 0" — the two
  cases must stay distinguishable on the wire) and were left alone as instructed.

**Call-site ratio (grepped `from_server:`/`marked_new:` constructions across `src/main.rs`
and `src/daemon.rs`, excluding the struct/field declarations themselves):**
- `from_server`: 10 call sites set the literal `true` (2 in daemon.rs, 8 in main.rs) vs. 59
  set the literal `false` (27 daemon.rs, 32 main.rs) — literal-`false` outnumbers literal-`true`
  at the source-code level. However, the ~19 remaining call sites don't use a literal at all —
  they forward a **dynamic** per-line value (`line.from_server`, `tl.from_server`,
  `batch_from_server`, `s.from_server`) sourced from the underlying `TextLine`/`OutputLine`,
  which itself defaults `from_server: true` (`src/main.rs:2540`,`:2576`). Inspecting those
  sites (`main.rs:5915`, `6016`, `6100`, `6585`, `6654`, `7729`, `7821`, `10724`, etc.) shows
  they're exactly the hot path: real MUD output streamed to WS clients on every line/batch of
  incoming game text. The 59 literal-`false` sites are synthesized system/status/error
  messages (`"Disconnected."`, `"World '...' not found."`, help text, ban-list output,
  version string, etc.) — each fires once per rare user action or admin event, not per line
  of MUD traffic. So while `false` wins the literal-call-site count, `true` is the
  overwhelming case **by message volume** (continuous game output vs. occasional system
  messages), which is the actual justification `skip_serializing_if` needs — confirmed by
  reading the hot-path call sites rather than trusting the raw grep count.
- `marked_new`: 63 call sites set the literal `false` (28 daemon.rs, 35 main.rs), **0** set a
  literal `true` anywhere in either file; the remaining ~18 forward a dynamic
  `line.marked_new`/`tl.marked_new`/`batch_marked_new`/`s.marked_new`, itself only ever `true`
  in the narrow case of a pending line released while the user is scrolled back (rare). `false`
  is unambiguously the common case both by literal count and by volume.

**Wire compatibility / round-trip test:** added two tests to `src/tests.rs` (appended after
`test_version_string_includes_platform_tag`):
- `test_server_data_common_case_omits_from_server_and_marked_new` — serializes a `ServerData`
  with `from_server: true, marked_new: false` (the now-both-omitted case, alongside the
  already-omitted `flush: false, gagged: false`), asserts the resulting JSON string contains
  neither `"from_server"` nor `"marked_new"` (nor `"flush"`/`"gagged"`), then
  `serde_json::from_str`s that trimmed JSON back and asserts every field — including the four
  omitted booleans — matches the original struct exactly (defaults reconstruct correctly via
  the existing `#[serde(default = "default_true")]`/`#[serde(default)]` attributes).
- `test_server_data_non_default_from_server_and_marked_new_are_serialized` — companion
  negative case: `from_server: false, marked_new: true` round-trips with both fields
  explicitly present on the wire (`"from_server":false`, `"marked_new":true`), confirming the
  `skip_serializing_if` is conditional on the default rather than a blanket omission.

**`src/web/app.js` check (grepped `.from_server`/`.marked_new`, ~15 sites):** no strict
`=== true`/`=== false` comparisons against a raw incoming `msg` object were found. The two
sites that read the wire message directly already use truthiness-safe patterns that treat a
missing key correctly: `const isFromServer = msg.from_server !== false;` (`app.js:2663` —
`undefined !== false` is `true`, matching the field's default) and
`msg.marked_new || false` (`app.js:2743`, and `lineObj`'s construction at `:2712`; `undefined
|| false` is `false`, also matching the field's default). The remaining sites
(`app.js:2299`, `5570`, `8896`) read `.from_server`/`.marked_new` off already-normalized local
line objects (constructed via the `lineObj` literal at `:2712`, which always sets both fields
explicitly), not off the raw wire message, so they were never at risk regardless of trimming.
No `app.js` changes were needed. `node --check src/web/app.js` passes (using the Node
20.18.1 build already staged in the session scratchpad from a prior step).

`cargo build --target x86_64-unknown-linux-musl --no-default-features --features
rustls-backend,ssh-transport` clean (only the pre-existing russh future-incompat notice, no
new warnings). `cargo test` (same flags) 665/665 passing (663 baseline + 2 new).

Step 11 complete as of 2026-07-31: `websockets.readme` was rewritten from scratch against a
fresh read of `src/websocket.rs` (plus targeted checks of `src/main.rs`, `src/daemon.rs`,
`src/http.rs`, `src/web/app.js`), replacing a version that predated Steps 1-10 entirely.

**Scope of the rewrite:** transport/framing (single-port first-byte multiplex summarized
with a pointer to `CLAUDE.md`/`SECURITY-ROADMAP.md` for the full design, JSON-text-frame
format, size caps); the real challenge-response auth flow (`ServerHello.challenge` ->
`AuthRequest` with `challenge_response`/`password_hash = SHA256(SHA256(password) +
challenge)` -> `AuthResponse`), replacing the old doc's plain `password_hash` description;
the ordering/reliability contract as a coherent design — `seq` as the delivery contract,
per-world last-contiguous-seq tracking, `AuthRequest.resume` and `PongCheck.acked`,
the bounded `WS_CLIENT_CHANNEL_CAPACITY` channel and `ResyncRequired` on overflow,
`RequestScrollback`/`ScrollbackLines` as the gap-fill mechanism for both reconnect and live
resync, and an explicit note that this replaced the old capped client-side gap-guessing
heuristic (`_seqGaps` et al., still present in `app.js` as a defense-in-depth net, not as the
authoritative recovery path); `ServerData`'s exact current field list including the
`skip_serializing_if`-trimmed fields (`from_server`/`marked_new`/`flush`/`gagged`) and why
`seq`/`ts` are never trimmed; a message catalog grouped by category (handshake/auth, state
bootstrap, live output, client commands, settings, scrollback/resume, diagnostics, keepalive,
cert management, import/export) with representative examples rather than all ~114 variants,
pointing at `WsMessage` in `src/websocket.rs` as the field-level source of truth; a multiuser
note on owner-scoped resume/scrollback (Step 6a); and a keepalive/reconnect summary with the
current constants. Caught and corrected one thing the task prompt didn't flag: the old file's
`RequestState` documentation was still accurate (the variant is alive and still used by
`app.js` on visibility-change wake) — an early draft nearly cut it, since a stale grep pass
had missed its single-line `RequestState,` enum declaration; the second, targeted grep
(`grep -n "RequestState\b"`) caught this before publishing, and the final doc keeps it,
correctly distinguished from `RequestWorldState`/`WorldStateResponse` (per-world) and from
the `resume`/`ResyncRequired` contract (which `RequestState` does not itself drive).

**Five facts verified against a fresh grep of source immediately before finalizing:**
1. `WS_CLIENT_CHANNEL_CAPACITY: usize = 256` — `src/websocket.rs`, the constant's own
   declaration and doc comment.
2. `ServerData`'s exact field list and attributes, including `from_server`'s
   `#[serde(default = "default_true", skip_serializing_if = "is_true")]` and `marked_new`'s
   `#[serde(default, skip_serializing_if = "is_false")]` — `src/websocket.rs:294`, the struct
   literal itself (not a summary of it).
3. `KNOCK_MAGIC: [u8; 4] = [0xC7, 0x4C, 0x41, 0x59]` (`0xC7` + ASCII `"LAY"`) — `src/http.rs`.
4. `WS_KEEPALIVE_INTERVAL_SECS = 60`, `WS_PONG_TIMEOUT_SECS = 20`, `WS_AUTH_TIMEOUT_SECS = 30`
   — `src/websocket.rs`, the three constants' declarations.
5. `AuthRequest.resume: Vec<(usize, u64)>` and `PongCheck.acked: Vec<(usize, u64)>` field
   names/types, and `ResyncRequired { world_index: usize, from_seq: u64 }` — `src/websocket.rs`,
   the `WsMessage` enum's variant declarations directly (not the roadmap's own summary of
   them, per the task's instruction not to trust a prompt/roadmap summary over the source).

Also spot-checked: `hash_password()`/`hash_with_challenge()` implementations
(`src/websocket.rs`) to confirm the exact challenge-response formula documented is
`SHA256(SHA256(password) + challenge)`; `max_message_size`/`max_frame_size` (2 MiB / 256 KiB)
at the `WebSocketConfig` construction site; `RequestState`'s continued existence and its
handler (`src/main.rs`, `src/daemon.rs`) plus its two `app.js` call sites; and
`owner_filtered_pairs`/`handle_request_scrollback_owned` (`src/daemon.rs`/`src/main.rs`) for
the multiuser section. Docs-only change — no Rust/JS/Java files modified, no build or test
run required or performed.


All required steps (1-11, plus the 6a security fix) complete as of 2026-07-31. Step 12 measured and deliberately not pursued (see its entry above). Nothing in this roadmap has been committed to git yet.

## Phase B — seq drift, stuck scrollback indicator, and unreachable deep scrollback

Design record for a second incident, distinct from Phase A above but living in the same
delivery contract: a user reported (1) the last ~9 lines of one world's output duplicated on
Android after several app backgrounds/resumes, surviving a manual resync; (2) the Android
scrollback-download indicator sticking at 90% after resuming the app, clearing only on a
resync; (3) `clay --gui=remote` unable to scroll back past ~500 lines, with raising the
Remote Lines setting from 1000 to 5000 and reconnecting having no visible effect. Full plan
file: `on-the-android-app-calm-curry` (session-local; superseded by this section as the
permanent record). Scrollback depth policy set by the user for Phase B: clients that fetch a
screenful on demand (the `--console` remote TUI) should reach as far back as the master
instance holds in memory; clients that download their history up front (`--gui`, Android)
stay bounded by the Remote Lines (`remote_initial_lines`) setting.

**Root cause of (1):** the client (`app.js`) derived each line's seq from
`msg.seq + appendedLineCount` — a count of lines the client *kept* after filtering (ANSI-only
lines, idler markers, grep mode) — rather than the server's true per-line seq assignment.
Every filtered line permanently drifted the client's `_max_seq` one below the server's true
value. Once drifted, the next real batch's seq exceeded `_max_seq + 1`, recording a phantom
gap; `lastContiguousSeq()` then reported a stale boundary to `AuthRequest.resume`, and the
server's resume replay faithfully re-sent exactly the drifted-away tail as "new" — appended
as duplicates. `D ≈ 9` from a color test's ANSI-only lines matched the report precisely.
Four independent duplicate sources were found and fixed in the same broadcast path (Steps 1,
2, 3, 6 below) — not all of them required the drift mechanism above.

**Root cause of (2):** `ScrollbackLines` carried no way to correlate a reply to the request
that caused it; the client routed purely on `world._gapFillPending`. A `before_seq` backfill
reply landing while that flag happened to be (wrongly) true was misrouted into the gap-fill
splice branch, which silently dropped every line (they're older than `_max_seq`) and `break`d
before ever advancing the backfill pump — `backfillInProgress` stayed true forever, and the
percentage (floored to multiples of 10, hidden only at 100%) froze at whatever it last
computed. `_gapFillPending` got stuck specifically because Android's background-wake
`RequestState` resync set it from a heuristic (`priorWorld && lastContiguousSeq(priorWorld) >
0`) that stayed true even though `RequestState` sends no resume list and triggers no server
replay to ever clear it.

**Root cause of (3):** two independent client-side caps, both in `app.js`, neither
server-side: `renderOutput()` hard-capped the DOM at the newest 500 lines with a stale comment
claiming PageUp re-rendered to reveal more (it only ever moved `scrollTop`); and
`backfillTotalTarget` was computed once, at connect time, so raising Remote Lines without
reconnecting fetched nothing new. A third, independent bug (`_oldest_seq` recomputed without
checking `_has_real_seq`, so one ephemeral `seq: 0` line poisoned it to a near-zero value) had
already been silently capping *fetched* history in some sessions before the render cap was
even reached.

### Progress checklist

Same resume protocol as Phase A: find the first unchecked box, verify against the tree
whether it's actually done, continue from there. One step at a time. Verify
`cargo build --target x86_64-unknown-linux-musl --no-default-features --features
rustls-backend,ssh-transport` and `cargo test` after every Rust step; `node --check
src/web/app.js` plus a standalone Node harness (no browser in this sandbox — same
constraint Phase A hit) after every `app.js` step.

**Phase 0 — server-side duplicate elimination (no wire change):**
- [x] **Step 1** — `App::broadcast_released_lines` (was five separately hand-rolled copies):
      `World::release_pending`/`release_all_pending` now return the drained lines so the
      broadcast can never diverge from what was actually released.
      `release_pending_screenful` previously sized its broadcast with `visual_line_count`
      (a full-width estimate) while `release_pending` decided what to actually drain with
      `nli_visual_rows` (NLI-narrowed, real wrapping) — surplus lines were broadcast but
      stayed pending, re-broadcast on the next release. Test:
      `test_release_pending_screenful_broadcasts_exactly_what_it_releases`.
      **Found, out of scope:** multiuser's `ReleasePending`/`SelectiveFlush` handlers
      (`daemon.rs`) are separate hand-rolled implementations (need `broadcast_to_owner`, not
      the single-user broadcast primitives) that never broadcast the released text at all —
      only `PendingReleased`/`PendingLinesUpdate` metadata. Independent bug, deferred.
- [x] **Step 2** — `App::ws_broadcast` gained the `received_initial_state` gate its three
      `WebSocketServer` siblings already had (whitelisted clients can be `authenticated: true`
      before the app loop ever processes their connection). Test:
      `test_ws_broadcast_skips_client_without_initial_state`.
- [x] **Step 3** — `build_multiuser_initial_state` had no cap at all (unlike single-user's
      `remote_initial_lines`-driven budget) and sent `pending_lines_ts` (single-user
      deliberately sends none). Extracted `App::build_initial_output_lines`, shared by both.
      Also removed `UserConnection.pending_lines` (dead code — never written anywhere).
      Tests: `test_multiuser_initial_state_caps_lines_and_omits_pending`,
      `test_multiuser_initial_state_empty_for_user_with_no_connection`.

**Phase 1 — make `seq` authoritative on the wire:**
- [x] **Step 4** — Established the invariant **`World::output_lines` is always sorted by
      `seq`** (now documented in CLAUDE.md's Key Design Patterns). A gagged line while paused
      with more-mode on used to jump straight into `output_lines` with a fresh (higher) seq
      even while lower-seq pending lines sat unreleased — now routed into `pending_lines`
      instead (`process_server_data`'s `hold_gagged_in_pending`) when that's the case;
      `release_pending`'s budget loop skips gagged lines' visual-row cost (always released,
      never counted against the row budget). Test:
      `test_gagged_line_while_paused_does_not_jump_ahead_of_pending`.
- [x] **Step 5** — Added `ServerData.end_seq: Option<u64>` (never trimmed to a bare `u64` —
      seq 0 is real, same reasoning as `seq` itself). `App::broadcast_output_range` computes
      `seq`/`end_seq` from the actual `output_lines` slice; used by `process_server_data` and
      `emit_client_lines`. ~66 other construction sites mechanically get `end_seq: None`
      (confirmed ephemeral, `seq: 0` there is correct). Tests:
      `test_server_data_end_seq_covers_filtered_lines`, plus `end_seq` assertions folded into
      the existing from_server/marked_new trim tests.
- [x] **Step 6** — Release-path broadcasts (`broadcast_released_lines`) switched from the
      `seq: 0` "bypass dedup" sentinel to the real per-batch span, safe now that Step 4
      guarantees `output_lines` stays seq-sorted through a pause. Batch grouping extended to
      `(marked_new, from_server, seq contiguity)` — `selective_flush`'s kept-lines subset is
      not seq-contiguous, so a seq gap now also forces a new batch. Tests:
      `test_released_pending_carries_real_seqs_no_false_duplicate`,
      `test_selective_flush_emits_contiguous_seq_runs`.
- [x] **Step 7** — `App::add_output`/`add_output_to_world` routed through
      `broadcast_output_range` instead of hand-rolling a `seq: 0` broadcast of the raw input
      text. Fixed two latent bugs this exposed: neither function previously checked whether
      its text landed in `output_lines` vs. `pending_lines` while paused (could double-
      broadcast); `add_output_to_world` hardcoded `marked_new: false` even for a background
      world, inconsistent with what `World::add_output` itself stores on the line. Tests:
      `test_add_output_broadcasts_real_seq`, `test_add_output_to_world_broadcasts_real_seq`.

**Phase 2 — client seq correctness (`src/web/app.js`):**
- [x] **Step 8** — `case 'ServerData'` derives `lineSeq = msg.seq + rawIdx` (the line's
      position in the full pre-filter split) instead of `msg.seq + appendedLineCount` — the
      seq-drift root cause. `hasRealSeq` widened to recognize a real seq-0 first line via
      `end_seq`. `_max_seq` now advances from `end_seq` (or the full pre-filter batch length)
      whenever the batch has a real seq, not gated on `appendedLineCount > 0` — an
      all-filtered batch (a lone idler line) must still advance it. Verified via a standalone
      Node harness (T8a-T8d); confirmed non-vacuous by reverting to the old formulas and
      rerunning (5/11 assertions fail, reproducing the exact phantom-gap pattern).
- [x] **Step 9** — `world._seqGaps` now carried across `InitialState` (`priorWorld` branch)
      and persisted/restored alongside the IndexedDB cache (`cachedWorld` branch;
      `scheduleWorldCacheSave` gained a `seqGaps` field) — previously silently dropped every
      reconnect, hiding real holes from `lastContiguousSeq()`. The `_oldest_seq`/`_max_seq`
      recompute loops guard on `line._has_real_seq !== false` (deliberately not a truthy
      check — server-provided lines from `output_lines_ts`/cache never set the field at all
      but always carry a real seq; only the live handler's fake-index fallback ever sets
      `false` explicitly). Harness (T9a-T9c) confirmed non-vacuous twice: once for the
      carry-over, once for the `!== false` vs. truthy distinction (a truthy-only guard broke
      `_oldest_seq`/`_max_seq` on every ordinary fresh connect).
- [x] **Step 10** — `dedupBySeq()`: one-time, seq-keyed (exact, not a text heuristic) dedup
      pass applied on hydrate (both `priorWorld` and `cachedWorld` branches), cleaning up any
      duplicate a client already picked up before Steps 8-9 shipped, without a cache DB
      version bump (would silently discard everyone's cached scrollback — same precedent as
      the existing `CLIENT_LINE_PREFIX` migration). Harness (T10/T10b/T10c) reproduces the
      exact bug-report shape (20 real lines + a duplicated 9-line tail) and confirms correct
      dedup, no false-positive on genuinely repeated text, and no interference with
      no-real-seq lines.

**Phase 3 — request/response correlation (the stuck-90% indicator):**
- [x] **Step 11** — `RequestScrollback`/`ScrollbackLines` gained `request_id: Option<u64>`
      (`#[serde(default)]`); `Some(0)` reserved for the server-initiated unprompted resume
      replay, wired at all three server dispatch paths (master-WS, `-D`, multiuser). Also
      closed a divergence the three-path audit turned up: multiuser's `RequestState` handler
      was missing the `ActivityUpdate`/`PausedState` sends the single-user handler always
      made — added, computed per-user from `user_connections` (not
      `App::activity_count()`/a shared paused flag, which would leak across the multiuser
      boundary). Test: `test_request_scrollback_echoes_request_id`; extended
      `test_resume_replay_on_reconnect_sends_exact_gap_no_duplicate` and the multiuser
      owner-scoping negative test (a real `request_id` attached to the leak attempt must not
      bypass the owner check).
- [x] **Step 12** — `app.js`'s `ScrollbackLines` handler now resolves which outstanding
      request a reply answers via `request_id` (a `pendingScrollbackRequests` Map +
      `registerScrollbackRequest`/`resolveScrollbackRequest`, with a
      `SCROLLBACK_REQUEST_TIMEOUT_MS = 15000` watchdog) instead of routing purely on
      `world._gapFillPending`. The bare `break` that skipped the pump-advance tail for
      gap-fill replies is gone — both branches converge on
      `updateScrollbackProgress()` + `backfillNextWorld()`. `_resumedFromServer` is now
      derived from a `resumeSentThisConnection` map (populated by a new
      `buildResumeAckListForAuthRequest()`, cleared on socket close) recording exactly what
      was sent in `AuthRequest.resume` this connection, instead of a heuristic that stayed
      true across a `RequestState` resync (no resume list is ever sent for that path) —
      the actual stuck-flag mechanism. Both backfill queue builders now exclude worlds with
      an outstanding gap-fill. Harness (T12a-T12e, 17 assertions) covers the routing decision,
      the reserved id, the `_resumedFromServer` fix (with an old-heuristic comparison), the
      watchdog, and watchdog-cancellation-on-resolve.
- [x] **Step 12b** — Same correlation for the `--console` remote client (Rust): a
      `ScrollbackRequestKind` enum + `App::register_scrollback_request`/
      `pending_scrollback_requests` (mirroring `app.js`'s Map), threaded through
      `backfill_next`'s tuple and the `scroll_page_up` scroll-triggered request in
      `remote_client.rs`. `App::handle_remote_ws_message`'s `ScrollbackLines` handler
      resolves `is_gap_fill` from `request_id` first, falling back to the legacy
      `World::pending_gap`-presence heuristic. No watchdog needed here (unlike `app.js`) —
      every request site already has a correctly-behaving `pending_gap` fallback, so a
      stuck-forever state was never possible for this client. Test:
      `test_console_client_scroll_backfill_reply_not_treated_as_gap_fill`; confirmed
      non-vacuous by temporarily reverting to the legacy-only heuristic (fails exactly as
      predicted — all 11 older-history lines dropped instead of prepended).

**Phase 4 — reachable scrollback in `--gui`/Android:**
- [x] **Step 13** — `backfillTotalTarget` recomputes live when Remote Lines changes
      mid-session (previously computed once, at connect, inside `startBackfill()` — the
      literal "raised it, reconnected, nothing changed" bug report). Only kicks
      `startBackfillPhase2()` when a backfill isn't already in progress (one already running
      picks up the new target on its own next check); `_backfill_exhausted` is cleared on
      every world since a prior verdict may have been an artifact of Step 9's now-fixed
      `_oldest_seq` poisoning. Harness (T13a-T13d).
- [x] **Step 14** — The DOM render window (`renderOutput()`) is now expandable:
      `RENDER_WINDOW_INITIAL/STEP = 500`, `RENDER_WINDOW_MAX = 5000` (matching
      `remote_initial_lines`'s own upper clamp), grown on scroll-toward-top
      (`scheduleRenderWindowCheck()`, rAF-throttled) via a new `renderOutput({preserveScroll})`
      mode that corrects `scrollTop` by the height delta instead of jumping to the bottom.
      Resets to the initial window on reaching the bottom and on world switch, so per-world
      DOM cost doesn't stay elevated indefinitely. The 5000 ceiling is a deliberate,
      tested-safe performance bound for WebKitGTK/Android WebView — lower it, don't remove
      the mechanism, if a low-end device struggles. Harness (T14a-T14f).
      **Noted, not fixed:** live incoming lines while parked at the bottom are still appended
      via unbounded `insertAdjacentHTML`, independent of the render window (which only
      governs a full rebuild) — pre-existing, out of this phase's scope.
      **Deliberately not built:** unbounded scroll-triggered history *fetching* for
      `--gui`/Android — per the scrollback policy above, those clients stay bounded by
      Remote Lines; the render window just makes what they already hold reachable. The
      `--console` remote client already had on-demand fetch (Step 12b fixed its one
      correctness bug); nothing else was needed there.
- [x] **Step 15** — Docs: this section; `end_seq`/`request_id`/the reserved `request_id: 0`
      documented in `websockets.readme`; the `output_lines`-is-seq-sorted invariant added to
      CLAUDE.md's Key Design Patterns; the stale `app.js` comment claiming Phase A's Step 7
      (Android bridge ordering) "has NOT shipped yet" corrected — it shipped in Phase A.

### Deliberately out of scope for Phase B

- **SQLite archive (`scrollback.db`) for remote clients.** `handle_request_scrollback` reads
  only `world.output_lines`; the archive is read solely by the local TUI's
  `try_load_archive_lines`, and `-D`/multiuser never call `init_scrollback()` at all. Making
  it remotely reachable needs an archive-backed branch in `handle_request_scrollback`,
  `init_scrollback()` in both daemon entry points, seq assignment for archive lines, and
  `from_archive` plumbed through `ScrollbackLines` — a separate project.
- **Changing `World::next_seq` to start at 1** (reserving seq 0 entirely) instead of Step 8's
  client-side `hasRealSeq` widening. Cleaner in the abstract, but touches three `next_seq`
  recompute sites and invalidates every existing client's cached seq-0 lines. Revisit only if
  the seq-0 ambiguity bites again in practice.
- **Multiuser pause/pending support**, found broadly incomplete during Step 1 (no live
  `ServerData` broadcast for released pending lines) and Step 4 (no gagged-line/pause concept
  in `AppEvent::MultiuserServerData` at all). Real gaps, but outside this incident's three
  reported symptoms (all single-user: `--gui`, Android, `--console`) — flagged for a future,
  separately-scoped multiuser pass.

All 15 steps (12b included) complete. `cargo build`/`cargo test` green throughout (677/677 at
completion, up from the 665/665 baseline this phase started from). Shipped as `99fe3dc`
("Fix Android duplicate output, stuck scrollback indicator, and 500-line scrollback wall").

---

# Phase C — end the seq-watermark bug class

Phase B closed one poisoning path. Three more turned up within days, each the same shape and
each fixed the same way:

| Commit | Symptom | Poisoning path |
|---|---|---|
| `03ab4f9` | remote output freezes after a server restart | cached buffer's `_max_seq` outlived a `next_seq` reset to 0 |
| `bbb8837` | one world silently stops updating on Android | `handle_disconnected` broadcast a real seq for a line `push_line_respecting_pending` had deferred into `pending_lines` |
| `6c846db` | periodic Android output loss | six further paths where a seq reached a client before its line was ordered in `output_lines` |

The pattern is not a run of unrelated bugs. Every one of them was survivable only because the
protocol had two structural weaknesses, and neither was addressed by fixing the individual
paths:

1. **The client's dedup mark was a one-way ratchet.** `world._max_seq` only moved forward. A
   `ServerData` batch with `seq <= _max_seq` was dropped *whole* unless it happened to overlap
   a *recorded* `_seqGaps` entry — and `_seqGaps` could only ever record "a batch skipped ahead
   of the expected next seq". Neither structure could represent "lines I was never sent", so a
   mark that got ahead of the buffer ate that world's output permanently. Worse, the repair was
   re-dropped by the same test: `ScrollbackLines`' gap-fill branch required a line to be newer
   than `_max_seq` or to overlap a recorded gap, and `requestGapFill()` asked from `_max_seq`
   rather than the contiguous frontier, so it never even requested the damaged range.
2. **Nothing on the server detected loss.** `ResyncRequired` fired *only* on outbound-channel
   backpressure (`reconcile_resync`). `WsClientInfo::acked_seq` was tracked per client per
   world and refreshed every 30 s from `PongCheck.acked`, but never compared against anything.
   `ReportSeqMismatch`/`ReportDuplicate`/`ReportOutOfOrder` were logged and ignored. All real
   gap *detection* lived on the client, resting on the very value the bugs corrupted.

## What changed

**Client — `_seenRanges` replaces `_max_seq` + `_seqGaps` (`src/web/app.js`).** A sorted,
coalesced, non-overlapping array of inclusive seq ranges recording every seq the server has
actually delivered. `_max_seq` becomes `last().end`, the resume/ack boundary becomes
`ranges[0].end` (`contiguousFrontier`), and dedup becomes exact set membership (`hasSeenSeq` /
`hasSeenRange`, both O(log n)). Holes are simply the spaces between ranges, so a hole exists
whether or not anything noticed it opening. Ranges are recorded for the **whole delivered batch
span**, never per surviving line, so client-side display filtering (ANSI-only lines, idler
markers, grep) can't punch phantom holes — the property Phase B's `rawIdx` fix established.
Bounded by coalescing (normally exactly one range); `MAX_SEEN_SEQ_RANGES = 512` is a backstop
that merges the oldest hole shut, which only ever moves the frontier forward.

Carried across `InitialState` (both the in-memory reconnect and IndexedDB paths) and persisted
in the cache as `seenRanges`. Legacy `{maxSeq, seqGaps}` entries convert exactly via
`seenRangesFromLegacyGaps()` — no IndexedDB version bump, which would discard every user's
cached scrollback (the `CLIENT_LINE_PREFIX` precedent).

Consequences at the three decision points that used to drop data:
- `ServerData`: a batch is a duplicate only if *every* seq it spans is already seen; otherwise
  it is accepted and only the individually-seen lines are skipped.
- `ScrollbackLines` gap-fill: accepts any line whose exact seq is unseen. **This is what lets a
  resync repair actually land** — previously those lines went into `droppedCount`.
- `requestGapFill()`: asks from `contiguousFrontier(world)`, not `_max_seq`.

Deliberately *excluded* from `_seenRanges`: `before_seq` backfill replies. That's the
downward-growing deep-history region; folding a not-yet-adjacent older chunk in would drag
`ranges[0]` — and therefore the frontier — backwards. Pre-Phase-C code excluded backfill from
`_max_seq`/`_seqGaps` for the same reason.

**Server — periodic ack audit (`App::audit_client_acks`, `WebSocketServer::evaluate_ack_audit`).**
The detector the protocol was missing. `World::deliverable_high_seq()` reports the highest seq a
world owes a caught-up client — the greatest seq in `output_lines` strictly below
`pending_floor_seq()`, so a paused world's withheld backlog doesn't make every client look
behind. On each `PongCheck.acked` (a ~30 s per-client cadence that already carries the data, so
no new timer in any of the three event loops) each client's ack is compared against it, and a
world behind *at the same position across two consecutive audits* gets a
`ResyncRequired { from_seq: acked }`. The client repairs through the existing
`ResyncRequired → RequestScrollback{after_seq} → ScrollbackLines` path that web/Android
(`app.js`) and `--console` (`handle_remote_ws_message`) both already implement.

`AckAuditOutcome` distinguishes `CaughtUp` / `Lagging` / `Fired` / `StillStalled` / `Recovered`.
Guards, each for a distinct false-positive:
- Two-audit stall requirement — ordinary in-flight lag never fires.
- Never-acked and explicitly-zero-acked worlds are exempt — that's the "`build_initial_state`'s
  aggregate line budget ran out before this world" case, which `startBackfill()`'s phase-1 queue
  already covers; firing from 0 would pull the whole in-memory ring per world per connect.
- One fire per stall point (`audit_fired_at`) — a genuinely undeliverable seq costs one message,
  not one per keepalive forever.
- `next_seq == 0` worlds skipped. **Not wired into multiuser at all**: `daemon.rs` emits
  `seq: 0, end_seq: None` universally, so an ack and `deliverable_high_seq` there aren't in the
  same units. Multiuser's missing seq support remains the separate item noted in Phase B.

**Switch-time verification.** `WsMessage::WorldStateResponse` gains `deliverable_high_seq`
(`#[serde(default)]`, so older clients are unaffected). Clients already send `RequestWorldState`
on every world switch, which makes the reply the cheapest place to verify a world the user is
about to *look at* — previously the one moment nothing checked, since `SwitchWorld` sends no
content and the client renders straight from its local buffer. Both `app.js` and the `--console`
client request a gap-fill on the spot when their own frontier is behind.

**Diagnostics.** `SEQ-AUDIT` events in `~/.clay/remote.log` for every transition
(`Fired`/`StillStalled`/`Recovered`); the two steady states are logged only under debug mode, or
N worlds × M clients would write a line each per keepalive and drown the log this exists to
serve. `/dump` gains a `SEQ RECONCILIATION` table of acked / prev-audit / deliverable / next_seq
/ pending / behind per client per world — the live picture on demand.

## Status

720/720 `cargo test` (713 baseline + 7 new), clean `cargo clippy` and musl build. The range
algebra was additionally exercised outside the test suite (no JS runtime is available in the
development sandbox — see "Verification gaps" below). The four new server-side behaviours were
mutation-checked: removing the stall requirement, ignoring the pending floor, removing the
re-fire suppression, and auditing zero-ack worlds each fail a specific test.

## Verification gaps

- **No JS runtime in the sandbox**, so `app.js` was not executed. The `_seenRanges` helpers were
  transliterated line-for-line into Python and fuzzed against a brute-force set model
  (membership, coalescing, order-independence, the overflow cap's forward-only frontier, legacy
  conversion, and the poisoned-mark case); the surrounding handler edits are reviewed, not run.
  A browser/Node pass over the `ServerData` and `ScrollbackLines` handlers is still owed.
- **No on-device run.** The end-to-end claim — Android backgrounded past the heartbeat, resumed,
  every world compared against the TUI, `SEQ-AUDIT` lines and the `/dump` table inspected — has
  not been performed.

---

# Phase D — per-line ▶ ownership

## The report

Main instance viewing world A, a second instance (`clay --console=<host>`) viewing world B.
Text arriving on B was marked ▶ **on the instance that was watching B**.

## Two defects

**1. The remote console compared seqs from two different number spaces.** ▶ was decided by
`line.seq >= new_from_seq && line.seq < viewed_from_seq`, with both watermarks mirrored from
the server — but the mirror's live-text handler threw the server's seq away and invented its
own (`main.rs`, the `ServerData` arm), keeping the real one only for dedup. So the
"somebody is viewing, don't mark it" suppression could never take effect on a remote console.
Drift is unbounded: a world that received zero lines in `InitialState` (routine — the
aggregate budget is 500 lines across *all* worlds) starts at 1 while the server is at 50 000.

`app.js` already derived `msg.seq + rawIdx` correctly — Phase B Step 8, applied to the web
client and never mirrored into the Rust one. Fixed in both `ServerData` and
`WorldStateResponse`; the mirror's `output_lines` had been a *mixture* of real and invented
seqs, which also undermined the sorted-by-seq assumption the gap-fill splice relies on.

**2. The ▶ watermark was per-world, shared by every client.** `new_from_seq`/`viewed_from_seq`
were advanced from a global OR (`world_idx == current_world_index ||
ws_client_viewing(world_idx)`) and broadcast to everyone. One shared pair cannot express "new
for you but not for me", so it was wrong in both directions: the console on world 0 suppressed
▶ for a remote parked on world 5, and one client leaving a world called `mark_displayed()`,
advancing the shared floor and **wiping another client's markers**.

## The model

Two fields on `OutputLine` (and `TimestampedLine`):

| Event | Effect |
|---|---|
| Line arrives, world **not** viewed anywhere | `viewed = false`, `display_id = None` |
| Line arrives, world **is** viewed | `viewed = true`, `display_id = None` |
| A client displays it, **and `!viewed`** | `viewed = true`, `display_id = Some(that client)` |
| A client displays it, already `viewed` | nothing — the claim is never stolen |

A client renders ▶ iff `display_id` equals its own id. The `!viewed` one-way latch is the
whole design: it gives first-viewer-wins, makes a line that arrived while somebody was
watching permanently un-new, and makes each client's marker untouchable by anyone else.

This is a deliberate return to per-line state. Per-line `marked_new` was removed in `a23d2c1`
because of cross-instance drift — a client that left a world never told the others to stop
drawing ▶. An owner id is precisely that fix.

- **Claim** (`World::claim_unviewed`, `App::claim_world_for`): on world switch-in, on
  visibility-visible, and on pending release (`broadcast_released_lines`). Sweeps the **whole**
  buffer — unviewed lines are *not* a contiguous tail, since `viewed` flips with whether
  anyone was watching and `[unviewed, viewed, unviewed]` is reachable. An early version broke
  out of a reverse scan on the first viewed line and silently skipped older backlog behind it;
  caught by `test_claim_sweeps_unviewed_lines_behind_a_viewed_one`. For the same reason
  `ClaimedNew` carries an explicit **seq list**, not a range.
- **Release** (`World::release_claims`, `App::release_world_for`): on world switch-away, Ctrl+L,
  visibility-hidden, and disconnect past `WS_VIEWER_GRACE`. Clears only that viewer's markers,
  leaving `viewed` true so nobody re-claims them.
- **Wire**: `NewWatermark` (broadcast) is replaced by `ClaimedNew { world_index, seqs }` and
  `ReleasedNew { world_index }`, both sent to **one** client — a claim only moves a line from
  unowned to owned-by-that-client, so nobody else's rendering changes.
- **Identity**: `AuthRequest.client_uid` (stable, localStorage-backed on web/Android) is hashed
  to the ownership id and reported back as `InitialState.your_display_id`, so a brief transport
  drop keeps a client's markers even though it gets a fresh connection id. Empty uid falls back
  to the connection id (one-shot Rust clients, older peers). `CONSOLE_DISPLAY_ID = u64::MAX` is
  the local TUI's reserved id.
- **Visibility**: `ClientVisibility { visible }`, tracked in its own `ClientViewState::visible`
  field, driven from `app.js`'s `visibilitychange` and
  Android's `MainActivity.onPause`/`onResume`. Backgrounding is **not** a disconnect (the socket
  stays open), so it must be signalled: a hidden client stops counting as a viewer and releases
  its markers, so text arriving meanwhile is unviewed and becomes ▶ on return.
  `WS_VIEWER_GRACE` therefore drops from 5 min to **10s** — it now only has to outlast a
  transport blip, not absorb backgrounding.
- **Follow-up (found while answering a question about the status bar, fixed after v1.5.16):**
  visibility originally *reused* `ClientViewState::paused` as its "doesn't count as a viewer"
  flag. Both do keep a client out of `ws_client_viewing`, but they are not interchangeable:
  `paused` means only `/remote --pause`, and `handle_request_state` reports it back to the
  client as the user-visible `PAUSED` badge. So a resync landing while an Android client was
  backgrounded — the 2-missed-heartbeat `triggerResync()` path runs in the background — would
  light `PAUSED` as though an operator had paused the session, and
  `handle_client_visibility` sent no `PausedState` on return to clear it. Split into a
  separate `visible` field, carried over (not reset) at every `ClientViewState` reinsertion
  site so a world switch or view-state update can't silently mark a backgrounded client
  visible again.
- **Persistence**: `viewed` rides in the `[output:N]` section as a `v` flag; `display_id` is
  deliberately not persisted (a reload has no live clients, so markers restore unowned). The
  old save-time watermark widening is gone — a displayed line already carries `viewed: true`.

## Status

726/726 `cargo test` (720 baseline + 6 new, several rewritten), clean `cargo clippy`, clean
musl build. Four mutations each fail a specific test: removing the `!viewed` guard,
reintroducing the contiguous-tail `break`, releasing all markers instead of one viewer's, and
ignoring `is_current` on arrival.

## Verification gaps

- **No JS runtime in the sandbox**, so `app.js` was not executed — the changes there
  (`lineIsNew`, the `ClaimedNew`/`ReleasedNew` handlers, `clientUid`, `sendClientVisibility`)
  are reviewed and brace-balance-checked only.
- **No on-device / two-instance run.** The scenario in the report — master TUI on A, a second
  instance on B, plus the "client 2 switching in must not disturb client 1's markers" case and
  the Android background/foreground cycle — has not been exercised against real processes.

---

# Phase E — three independent "missing output at the bottom" causes

Reported: the Android client attached to a remote Clay is missing lines at the **bottom**
(newest end) of a world compared to the TUI, frequently, and ambiguously "new input or
existing output". Three unrelated mechanisms each produce exactly that.

## Cause 1 (primary) — after a resync the client fetched nothing newer

`resumeSentThisConnection` was cleared **only on socket close**
(`app.js` `handleSessionDisconnect`), but `_resumedFromServer` is re-derived from it on
**every** `InitialState`.

A `RequestState` resync runs on a still-open, already-authenticated socket: no close, no
`AuthRequest`, no resume list — and the server sends **no** unprompted replay for it (only
`AuthRequest.resume` triggers one). But the map from that connection's original `AuthRequest`
was still populated, so the resync's `InitialState` set `_resumedFromServer = true`, which
skips `requestGapFill()` *and* sets `_gapFillPending`, which excludes the world from **both**
the phase-1 and phase-2 backfill queues. Net: nothing newer was ever fetched, and
`_gapFillPending` stuck true forever (the `request_id: 0` replay is never registered, so no
watchdog existed). `RequestState` is the dominant Android post-wake path, so this recurred on
every wake until the socket actually closed.

Phase B had already tried to fix this class of bug by keying on `resumeSentThisConnection`
instead of the older `priorWorld && contiguousFrontier(priorWorld) > 0` heuristic — but a map
that outlives its one use behaves *identically* to that heuristic. **The lifetime was the bug,
not the key.**

**Fix:** consume the map at the end of the `InitialState` handler (it applies to exactly one
`InitialState`), plus a standalone watchdog so `_gapFillPending` can never stay true with
nothing in flight.

## Cause 2 — server-side lines that were never broadcast at all

The TUI rendered them; no client ever heard about them, and each burned a `seq`, leaving a
permanent hole in every client's delivered-range tracking.

| Site | Line |
|---|---|
| `handle_disconnected` | the world's final prompt (the `"Disconnected."` line right below it *was* broadcast, which is what made this easy to miss) |
| `handle_prompt` | disconnected-world prompt-as-output — set `needs_output_redraw`, returned, broadcast nothing |
| WONT-ECHO prompt timeout ×2 | raw `output_lines.push` inside `for world in &mut app.worlds`, so no `app` to broadcast from; also bypassed `push_line_respecting_pending`, violating the sorted-by-seq invariant. The connected branch also set `world.prompt` without ever sending `PromptUpdate`. |
| TLS-proxy-death notice ×3 | including the **daemon** copy, which is what Android attaches to; also never sent `WorldDisconnected`, so clients kept showing the world as connected |

**Fix:** a shared `App::push_and_broadcast_line` (push + broadcast iff it landed in
`output_lines`, always with `end_seq: Some(seq)`), and restructuring the two loop-bound sites
to collect and emit after the borrow ends. The duplication across the console/headless/daemon
loops is why these drifted apart, so they now route through one helper.

## Cause 3 — a `before_seq: null` reply carries the NEWEST lines but was prepended

`requestBackfillChunk` sends `before_seq: world._oldest_seq`, which is `null` for a world with
no real seqs — and the server answers that with the **newest** N visible lines. The reply was
registered `kind: 'backfill'`, so the handler blind-`concat`ed it *above* existing content and
deliberately left it out of `_seenRanges`. New output landed hundreds of lines up.

**Fix:** tag the kind at request time (`'initial-fill'` vs `'backfill'`) rather than
re-deriving it on the reply, and place an initial-fill in seq order, marked into `_seenRanges`.
The scroll compensation is skipped for it too — that correction assumes content was inserted
*above* the viewport.

## Also fixed

- **Inverted seq span on the more-mode-off drain.** `World::add_output` appended drained
  `pending_lines` (higher seqs) *after* the loop's newly-allocated lines, so
  `broadcast_output_range` derived `end_seq < first_seq`; the client read that as a mid-buffer
  gap-fill and spliced the batch far from the tail. The drain now runs **before** the append
  loop, which keeps `output_lines` sorted and — as a bonus — puts those released lines inside
  the range every caller already broadcasts, so they reach clients at all for the first time.
  `broadcast_output_range` additionally derives its span from the slice min/max, so a future
  invariant violation degrades to a harmlessly wide forward span instead of misplacing text.
- **`highlight_color` on the live path.** Applied to the buffer but never transmitted, so
  highlighted lines rendered plain until a resync. `ServerData` gains
  `highlight_colors: Vec<Option<String>>`, parallel to the lines in `data` and omitted entirely
  when nothing in the batch is highlighted (zero cost on the hot path).
- **Two `requestGapFill` degradations.** `_max_seq` truthiness and `contiguousFrontier() === 0`
  both treated a legitimate seq 0 as "no data" (`World::next_seq` starts at 0), silently
  downgrading to older-history-only. Now keyed on `hasDeliveredSeqs()`.
- **Pending-clamped gap-fill stalled the pump.** A result truncated by `pending_floor_seq()`
  reported `backfill_complete`, clearing `_gapFillPending` while the client was still behind.
  It now reports incomplete, and `PendingReleased` re-drives it.
- **`seq: 0` broadcasts that stored a real seq** (`SystemMessage`, both Slack/Discord
  non-gagged paths): the line arrived but its seq was never recorded, leaving a permanent gap
  for the Phase C ack audit. They now carry the real seqs via `broadcast_output_range`.

## Deliberately out of scope

- **~200 `App::add_output(...)` call sites target the console's `current_world_index`**
  regardless of which world the action/trigger fired on. They *do* broadcast — the data is
  mis-filed into the wrong client-side buffer, not lost. Threading a world index through every
  command handler is a large, separate refactor.
- **~9 reload/crash-recovery messages** are not broadcast, but clients get `ServerReloading`
  and fully re-bootstrap immediately after, so broadcasting them would be redundant.
- **TF hook output is discarded entirely** (`bridge.rs`'s `messages`/`errors` dropped at all
  four call sites). Real, but invisible on *both* TUI and client — a different bug.

## Status

732/732 `cargo test` (726 baseline + 6 new), clean `cargo clippy` on both the musl and
`webview-gui,native-audio` feature sets, clean musl build. Five mutations each fail a specific
test: un-broadcasting the disconnect prompt, moving the drain back after the append loop,
reverting the span to first/last, reporting a clamped gap-fill complete, and dropping
`highlight_colors`.

## Verification gaps

- **No JS runtime in the sandbox** — `app.js` was not executed. The two decision points changed
  here (the `_resumedFromServer` derivation and the reply-kind routing) were transliterated to
  Python and exercised, including asserting that the *old* behaviour reproduces the bug
  (fetches nothing after a resync, leaves `_gapFillPending` stuck); the surrounding handler
  edits are reviewed and brace-balance-checked only. This is the fourth consecutive change to
  carry this gap.
- **No two-instance or on-device run.** The reported scenario — background the phone past the
  heartbeat, resume (forcing the `RequestState` path), compare each world's bottom against the
  TUI — has not been performed against real processes.

---

# Phase F — stop fixing this one path at a time

## The report

> The android app is still having sync issues for the world output. Recently I've had it
> display a world and it seems to have gotten the last line but skipped the second-to-last
> line and then had the third-to-last line. Sometimes it just misses the last lines.
>
> Consider any logging or testing that can be put in to help automate the detection of the
> issue as claude has taken several stabs at getting this correct in the last week or two.

That second paragraph is the actual brief. Phases B through E each found a real bug and fixed
it, and each fix was specific to the path that happened to be noticed: a bare
`output_lines.push` with no broadcast, a disconnect prompt, a disconnected-world prompt, an
inverted `seq..=end_seq` span, a broadcast match arm that silently stopped matching, a release
batch sized by the wrong formula. Six fixes, six paths, one recurring symptom. The pattern says
the problem is not any individual path — it is that **nothing checks the invariant**, so the
seventh path costs another round trip through a user report.

## What this phase adds

Three detectors, none of which depends on knowing which code path is at fault.

### 1. Server-side broadcast ledger (`World::broadcast_ledger`, `App::audit_broadcast_ledger`)

The server now records every seq it actually puts on the wire, as coalesced inclusive ranges —
the exact mirror of `app.js`'s `_seenRanges`, using the same algorithm so the two sides cannot
drift in what "delivered" means. On each keepalive it compares that ledger against
`output_lines`:

> **Every line in `output_lines` below `pending_floor_seq()` must have been broadcast.**

Any line that fails is logged to `~/.clay/remote.log` as `SEQ-LEDGER` *with its text* — so the
offending path is identifiable from a user's log alone — and then **re-broadcast**, grouped into
runs sharing `(gagged, from_server)` and contiguous seqs so each repair batch's span covers
exactly the lines it carries. The bug class now self-heals instead of costing the user output.

Deliberately not violations: lines at/above the pending floor (held for more-mode on purpose),
the newest line in the buffer (a broadcast for it may be in flight this instant), and an
outstanding partial line. Forward-only via `ledger_audited_upto`, so per-tick cost tracks new
output rather than buffer size.

`/dump` prints the ledger per world. A healthy world shows exactly one range, `(0, next_seq-1)`.
More than one range is the signature of this whole bug class.

### 2. Client-side gap reporting (`ReportGap` → `SEQ-HOLE`)

The ledger proves what the *server* sent. It cannot see a hole that opens in transit — a dropped
frame, a full outbound channel, an Android socket blip. When the client gives up on a seq range
(see below) it now reports the exact range, and the server logs `SEQ-HOLE`. Between the two,
every hole is attributable to one side or the other rather than being a symptom without a
location.

### 3. A path-independent fuzz test (`test_fuzz_every_output_line_is_broadcast`)

200 seeded runs, 40 random operations each, over the operations that mutate `output_lines`
(server chunks with and without a trailing partial prompt, captured user input, client-generated
text, `/recall` blocks, pending release, more-mode toggling). After every step it asserts three
things against what actually came out of the broadcast channel:

- every stored line's seq was broadcast;
- every batch's declared `seq..=end_seq` span matches the number of lines it carries (a wider
  span makes the client mark seqs as delivered that it never received — silent, permanent loss);
- the `seq → text` map a Phase-C client would build (`lineSeq = msg.seq + rawIdx`) matches what
  the server actually stored. **A misfiled seq is the reported symptom**: file line N+1 under
  seq N, and the real seq N+1 is later dropped as an already-seen duplicate, leaving one line
  missing with its neighbours present.

It also runs the shipping `App::audit_broadcast_ledger` as a second, independent oracle and
fails if the two disagree — so a blind spot in the safety net users rely on is itself a test
failure.

## Bugs found and fixed

**1. Unbounded gap-fill request loop (client).** `requestGapFill()` anchors on
`contiguousFrontier()`. A seq the server can no longer produce — Ctrl+L's selective flush and a
splash clear both remove lines from `output_lines`, and the archive prepend caps it at 10k —
freezes that anchor. Every reply then re-delivers only lines the client already holds, the
frontier does not move, and because a full chunk came back `backfill_complete` is false, so the
`ScrollbackLines` handler immediately re-requests the identical range. Forever, on a live
socket, on the client most likely to be metered.

Verified by transliterating the `_seenRanges` family and the handler's loop condition to Python:
against a server whose `output_lines` is `0..999` with seq 500 removed, the old code issues
requests without bound (cut off at 10,000); the new code terminates in 2 requests, identifies
the lost seq exactly, and advances the frontier to 999. Fix: count consecutive no-progress
replies, and at 2 declare the oldest hole lost (`closeOldestSeqHole`), report it via `ReportGap`,
and move on.

**2. Ack-audit suppression was permanent (server).** `evaluate_ack_audit` fired one
`ResyncRequired` per (world, stall position) and then went silent forever. That is exactly wrong
when the resync itself never arrived — and the likeliest cause of a stall is a full outbound
channel, i.e. precisely the condition under which the `ResyncRequired` is also dropped. A client
could sit permanently behind with the server having written it off. Now retries every
`AUDIT_REFIRE_INTERVAL` (6) audits while still stalled: often enough to survive a lost message,
rare enough that a genuinely unfillable hole costs one message per ~3 minutes rather than one
per 30 seconds.

**3. `end_seq: None` alongside a real seq (server, 6 sites).** `push_and_broadcast_line`'s
contract is that `end_seq` is always `Some(seq)`, because a present `end_seq` is what marks the
seq as real — seq 0 is legitimate for a world's first line. Six single-line broadcast sites
(captured user input, the gagged-line path in `process_server_data`, both `handle_disconnected`
messages, and the two Slack/Discord gagged paths) sent a bare `seq`. Whenever that seq was 0 the
client filed the line under an array index instead of a seq and never recorded it as delivered —
a permanent hole for the ack audit to chase.

**4. `appendNewLine` dropped `highlight_color` (client).** `renderOutput()` applies a `/hilite`
background; the incremental live-append path did not. A highlighted line rendered plain until
something forced a full re-render, at which point it silently changed appearance. The colour now
threads through `handleIncomingLine` and the local more-mode queue as well.

## What was ruled out

Worth recording so the next investigation doesn't re-walk it. The live single-user broadcast
path is clean: the fuzz above finds nothing, and neither does an end-to-end run. `World::partial_line`
is effectively dead in production — all four `World::add_output` callers force a trailing
newline, and `process_server_data` holds its partial in `trigger_partial_line` instead — so the
`had_partial_in_output` adjustments in `add_output`/`add_output_to_world`/`emit_client_lines`/
`process_server_data` never fire. The client's ANSI-only-line filter matches the TUI's
(`process_output_line` skips `is_ansi_only_line` too), so it is not a source of divergence.

## Status

738/738 `cargo test` (734 baseline + 4 new: the path-independent fuzz invariant and three
ledger-audit tests, one of which guards the audit's own diagnostic against panicking on
multi-byte MUD text), clean `cargo clippy`, clean musl build.

**End-to-end verified against real processes.** A `-D` daemon, a scripted MUD emitting numbered
lines interleaved with blank lines, ANSI-only lines and unterminated prompts, and a WebSocket
client written for this purpose that speaks the real protocol (challenge-response auth,
`InitialState`, `flush`, `PongCheck`). Result over 280 lines: 280 seqs delivered, **0 holes, 0
span mismatches, 0 misfiled seqs**; `/dump` reports `broadcast_ledger: 1 range(s) [(0, 279)]`
against `next_seq: 280`, and `ledger_audited_upto: Some(278)` confirms the self-audit ran on the
real `PongCheck` path inside the daemon. `remote.log` contains no `SEQ-LEDGER` or `SEQ-HOLE`
entries, which is the correct clean result.

## Verification gaps

- **Still no JS runtime in the sandbox.** `app.js` was not executed. The gap-fill loop and the
  `_seenRanges` operations it depends on were transliterated to Python and exercised (including
  asserting the old code livelocks); the rest of the edit is reviewed and checked with a
  string/regex/template-aware brace balancer, validated against the pre-change file. Fifth
  consecutive change carrying this gap — a headless JS runtime in the dev environment would
  retire it.
- **No on-device Android run**, and no two-instance comparison of a world's tail against the TUI.
- **The detector has not been observed firing in a live process** — only in unit tests. Producing
  a real hole requires injecting a bug into a running binary.
- **Multiuser is out of scope**, as in Phase C: `daemon.rs`'s multiuser path emits
  `seq: 0, end_seq: None` universally, so there are no real seqs there to audit.
