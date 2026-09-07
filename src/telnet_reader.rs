// The one async telnet reader (plan Phase 2, Job 4 of
// investigate-differences-between-tinyfugu-fluffy-stallman.md). Design
// commitment 2: this module is the only place that touches a socket, a
// `TelnetSession`, and an `AppEvent` channel at once. `spawn_telnet_reader`
// is now the sole production reader (Jobs 5-7 migrated `daemon.rs`/
// `commands.rs`/`main.rs`'s thirteen hand-copied reader loops onto it one at
// a time; `testharness.rs`, Job 4 Step 2.2, was first).
//
// `process_telnet`/`find_safe_split_point` are private to `telnet.rs` now
// (Job 8, Step 2.6) — kept only as the characterization/differential test
// oracle, unreachable from here or any other production code. `TelnetResult`
// is `#[cfg(test)]`-only for the same reason.

use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use crate::telnet::{
    StreamReader, TelnetConfig, TelnetEvent, TelnetSession, WriteCommand, TELNET_OPT_CHARSET,
    TELNET_OPT_EOR, TELNET_OPT_GMCP, TELNET_OPT_MCCP2, TELNET_OPT_MSDP, TELNET_OPT_MSP,
    TELNET_OPT_MSSP, TELNET_OPT_NAWS, TELNET_OPT_SGA, TELNET_OPT_TTYPE,
};
use crate::AppEvent;

/// How long `spawn_telnet_reader` waits after the most recent socket read
/// before flushing a still-held-back `pending_text` line on its own,
/// reusing the constant every `prompt_check_sleep` site already uses
/// (`main.rs:16903`, `17365`, `19137`, `19853`, `20055`).
///
/// This is not a nicety — see the plan's "Hard requirement discovered in
/// Job 2" at the top of Phase 2. `TelnetSession` holds back a trailing
/// partial line (up to 32 bytes) so that GA/EOR/WONT-ECHO prompt extraction
/// gives the same answer no matter where a TCP read happens to split the
/// bytes in front of a prompt marker. That is correct for servers that send
/// GA/EOR, but a MUSH or MOO — the norm for the user's own servers — usually
/// sends neither: its prompt would sit in `pending_text` forever, visibly
/// missing, unless something flushes it after a period of silence.
const IDLE_FLUSH_INTERVAL: Duration = Duration::from_millis(150);

/// A `tokio::time::sleep` this far out never fires in practice; used to
/// "park" the idle timer between reads exactly like `main.rs`'s
/// `prompt_check_sleep`/`FAR_FUTURE` pattern.
const FAR_FUTURE: Duration = Duration::from_secs(86400);

/// Read buffer size for one socket read. Matches the middle of the range the
/// thirteen existing loops use (4096-10240); not load-bearing either way —
/// `TelnetSession::feed` is correct for any chunking (see the split-invariance
/// property test in `telnet.rs`).
const READ_BUFFER_SIZE: usize = 8192;

/// Which `AppEvent` shape a `spawn_telnet_reader` instance should emit into.
/// `App`'s single-user world events (`AppEvent::ServerData`,
/// `AppEvent::Prompt`, ...) carry a world *name*; multiuser's per-connection
/// events carry a world *index* plus the owning username instead. A third
/// shape is not anticipated, but if one is ever added, every match on
/// `TelnetTarget` in this file uses an explicit two-arm match with no
/// wildcard, so the compiler — not a code reviewer — will find every site
/// that needs to learn about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelnetTarget {
    /// Single-user (or daemon direct-connect) world, identified by name —
    /// mirrors `AppEvent::ServerData(String, ..)` and friends.
    World(String),
    /// A multiuser server's per-user connection to a world, identified by
    /// the world's global index plus the connecting username — mirrors
    /// `AppEvent::MultiuserServerData(usize, String, ..)` and friends.
    Multiuser { world_index: usize, username: String },
}

/// Build the `AppEvent` that carries plain server text for `target`. Not
/// part of the `TelnetEvent` → `AppEvent` mapping table below (`text` is a
/// whole-outcome field, not a `TelnetEvent`), but it's the same
/// per-target-shape branch, so it lives next to `to_app_event` rather than
/// inline in the reader loop.
fn make_server_data_event(target: &TelnetTarget, bytes: Vec<u8>) -> AppEvent {
    match target {
        TelnetTarget::World(name) => AppEvent::ServerData(name.clone(), bytes),
        TelnetTarget::Multiuser { world_index, username } => {
            AppEvent::MultiuserServerData(*world_index, username.clone(), bytes)
        }
    }
}

/// Build the `AppEvent` that reports the connection closing for `target`.
/// `conn_id` is only meaningful for `World` (`AppEvent::Disconnected`'s
/// second field, used to ignore a stale reader's disconnect after a
/// reconnect); `AppEvent::MultiuserDisconnected` carries no connection id at
/// all, so it is simply not passed through on that arm.
fn make_disconnected_event(target: &TelnetTarget, conn_id: u64) -> AppEvent {
    match target {
        TelnetTarget::World(name) => AppEvent::Disconnected(name.clone(), conn_id),
        TelnetTarget::Multiuser { world_index, username } => {
            AppEvent::MultiuserDisconnected(*world_index, username.clone())
        }
    }
}

/// The event mapper — design commitment 2's anti-drift mechanism. Maps one
/// `TelnetEvent` (from `TelnetSession::feed`/`flush_eof`) to the `AppEvent`
/// `spawn_telnet_reader` should send for it, for the given `target` shape.
///
/// This match is deliberately exhaustive over `TelnetEvent` with **no
/// wildcard arm**, on both `TelnetTarget` branches: adding a `TelnetEvent`
/// variant later will not compile until this function (and every other
/// exhaustive match on it) says what each target shape does with it. See
/// design commitment 2 and Phase 2's introduction in the plan.
///
/// Two classes of `None`, both deliberate — never a placeholder for future
/// work landing in *this* job:
///
/// - `ProtocolError` is informational on every target: nothing downstream
///   needs an `AppEvent` for it. It is still surfaced — `spawn_telnet_reader`
///   logs it via `debug_log` directly, since there is no `AppEvent::Telnet`-
///   carried payload built for it here. `CompressionStarted`/
///   `CompressionEnded` used to be in this same informational bucket, but
///   Job 10a (plan Phase 3, step 3.3, the MCCP2 hot-reload guard) gave `World`
///   a real consumer — `App::handle_telnet_event` maintains
///   `World::mccp2_active` from them, which the reload restore path needs to
///   disconnect a compressed world instead of handing its dead zlib stream to
///   a fresh plaintext parser — so they now forward through `AppEvent::Telnet`
///   on `World` like `OptionEnabled`/`OptionDisabled`. `Multiuser` still maps
///   both to `None`: it has no per-user reload-restore path this guards.
/// - Every `Multiuser` arm below `TelnetDetected`/`Prompt` has no
///   corresponding `AppEvent` at all: `NawsRequested`, `TtypeRequested`,
///   `GmcpMessage`, `MsdpVariable`, `MsspData` (Job 12) only ever reach `App`
///   through `AppEvent::Telnet(TelnetTarget::World(name), ev)` (Phase 2,
///   Step 2.7), which `App::handle_telnet_event` resolves via a plain
///   `world_idx` — multiuser's per-user connection state lives in
///   `user_connections`, keyed by `(world_index, username)`, not in
///   `App::worlds`, so there is no `world_idx`-shaped home for these on a
///   `Multiuser` target yet. Finding 2 in the plan already names today's
///   multiuser reader loops as calling the old `CharsetRequested` with an
///   empty world-name string that `find_world_index("")` always rejects,
///   i.e. already-dead code; this mapper does not resurrect that by
///   reproducing it. Giving multiuser connections real NAWS/TTYPE/GMCP/MSDP/
///   MSSP support needs a per-user-shaped event, not this job's to invent.
///
/// Job 4.5 added `OptionEnabled`/`OptionDisabled`/`WontEchoPromptHint`.
/// `OptionEnabled`/`OptionDisabled` carry a `u8` option code rather than
/// being one `TelnetEvent` variant per option, so the inner match on that
/// code can't itself be exhaustive over every possible `u8` the way the
/// outer match is over `TelnetEvent` — but `TelnetSession` only ever emits
/// these two for the fixed set of options Clay accepts (SGA, EOR, NAWS,
/// TTYPE, CHARSET, MCCP2, GMCP, MSDP, and — as of Job 12 — MSSP; see the
/// `OptionEnabled` doc comment in telnet.rs), so every one of those is
/// spelled out by name below with a
/// trailing wildcard only for option codes the session cannot actually
/// produce. On `World`: GMCP and MSDP are the two `process_telnet` already
/// reported as `gmcp_negotiated`/`msdp_negotiated` and that
/// `App::handle_telnet_event` still consumes (`OptionEnabled(TELNET_OPT_GMCP)`
/// triggers `Core.Hello`+`Core.Supports.Set` — see `App::handle_gmcp_negotiated`);
/// the other seven (added MSSP, Job 12) map to `None` because nothing
/// downstream reads an "option enabled" signal for them today
/// (`NawsRequested`/`TtypeRequested`/`CharsetRequest`/`MsspData` already
/// cover the actions those options unlock, independently of this variant).
/// `OptionDisabled` is Job 9's fix for
/// finding 4 ("DONT/WONT are ignored entirely and never clear anything"):
/// GMCP, MSDP and NAWS — the three options with a `World` mirror to clear
/// (`gmcp_enabled`/`msdp_enabled`/`naws_enabled`; see
/// `App::handle_telnet_event`'s `OptionDisabled` arm) — map through to
/// `AppEvent::Telnet` on `World`, same as their `OptionEnabled`
/// counterparts; the rest map to `None` for the same reason `OptionEnabled`
/// does. Multiuser still maps every option to `None` on both variants —
/// this job doesn't invent a per-user-shaped mirror any more than Job 4.5
/// did. `WontEchoPromptHint` maps through `AppEvent::Telnet` on `World`
/// (drives `World::uses_wont_echo_prompt` via `App::handle_wont_echo_seen`,
/// the 150ms timeout-prompt path) and to `None` on `Multiuser`, which has no
/// per-user equivalent — consistent with every other Multiuser gap above.
///
/// Job 10b (plan Phase 3, step 3.4, finding 7) adds `EchoOff`/`EchoOn`: real
/// password-masking state, distinct from `WontEchoPromptHint`'s prompt-boundary
/// heuristic above. Both map through `AppEvent::Telnet` on `World`
/// (`App::handle_telnet_event` maintains `World::echo_masked` from them and
/// broadcasts the change to web/GUI clients) and to `None` on `Multiuser`, same
/// gap as every other per-user-shaped signal here.
pub fn to_app_event(target: &TelnetTarget, ev: TelnetEvent) -> Option<AppEvent> {
    match target {
        TelnetTarget::World(name) => match ev {
            TelnetEvent::TelnetDetected => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::TelnetDetected))
            }
            TelnetEvent::Prompt(bytes) => Some(AppEvent::Prompt(name.clone(), bytes)),
            TelnetEvent::NawsRequested => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::NawsRequested))
            }
            TelnetEvent::TtypeRequested => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::TtypeRequested))
            }
            TelnetEvent::CharsetRequest(charsets) => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::CharsetRequest(charsets)))
            }
            TelnetEvent::GmcpMessage(package, json) => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::GmcpMessage(package, json)))
            }
            TelnetEvent::MsdpVariable(variable, value) => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::MsdpVariable(variable, value)))
            }
            // Job 12 (plan Phase 4, 4.2): App::handle_telnet_event replaces
            // World::mssp_data with this - same shape as MsdpVariable above.
            TelnetEvent::MsspData(pairs) => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::MsspData(pairs)))
            }
            // Job 10a: App::handle_telnet_event maintains World::mccp2_active from
            // these, which the reload restore path needs (see this function's doc
            // comment).
            TelnetEvent::CompressionStarted => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::CompressionStarted))
            }
            TelnetEvent::CompressionEnded => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::CompressionEnded))
            }
            // No AppEvent::Telnet payload built for this — the reader logs
            // it directly instead of mapping it.
            TelnetEvent::ProtocolError(_) => None,
            TelnetEvent::OptionEnabled(opt) => match opt {
                // The two option codes App::handle_telnet_event consumes
                // today (see this function's doc comment) — losing either
                // silently breaks GMCP announcement or the
                // gmcp_enabled/msdp_enabled mirrors.
                TELNET_OPT_GMCP | TELNET_OPT_MSDP => {
                    Some(AppEvent::Telnet(target.clone(), TelnetEvent::OptionEnabled(opt)))
                }
                // Negotiating successfully has no AppEvent of its own for
                // these — NawsRequested/TtypeRequested/CharsetRequest/MsspData
                // already cover the actions they unlock (MSSP added Job 12,
                // plan Phase 4, 4.2 - the MsspData event carries the payload
                // itself, so OptionEnabled(MSSP) needs no action of its own,
                // same as CHARSET/MCCP2/TTYPE above). MSP (Job 14) is the same
                // shape again: the in-band `!!SOUND(...)`/`!!MUSIC(...)` text
                // triggers work whether or not option 90 ever negotiates (see
                // `msp_enabled`'s doc comment), so confirming the option
                // itself unlocks nothing `MspTrigger` doesn't already cover.
                TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_NAWS | TELNET_OPT_TTYPE
                | TELNET_OPT_CHARSET | TELNET_OPT_MCCP2 | TELNET_OPT_MSSP | TELNET_OPT_MSP => None,
                // TelnetSession never actually emits OptionEnabled for any
                // other code (see its WILL/DO accept lists) - unreachable in
                // practice, but the u8 payload still needs a catch-all.
                _ => None,
            },
            // Job 9 / finding 4: the three options with a World mirror to
            // clear (see this function's doc comment) forward through;
            // everything else nothing reacts to, same as OptionEnabled.
            TelnetEvent::OptionDisabled(opt) => match opt {
                TELNET_OPT_GMCP | TELNET_OPT_MSDP | TELNET_OPT_NAWS => {
                    Some(AppEvent::Telnet(target.clone(), TelnetEvent::OptionDisabled(opt)))
                }
                // MSSP (Job 12) has no World mirror to clear on disable beyond
                // World::mssp_data itself, which clear_connection_state already
                // handles independently of this event - see its doc comment.
                // MSP (Job 14) has no mirror at all to clear - msp_enabled is a
                // TelnetConfig/WorldSettings toggle, not a negotiated-state flag.
                TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_TTYPE | TELNET_OPT_CHARSET
                | TELNET_OPT_MCCP2 | TELNET_OPT_MSSP | TELNET_OPT_MSP => None,
                _ => None,
            },
            TelnetEvent::WontEchoPromptHint => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::WontEchoPromptHint))
            }
            // Job 10b (plan Phase 3, step 3.4, finding 7): App::handle_telnet_event
            // maintains World::echo_masked from these (and broadcasts it to
            // web/GUI clients) - the reader must forward both or password masking
            // never reaches App at all.
            TelnetEvent::EchoOff => Some(AppEvent::Telnet(target.clone(), TelnetEvent::EchoOff)),
            TelnetEvent::EchoOn => Some(AppEvent::Telnet(target.clone(), TelnetEvent::EchoOn)),
            // Job 14 (plan Phase 4): App::handle_telnet_event resolves this
            // into an actual audio::play_file call - see its doc comment.
            TelnetEvent::MspTrigger(trigger) => {
                Some(AppEvent::Telnet(target.clone(), TelnetEvent::MspTrigger(trigger)))
            }
        },
        TelnetTarget::Multiuser { world_index, username } => match ev {
            TelnetEvent::TelnetDetected => {
                Some(AppEvent::MultiuserTelnetDetected(*world_index, username.clone()))
            }
            TelnetEvent::Prompt(bytes) => {
                Some(AppEvent::MultiuserPrompt(*world_index, username.clone(), bytes))
            }
            // No per-user-shaped AppEvent exists for any of these today (see
            // the doc comment above) — a future job needs to add one, or
            // accept the gap, before multiuser connections get NAWS/TTYPE/
            // GMCP/MSDP support through this reader.
            TelnetEvent::NawsRequested => None,
            TelnetEvent::TtypeRequested => None,
            TelnetEvent::CharsetRequest(_) => None,
            TelnetEvent::GmcpMessage(_, _) => None,
            TelnetEvent::MsdpVariable(_, _) => None,
            TelnetEvent::MsspData(_) => None,
            TelnetEvent::CompressionStarted => None,
            TelnetEvent::CompressionEnded => None,
            TelnetEvent::ProtocolError(_) => None,
            // No per-user-shaped AppEvent::Telnet OptionEnabled equivalent
            // exists, and today's multiuser dispatch loop
            // (run_multiuser_server in daemon.rs) has no arm for
            // AppEvent::Telnet at all (it's never constructed for this
            // target) - so mapping every option to None here reproduces
            // that existing (already dead) behaviour rather than silently
            // fixing or worsening it.
            TelnetEvent::OptionEnabled(_) => None,
            TelnetEvent::OptionDisabled(_) => None,
            TelnetEvent::WontEchoPromptHint => None,
            // No per-user-shaped World to mirror echo_masked onto - same gap as
            // every other Multiuser arm above.
            TelnetEvent::EchoOff => None,
            TelnetEvent::EchoOn => None,
            // No per-user audio path exists for multiuser connections (the
            // server process has no single "current user" to play a sound
            // for) - same gap as GmcpMessage/MsdpVariable above, not this
            // job's to invent.
            TelnetEvent::MspTrigger(_) => None,
        },
    }
}

/// Spawn the one async telnet reader task for a connection. Reads raw bytes
/// from `read_half`, drives them through a fresh `TelnetSession`, writes
/// `outcome.wire` (negotiation replies) back through `cmd_tx`, maps each
/// `TelnetEvent` to an `AppEvent` via `to_app_event` and sends it, then sends
/// the emitted plain text as an `AppEvent::ServerData`/`MultiuserServerData`.
///
/// Unifies one behavioural difference the plan calls out by name: of the
/// thirteen existing reader loops, four print "Connection closed by
/// server.\n" on a clean EOF and nine silently do not. This reader always
/// prints it — a proxy or MUD disconnect going unremarked in the transcript
/// is the bug, not a feature worth keeping either variant of.
///
/// Runs the mandatory idle flush described on `IDLE_FLUSH_INTERVAL`: a
/// `tokio::select!` between the socket read and a resettable timer, reset on
/// every read, that releases `TelnetSession::take_pending_text()` after
/// `IDLE_FLUSH_INTERVAL` of silence. Without this, a prompt on a server that
/// never sends GA/EOR (MUSH/MOO) would sit invisibly in `pending_text` until
/// more output happened to arrive.
pub fn spawn_telnet_reader(
    mut read_half: StreamReader,
    cmd_tx: mpsc::Sender<WriteCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    target: TelnetTarget,
    conn_id: u64,
    cfg: TelnetConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let initiate_negotiation = cfg.initiate_negotiation;
        let mut session = TelnetSession::new(cfg);
        let mut buffer = vec![0u8; READ_BUFFER_SIZE];

        // Job 11 (plan Phase 3, step 3.5, finding 5): send Clay's opening
        // offer before the read loop starts, once per connection, gated on
        // the per-world setting (default on) so a server that reacts badly
        // to being spoken to first can be worked around without a rebuild.
        // Every one of the thirteen migrated call sites gets this for free
        // by going through this one function.
        if initiate_negotiation {
            let opening = session.initial_negotiation();
            if !opening.is_empty() {
                let _ = cmd_tx.send(WriteCommand::Raw(opening)).await;
            }
        }

        let idle_timer = tokio::time::sleep(FAR_FUTURE);
        tokio::pin!(idle_timer);

        loop {
            tokio::select! {
                result = read_half.read(&mut buffer) => {
                    match result {
                        Ok(0) => {
                            // Clean EOF. Release whatever text was still held
                            // back rather than losing it (this is exactly
                            // what flush_eof is for), then the unified close
                            // message and disconnect event.
                            let outcome = session.flush_eof();
                            if !outcome.text.is_empty() {
                                let _ = event_tx.send(make_server_data_event(&target, outcome.text)).await;
                            }
                            let _ = event_tx.send(make_server_data_event(
                                &target,
                                b"Connection closed by server.\n".to_vec(),
                            )).await;
                            let _ = event_tx.send(make_disconnected_event(&target, conn_id)).await;
                            return;
                        }
                        Ok(n) => {
                            let outcome = session.feed(&buffer[..n]);

                            if !outcome.wire.is_empty() {
                                let _ = cmd_tx.send(WriteCommand::Raw(outcome.wire)).await;
                            }

                            for ev in outcome.events {
                                if let TelnetEvent::ProtocolError(ref msg) = ev {
                                    // No AppEvent to carry this; log it directly
                                    // so an abandoned subnegotiation or a tripped
                                    // MCCP2 bomb guard is still diagnosable
                                    // (see CLAUDE.md's remote.log-style logging
                                    // guidance and the `ProtocolError` doc
                                    // comment in telnet.rs).
                                    crate::debug_log(true, &format!(
                                        "telnet_reader ({target:?}): {msg}"
                                    ));
                                }
                                if let Some(app_ev) = to_app_event(&target, ev) {
                                    let _ = event_tx.send(app_ev).await;
                                }
                            }

                            if !outcome.text.is_empty() {
                                let _ = event_tx.send(make_server_data_event(&target, outcome.text)).await;
                            }

                            // Mandatory idle flush: reset on every read (see
                            // IDLE_FLUSH_INTERVAL's doc comment / the plan's
                            // "Hard requirement discovered in Job 2").
                            idle_timer.as_mut().reset(tokio::time::Instant::now() + IDLE_FLUSH_INTERVAL);
                        }
                        Err(e) => {
                            let msg = format!("Read error: {}", e);
                            let _ = event_tx.send(make_server_data_event(&target, msg.into_bytes())).await;
                            let _ = event_tx.send(make_disconnected_event(&target, conn_id)).await;
                            return;
                        }
                    }
                }
                _ = &mut idle_timer => {
                    let text = session.take_pending_text();
                    if !text.is_empty() {
                        let _ = event_tx.send(make_server_data_event(&target, text)).await;
                    }
                    // Nothing more to flush until the next read arms it again.
                    idle_timer.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telnet::{
        TELNET_GA, TELNET_IAC, TELNET_OPT_ECHO, TELNET_OPT_GMCP, TELNET_OPT_MCCP2,
        TELNET_OPT_MSDP, TELNET_OPT_NAWS, TELNET_OPT_SGA, TELNET_SB, TELNET_SE, TELNET_WILL,
        TELNET_WONT, TELNET_DO,
    };
    use tokio::io::AsyncWriteExt;

    /// Default test config with Job 11's opening negotiation turned OFF. Every existing
    /// test in this module predates `initial_negotiation` and asserts on the exact first
    /// `WriteCommand`/reply sequence a canned script produces; leaving the feature's own
    /// default-on posture here would make each of them race the unprompted six-option
    /// opening block sent at task start (see `spawn_telnet_reader`), landing it as an
    /// unexpected `next_cmd()`. The dedicated `initial_negotiation_*` tests below use
    /// `TelnetConfig::default()` (or an explicit `true`) instead, to prove the *on*
    /// behavior at the production default.
    fn test_cfg() -> TelnetConfig {
        TelnetConfig { initiate_negotiation: false, ..TelnetConfig::default() }
    }

    /// Drives `spawn_telnet_reader` end-to-end over an in-process
    /// `tokio::io::duplex` pair standing in for the socket: `send` writes
    /// bytes as if they came from the MUD server, `event_rx`/`cmd_rx` observe
    /// exactly what production code would see (`AppEvent`s and, separately,
    /// `WriteCommand::Raw` wire replies). `StreamReader::Duplex` (telnet.rs,
    /// `#[cfg(test)]`-only) is what makes this possible without a real
    /// socket, while still exercising the exact `StreamReader` type
    /// `spawn_telnet_reader` takes in production.
    struct Harness {
        server: Option<tokio::io::DuplexStream>,
        event_rx: mpsc::Receiver<AppEvent>,
        cmd_rx: mpsc::Receiver<WriteCommand>,
        _handle: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn spawn(target: TelnetTarget) -> Self {
            Self::spawn_with_conn_id(target, 1)
        }

        fn spawn_with_conn_id(target: TelnetTarget, conn_id: u64) -> Self {
            Self::spawn_with_cfg(target, conn_id, test_cfg())
        }

        /// Like `spawn_with_conn_id`, but with an explicit `TelnetConfig` — used by the
        /// `initial_negotiation_*` tests below, which need to flip
        /// `initiate_negotiation` independently of every other test's `test_cfg()`
        /// default (off).
        fn spawn_with_cfg(target: TelnetTarget, conn_id: u64, cfg: TelnetConfig) -> Self {
            let (client, server) = tokio::io::duplex(4096);
            let (read_half, _write_half) = tokio::io::split(client);
            let (cmd_tx, cmd_rx) = mpsc::channel(16);
            let (event_tx, event_rx) = mpsc::channel(16);
            let handle = spawn_telnet_reader(
                StreamReader::Duplex(read_half),
                cmd_tx,
                event_tx,
                target,
                conn_id,
                cfg,
            );
            Harness { server: Some(server), event_rx, cmd_rx, _handle: handle }
        }

        async fn send(&mut self, bytes: &[u8]) {
            self.server.as_mut().expect("server side already closed")
                .write_all(bytes).await.expect("write into duplex");
        }

        /// Drop the "server" side of the duplex pair, simulating the MUD
        /// disconnecting: the reader's next socket read resolves to `Ok(0)`.
        fn close_server(&mut self) {
            self.server = None;
        }

        async fn next_event(&mut self) -> AppEvent {
            tokio::time::timeout(Duration::from_secs(5), self.event_rx.recv())
                .await
                .expect("timed out waiting for AppEvent")
                .expect("event channel closed")
        }

        async fn next_cmd(&mut self) -> WriteCommand {
            tokio::time::timeout(Duration::from_secs(5), self.cmd_rx.recv())
                .await
                .expect("timed out waiting for WriteCommand")
                .expect("cmd channel closed")
        }
    }

    // ==================================================================
    // to_app_event: the exhaustive mapping table itself, checked directly
    // (no async/socket machinery needed). This is what makes the "no
    // wildcard arm" claim in the doc comment verifiable rather than just
    // asserted in prose.
    // ==================================================================

    #[test]
    fn to_app_event_world_target_mapping() {
        let t = TelnetTarget::World("w".to_string());

        assert!(matches!(
            to_app_event(&t, TelnetEvent::TelnetDetected),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected)) if n == "w"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::Prompt(vec![1, 2, 3])),
            Some(AppEvent::Prompt(n, b)) if n == "w" && b == vec![1, 2, 3]
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::NawsRequested),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::NawsRequested)) if n == "w"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::TtypeRequested),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TtypeRequested)) if n == "w"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CharsetRequest(vec!["UTF-8".to_string()])),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CharsetRequest(cs)))
                if n == "w" && cs == vec!["UTF-8".to_string()]
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::GmcpMessage("Core.Hello".to_string(), "{}".to_string())),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::GmcpMessage(p, j)))
                if n == "w" && p == "Core.Hello" && j == "{}"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::MsdpVariable("HP".to_string(), "100".to_string())),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::MsdpVariable(v, val)))
                if n == "w" && v == "HP" && val == "100"
        ));
        // Job 12 (plan Phase 4, 4.2): MsspData forwards on World the same way
        // MsdpVariable does.
        assert!(matches!(
            to_app_event(&t, TelnetEvent::MsspData(vec![("NAME".to_string(), "Test MUD".to_string())])),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::MsspData(pairs)))
                if n == "w" && pairs == vec![("NAME".to_string(), "Test MUD".to_string())]
        ));
        // Job 10a: these now forward like OptionEnabled/OptionDisabled do, so
        // App::handle_telnet_event can maintain World::mccp2_active.
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CompressionStarted),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionStarted)) if n == "w"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CompressionEnded),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionEnded)) if n == "w"
        ));
        // No AppEvent::Telnet payload built for ProtocolError.
        assert!(to_app_event(&t, TelnetEvent::ProtocolError("boom".to_string())).is_none());

        // Job 4.5: OptionEnabled(GMCP)/OptionEnabled(MSDP) are the two that
        // matter - losing either silently breaks Core.Hello announcement or
        // the gmcp_enabled/msdp_enabled mirrors (see to_app_event's doc
        // comment).
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionEnabled(opt)))
                if n == "w" && opt == TELNET_OPT_GMCP
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionEnabled(TELNET_OPT_MSDP)),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionEnabled(opt)))
                if n == "w" && opt == TELNET_OPT_MSDP
        ));
        // Every other option Clay accepts negotiates with no AppEvent of its
        // own.
        for opt in [
            TELNET_OPT_SGA,
            TELNET_OPT_EOR,
            TELNET_OPT_NAWS,
            TELNET_OPT_TTYPE,
            TELNET_OPT_CHARSET,
            TELNET_OPT_MCCP2,
            TELNET_OPT_MSSP, // Job 12 (plan Phase 4, 4.2)
        ] {
            assert!(to_app_event(&t, TelnetEvent::OptionEnabled(opt)).is_none());
        }
        // Job 9 (finding 4): OptionDisabled(GMCP/MSDP/NAWS) now reaches
        // App::handle_telnet_event, which clears the matching mirror -
        // updated from Job 4.5/8's "no consumer anywhere yet" (both `None`).
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_GMCP)),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionDisabled(opt)))
                if n == "w" && opt == TELNET_OPT_GMCP
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_MSDP)),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionDisabled(opt)))
                if n == "w" && opt == TELNET_OPT_MSDP
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_NAWS)),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionDisabled(opt)))
                if n == "w" && opt == TELNET_OPT_NAWS
        ));
        // The rest still have no World mirror to clear.
        for opt in [
            TELNET_OPT_SGA, TELNET_OPT_EOR, TELNET_OPT_TTYPE, TELNET_OPT_CHARSET,
            TELNET_OPT_MCCP2, TELNET_OPT_MSSP, // MSSP added Job 12 (plan Phase 4, 4.2)
        ] {
            assert!(to_app_event(&t, TelnetEvent::OptionDisabled(opt)).is_none());
        }
        // WontEchoPromptHint drives World::uses_wont_echo_prompt via
        // App::handle_telnet_event's WontEchoPromptHint arm.
        assert!(matches!(
            to_app_event(&t, TelnetEvent::WontEchoPromptHint),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::WontEchoPromptHint)) if n == "w"
        ));
    }

    #[test]
    fn to_app_event_multiuser_target_mapping() {
        let t = TelnetTarget::Multiuser { world_index: 3, username: "bob".to_string() };

        assert!(matches!(
            to_app_event(&t, TelnetEvent::TelnetDetected),
            Some(AppEvent::MultiuserTelnetDetected(3, u)) if u == "bob"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::Prompt(vec![9])),
            Some(AppEvent::MultiuserPrompt(3, u, b)) if u == "bob" && b == vec![9]
        ));
        // No (usize, String)-shaped AppEvent exists for any of these yet
        // (see to_app_event's doc comment) — this is finding 2's dead-code
        // gap, not reproduced here, plus the further gap Job 4 discovered.
        assert!(to_app_event(&t, TelnetEvent::NawsRequested).is_none());
        assert!(to_app_event(&t, TelnetEvent::TtypeRequested).is_none());
        assert!(to_app_event(&t, TelnetEvent::CharsetRequest(vec!["UTF-8".to_string()])).is_none());
        assert!(to_app_event(&t, TelnetEvent::GmcpMessage("Core.Hello".to_string(), "{}".to_string())).is_none());
        assert!(to_app_event(&t, TelnetEvent::MsdpVariable("HP".to_string(), "100".to_string())).is_none());
        assert!(to_app_event(&t, TelnetEvent::MsspData(vec![("NAME".to_string(), "Test MUD".to_string())])).is_none());
        assert!(to_app_event(&t, TelnetEvent::CompressionStarted).is_none());
        assert!(to_app_event(&t, TelnetEvent::CompressionEnded).is_none());
        assert!(to_app_event(&t, TelnetEvent::ProtocolError("boom".to_string())).is_none());
        // Job 4.5: Multiuser has no (usize, String)-shaped GmcpNegotiated/
        // MsdpNegotiated/WontEchoSeen equivalent, and today's multiuser
        // dispatch loop wildcards these away even when a reader loop does
        // emit them - so every option (and the echo hint) maps to None here,
        // same as every other gap on this target.
        assert!(to_app_event(&t, TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)).is_none());
        assert!(to_app_event(&t, TelnetEvent::OptionEnabled(TELNET_OPT_MSDP)).is_none());
        assert!(to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_GMCP)).is_none());
        assert!(to_app_event(&t, TelnetEvent::WontEchoPromptHint).is_none());
    }

    // ==================================================================
    // End-to-end over tokio::io::duplex: one canned script driving a real
    // spawn_telnet_reader task, once per TelnetTarget shape. Both scripts
    // are identical on the wire; only the resulting AppEvent shapes (and,
    // for Multiuser, which ones are silently dropped) differ.
    // ==================================================================

    /// `IAC WILL SGA` `IAC DO NAWS` "Welcome\r\n" "Login: " `IAC GA` — a
    /// server that negotiates one option we accept unconditionally (SGA),
    /// one that additionally produces a `TelnetEvent` (NAWS), and then an
    /// old-school GA-terminated prompt. Exercises wire replies, `TelnetDetected`,
    /// `NawsRequested`, and `Prompt`/text-flush ordering (prompt before the
    /// text that precedes it, matching every existing reader loop) all in
    /// one read.
    fn canned_script() -> Vec<u8> {
        let mut data = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA];
        data.extend_from_slice(&[TELNET_IAC, TELNET_DO, TELNET_OPT_NAWS]);
        data.extend_from_slice(b"Welcome\r\n");
        data.extend_from_slice(b"Login: ");
        data.extend_from_slice(&[TELNET_IAC, TELNET_GA]);
        data
    }

    #[tokio::test]
    async fn world_target_negotiates_and_emits_events() {
        let mut h = Harness::spawn(TelnetTarget::World("myworld".to_string()));
        h.send(&canned_script()).await;

        // outcome.wire really does reach cmd_tx: negotiation is answered.
        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert!(bytes.windows(3).any(|w| w == [TELNET_IAC, TELNET_DO, TELNET_OPT_SGA]));
                assert!(bytes.windows(3).any(|w| w == [TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS]));
            }
            _ => panic!("expected a Raw wire write"),
        }

        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "myworld"));
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::NawsRequested) if n == "myworld"));
        assert!(matches!(
            h.next_event().await,
            AppEvent::Prompt(n, b) if n == "myworld" && b == b"Login: ".to_vec()
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "myworld" && b == b"Welcome\r\n".to_vec()
        ));

        // A second, ordinary read still works after all that negotiation.
        h.send(b"MARKER\n").await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "myworld" && b == b"MARKER\n".to_vec()
        ));
    }

    #[tokio::test]
    async fn multiuser_target_drops_unsupported_events_but_keeps_the_rest() {
        let mut h = Harness::spawn(TelnetTarget::Multiuser {
            world_index: 5,
            username: "alice".to_string(),
        });
        h.send(&canned_script()).await;

        // The session negotiates identically regardless of target shape.
        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert!(bytes.windows(3).any(|w| w == [TELNET_IAC, TELNET_DO, TELNET_OPT_SGA]));
                assert!(bytes.windows(3).any(|w| w == [TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS]));
            }
            _ => panic!("expected a Raw wire write"),
        }

        // TelnetDetected survives...
        assert!(matches!(
            h.next_event().await,
            AppEvent::MultiuserTelnetDetected(5, ref u) if u == "alice"
        ));
        // ...but NawsRequested has no AppEvent for Multiuser yet, so the very
        // next event must be the Prompt, not a NAWS event sitting in between.
        assert!(matches!(
            h.next_event().await,
            AppEvent::MultiuserPrompt(5, ref u, ref b) if u == "alice" && b == b"Login: "
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::MultiuserServerData(5, ref u, ref b) if u == "alice" && b == b"Welcome\r\n"
        ));

        h.send(b"MARKER\n").await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::MultiuserServerData(5, ref u, ref b) if u == "alice" && b == b"MARKER\n"
        ));
    }

    // ==================================================================
    // Job 4.5: the reader-level proof that migrating a production loop onto
    // spawn_telnet_reader will not silently drop GMCP announcement, MSDP
    // negotiation, or the WONT-ECHO prompt hint - the three TelnetResult
    // flags Job 4 found TelnetEvent had no equivalent for.
    // ==================================================================

    #[tokio::test]
    async fn will_gmcp_emits_gmcp_negotiated() {
        let mut h = Harness::spawn(TelnetTarget::World("myworld".to_string()));
        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP]).await;

        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(bytes, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_GMCP]);
            }
            _ => panic!("expected a Raw wire write"),
        }
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "myworld"));
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)) if n == "myworld"));
    }

    #[tokio::test]
    async fn will_msdp_emits_msdp_negotiated() {
        let mut h = Harness::spawn(TelnetTarget::World("myworld".to_string()));
        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MSDP]).await;

        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(bytes, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MSDP]);
            }
            _ => panic!("expected a Raw wire write"),
        }
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "myworld"));
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::OptionEnabled(TELNET_OPT_MSDP)) if n == "myworld"));
    }

    #[tokio::test]
    async fn wont_echo_emits_wont_echo_seen() {
        let mut h = Harness::spawn(TelnetTarget::World("myworld".to_string()));
        let mut data = b"Login: ".to_vec();
        data.extend_from_slice(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_ECHO]);
        h.send(&data).await;

        // Job 10b (plan Phase 3, step 3.4, finding 7): WONT ECHO now gets a real `IAC
        // DONT ECHO` reply and an EchoOn event, in addition to (not instead of) the
        // pre-existing WontEchoPromptHint - the comment below predates that job.
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "myworld"));
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::WontEchoPromptHint) if n == "myworld"));
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::EchoOn) if n == "myworld"));
        // The trailing partial line is extracted as a prompt, same as the
        // GA/EOR case.
        assert!(matches!(
            h.next_event().await,
            AppEvent::Prompt(n, b) if n == "myworld" && b == b"Login: ".to_vec()
        ));
    }

    // ==================================================================
    // The mandatory idle flush (plan Phase 2's "Hard requirement discovered
    // in Job 2"): a prompt with no trailing newline and no GA/EOR must not
    // be emitted until IDLE_FLUSH_INTERVAL of silence has passed.
    // ==================================================================

    #[tokio::test(start_paused = true)]
    async fn idle_flush_releases_held_back_prompt_after_150ms() {
        let mut h = Harness::spawn(TelnetTarget::World("w".to_string()));

        // No GA, no trailing newline: TelnetSession's text hold-back keeps
        // this in pending_text rather than flushing it immediately.
        h.send(b"Login: ").await;

        // Let the reader task actually poll the read and process it before
        // asserting nothing came out — without advancing virtual time (a
        // real `.await` on a timer here would let the paused clock
        // auto-advance to the idle timer and defeat the point of the test).
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            matches!(h.event_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "a GA/EOR-less prompt must stay held back until the idle flush fires"
        );

        // Advance past the idle flush interval; the reader's `select!` timer
        // branch fires and releases the held-back text.
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;

        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"Login: ".to_vec()
        ));
    }

    // ==================================================================
    // EOF handling: the unified "Connection closed by server." message
    // (plan: four of the thirteen existing loops printed it, nine didn't;
    // this reader always does) plus the Disconnected event, conn_id intact.
    // ==================================================================

    #[tokio::test]
    async fn eof_emits_unified_close_message_then_disconnected() {
        let mut h = Harness::spawn_with_conn_id(TelnetTarget::World("w".to_string()), 42);
        h.close_server(); // peer gone => the reader's next read returns Ok(0)

        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"Connection closed by server.\n".to_vec()
        ));
        assert!(matches!(h.next_event().await, AppEvent::Disconnected(n, id) if n == "w" && id == 42));
    }

    #[tokio::test]
    async fn eof_flushes_held_back_text_before_the_close_message() {
        let mut h = Harness::spawn(TelnetTarget::World("w".to_string()));
        // Held back (no trailing newline, no GA) right up until the peer
        // disconnects — flush_eof must still release it rather than lose it.
        h.send(b"half a line").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        h.close_server();

        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"half a line".to_vec()
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"Connection closed by server.\n".to_vec()
        ));
        assert!(matches!(h.next_event().await, AppEvent::Disconnected(n, _) if n == "w"));
    }

    // ==================================================================
    // Plan Job 5 (daemon.rs migration, investigate-differences-between-
    // tinyfugu-fluffy-stallman.md): proof that the proxy-shaped reader
    // loops' new MCCP2 support actually decompresses. Before Job 5,
    // `connect_daemon_world`'s Unix-socket and named-pipe TLS-proxy reader
    // loops (daemon.rs, the `tls_proxy_enabled` branches) accepted
    // `IAC DO MCCP2` and then never decompressed anything — finding 1's
    // live form — so a compressed MUD's output rendered as binary garbage
    // from that point on. Both loops now call `spawn_telnet_reader` with
    // `TelnetTarget::World`, exactly as this test does; a real Unix-socket
    // TLS proxy needs a spawned proxy process and certs, which is
    // disproportionate to what's being proven here (the proxy loops differ
    // from a direct connection only in which `StreamReader` variant wraps
    // the socket — see `StreamReader`'s `AsyncRead` impl in telnet.rs, which
    // `spawn_telnet_reader` is already generic over). Also covers the
    // deliberate close-message unification (Job 4): a compressed
    // connection's EOF still gets the same "Connection closed by server."
    // line the proxy loops used to skip entirely.
    // ==================================================================

    /// Build a complete zlib-wrapped compressed blob, matching the wire
    /// format real MCCP2 servers send. Mirrors telnet.rs's own
    /// `zlib_compress` test helper (private to that module's tests, not
    /// shared) rather than reaching across module boundaries for it.
    fn zlib_compress(input: &[u8]) -> Vec<u8> {
        use flate2::{Compress, Compression, FlushCompress};
        let mut compressed = vec![0u8; input.len() + 1024];
        let mut compressor = Compress::new(Compression::default(), true);
        let status = compressor
            .compress(input, &mut compressed, FlushCompress::Finish)
            .unwrap();
        assert_eq!(status, flate2::Status::StreamEnd, "fixture must be a complete zlib stream");
        let len = compressor.total_out() as usize;
        compressed.truncate(len);
        compressed
    }

    #[tokio::test]
    async fn proxy_shaped_reader_decompresses_mccp2_and_still_closes_cleanly() {
        let mut h = Harness::spawn(TelnetTarget::World("proxyworld".to_string()));

        // IAC WILL MCCP2 - the server offers compression; we accept.
        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]).await;
        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(bytes, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]);
            }
            _ => panic!("expected a Raw wire write"),
        }
        assert!(matches!(h.next_event().await, AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "proxyworld"));

        // IAC SB MCCP2 IAC SE (compression starts) immediately followed by a real
        // zlib-compressed payload, exactly as a live MCCP2 MUD sends it.
        let plaintext = b"Room: The Proxy Chamber\r\n";
        let compressed = zlib_compress(plaintext);
        let mut activation = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE];
        activation.extend_from_slice(&compressed);
        h.send(&activation).await;

        // Job 10a: CompressionStarted/CompressionEnded now forward through
        // AppEvent::Telnet on World (they used to map to None) so
        // App::handle_telnet_event can maintain World::mccp2_active for the reload
        // guard. Both fire here, back to back: `zlib_compress`'s `FlushCompress::Finish`
        // produces a *complete*, self-terminating zlib stream (matching telnet.rs's own
        // fixture helper), so the decompressor sees `Z_STREAM_END` in this same `feed`
        // call right after activation - CompressionStarted then CompressionEnded, both
        // ahead of the decompressed text below.
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionStarted) if n == "proxyworld"
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionEnded) if n == "proxyworld"
        ));

        // Before Job 5 this proxy-shaped loop had no MCCP2 branch at all: these bytes
        // would have arrived as-is, rendered as binary garbage. Now they must arrive
        // decompressed.
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "proxyworld" && b == plaintext.to_vec()
        ));

        // Job 4's close-message unification still fires after a compressed connection's
        // EOF - the proxy loops used to skip this message entirely.
        h.close_server();
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "proxyworld" && b == b"Connection closed by server.\n".to_vec()
        ));
        assert!(matches!(h.next_event().await, AppEvent::Disconnected(n, _) if n == "proxyworld"));
    }

    // ==================================================================
    // Job 11 (plan Phase 3, step 3.5, finding 5): spawn_telnet_reader sends
    // TelnetSession::initial_negotiation()'s bytes at task start, gated on
    // TelnetConfig::initiate_negotiation (the per-world setting).
    // ==================================================================

    #[test]
    fn test_cfg_disables_initiate_negotiation_by_default() {
        // Every other test in this module relies on this: if it ever flips back to
        // TelnetConfig::default()'s `true`, those tests would start racing the
        // unprompted opening block against their own canned-script assertions.
        assert!(!test_cfg().initiate_negotiation);
    }

    #[tokio::test]
    async fn spawn_telnet_reader_sends_opening_negotiation_before_any_data_arrives() {
        // Production's own default (TelnetConfig::default(), not test_cfg()): the
        // reader's very first wire write, with nothing sent from the "server" side at
        // all yet, must be Clay's opening offer (MSSP added Job 12, plan Phase 4, 4.2;
        // MSP added Job 14, same phase - offered last, conditional on msp_enabled,
        // default true).
        let mut h = Harness::spawn_with_cfg(
            TelnetTarget::World("negotiateworld".to_string()),
            1,
            TelnetConfig::default(),
        );

        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(
                    bytes,
                    vec![
                        TELNET_IAC, TELNET_WILL, TELNET_OPT_TTYPE,
                        TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS,
                        TELNET_IAC, TELNET_DO, TELNET_OPT_CHARSET,
                        TELNET_IAC, TELNET_DO, TELNET_OPT_GMCP,
                        TELNET_IAC, TELNET_DO, TELNET_OPT_MSDP,
                        TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2,
                        TELNET_IAC, TELNET_DO, TELNET_OPT_MSSP,
                        TELNET_IAC, TELNET_DO, TELNET_OPT_MSP,
                    ]
                );
            }
            other => panic!("expected the opening negotiation as a Raw wire write, got {other:?}"),
        }

        // The reader still works normally afterward - the opening offer doesn't
        // interfere with parsing whatever the server sends next. Plain text with no
        // IAC byte at all correctly does not fire TelnetDetected (that only fires once
        // an actual IAC arrives *from the peer* - initial_negotiation is purely
        // outbound and doesn't touch that flag).
        h.send(b"Welcome\r\n").await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "negotiateworld" && b == b"Welcome\r\n".to_vec()
        ));
    }

    #[tokio::test]
    async fn initiate_negotiation_false_suppresses_the_opening_bytes() {
        // The plan's required test: the per-world setting actually suppresses the
        // opening offer when off, rather than merely defaulting off in tests.
        let mut h = Harness::spawn_with_cfg(
            TelnetTarget::World("quietworld".to_string()),
            1,
            TelnetConfig { initiate_negotiation: false, ..TelnetConfig::default() },
        );

        // Nothing to drain: the very first thing to arrive must be the reply to the
        // server's own script, not an unprompted offer from Clay.
        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA]).await;
        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(
                    bytes,
                    vec![TELNET_IAC, TELNET_DO, TELNET_OPT_SGA],
                    "with initiate_negotiation off, the only wire write should be the \
                     reply to the server's own WILL SGA - no opening offer preceding it"
                );
            }
            other => panic!("expected a Raw wire write, got {other:?}"),
        }
    }
}
