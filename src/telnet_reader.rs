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

/// How many idle-flush prompts one connection may report (see `idle_prompt_event`).
/// Auto-login uses at most three; the rest is slack for a re-prompt.
const IDLE_PROMPT_BUDGET: u32 = 6;

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

/// Wrap `ev` as an `AppEvent::Telnet(target.clone(), ev)` - the "just forward it,
/// `App`/`ProtocolState` will decide what to do" case both `to_app_event` match arms
/// below use for the majority of `TelnetEvent` variants (plan Job 9, T3.2). Extracted so
/// the `World` and `Multiuser` arms are provably identical wherever they both forward,
/// rather than two hand-copied literals that could drift.
fn forward_via_telnet(target: &TelnetTarget, ev: TelnetEvent) -> Option<AppEvent> {
    Some(AppEvent::Telnet(target.clone(), ev))
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
/// Job 9 (T3.2) closed the gap this doc comment used to describe at length: before it,
/// every `Multiuser` arm below `TelnetDetected`/`Prompt` mapped to `None`, because
/// `App::handle_telnet_event`'s `world_idx`-shaped signature had nowhere to route a
/// multiuser per-user connection's protocol events, and `run_multiuser_server` had no
/// `AppEvent::Telnet` arm at all — NAWS, TTYPE/MTTS, CHARSET, GMCP (including the
/// `Core.Hello` that makes servers send anything), MSDP, MSSP, MSP, and ECHO masking
/// (**passwords shown in cleartext**) were all dead for every multiuser user. Both
/// targets now forward every variant they don't have a more specific `AppEvent` for
/// through `forward_via_telnet` above, with the identical `OptionEnabled`/
/// `OptionDisabled` option-code filter (see below) — the *only* difference between the
/// two branches is which `AppEvent` shape `TelnetDetected`/`Prompt` produce
/// (`AppEvent::Telnet`/`AppEvent::Prompt` for `World`; `AppEvent::MultiuserTelnetDetected`/
/// `AppEvent::MultiuserPrompt`, carrying `world_index`+`username` instead of a world
/// name, for `Multiuser`). `App::handle_telnet_event` (`World`) and
/// `App::handle_multiuser_telnet_event` (`Multiuser`) both resolve their target to a
/// `ProtocolState` (`World::protocol` / `UserConnection::protocol`) and call the same
/// `ProtocolState::apply_telnet_event` core - see that function's doc comment for the
/// full per-event behaviour, which this mapper no longer needs to describe since it is
/// now identical on both targets.
///
/// One remaining class of `None`, on both targets: `ProtocolError` is informational
/// only — nothing downstream needs an `AppEvent` for it. It is still surfaced —
/// `spawn_telnet_reader` logs it via `debug_log` directly, since there is no
/// `AppEvent::Telnet`-carried payload built for it here.
/// Build an `AppEvent::IdlePrompt` for a trailing partial line released by the idle
/// flush, or `None` if this release is not prompt-shaped or the budget is spent.
///
/// Prompt-shaped means the release does not end in a newline: the server sent text and
/// then stopped mid-line. Only the part after the last newline is the prompt — an idle
/// flush can release several complete lines plus a trailing fragment in one go, and it is
/// only the fragment that is the prompt (this mirrors `extract_prompt`'s "text from the
/// last newline" rule in telnet.rs, so both prompt paths carve the same text).
///
/// A leading `\r` is trimmed: Aardwolf terminates lines with `\n\r` rather than `\r\n`,
/// so the fragment after the last `\n` would otherwise start with a stray carriage return.
///
/// Only `TelnetTarget::World` reports these. The multiuser path has its own auto-login
/// flow and is deliberately left alone here.
fn idle_prompt_event(
    target: &TelnetTarget,
    text: &[u8],
    budget: &mut u32,
) -> Option<AppEvent> {
    if *budget == 0 || text.last() == Some(&b'\n') {
        return None;
    }
    let TelnetTarget::World(name) = target else { return None };
    let start = text.iter().rposition(|&b| b == b'\n').map(|p| p + 1).unwrap_or(0);
    let fragment: &[u8] = text[start..]
        .strip_prefix(b"\r")
        .unwrap_or(&text[start..]);
    if fragment.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    *budget -= 1;
    Some(AppEvent::IdlePrompt(name.clone(), fragment.to_vec()))
}

pub fn to_app_event(target: &TelnetTarget, ev: TelnetEvent) -> Option<AppEvent> {
    match target {
        TelnetTarget::World(name) => match ev {
            TelnetEvent::TelnetDetected => forward_via_telnet(target, TelnetEvent::TelnetDetected),
            TelnetEvent::Prompt(bytes) => Some(AppEvent::Prompt(name.clone(), bytes)),
            TelnetEvent::NawsRequested => forward_via_telnet(target, TelnetEvent::NawsRequested),
            TelnetEvent::TtypeRequested => forward_via_telnet(target, TelnetEvent::TtypeRequested),
            TelnetEvent::CharsetRequest(charsets) => {
                forward_via_telnet(target, TelnetEvent::CharsetRequest(charsets))
            }
            TelnetEvent::GmcpMessage(package, json) => {
                forward_via_telnet(target, TelnetEvent::GmcpMessage(package, json))
            }
            TelnetEvent::MsdpVariable(variable, value) => {
                forward_via_telnet(target, TelnetEvent::MsdpVariable(variable, value))
            }
            TelnetEvent::MsspData(pairs) => forward_via_telnet(target, TelnetEvent::MsspData(pairs)),
            TelnetEvent::CompressionStarted => forward_via_telnet(target, TelnetEvent::CompressionStarted),
            TelnetEvent::CompressionEnded => forward_via_telnet(target, TelnetEvent::CompressionEnded),
            TelnetEvent::CompressionFailed(reason) => {
                forward_via_telnet(target, TelnetEvent::CompressionFailed(reason))
            }
            // No AppEvent::Telnet payload built for this — the reader logs
            // it directly instead of mapping it.
            TelnetEvent::ProtocolError(_) => None,
            TelnetEvent::OptionEnabled(opt) => match opt {
                // The two option codes the core consumes today (Core.Hello/
                // Core.Supports.Set announcement, LIST REPORTABLE_VARIABLES) — losing
                // either silently breaks GMCP announcement or the
                // gmcp_enabled/msdp_enabled mirrors.
                TELNET_OPT_GMCP | TELNET_OPT_MSDP => {
                    forward_via_telnet(target, TelnetEvent::OptionEnabled(opt))
                }
                // Negotiating successfully has no action of its own for these —
                // NawsRequested/TtypeRequested/CharsetRequest/MsspData already cover
                // the actions they unlock, and MSP's in-band `!!SOUND(...)`/
                // `!!MUSIC(...)` triggers work whether or not option 90 ever
                // negotiates (see `msp_enabled`'s doc comment).
                TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_NAWS | TELNET_OPT_TTYPE
                | TELNET_OPT_CHARSET | TELNET_OPT_MCCP2 | TELNET_OPT_MSSP | TELNET_OPT_MSP => None,
                // TelnetSession never actually emits OptionEnabled for any
                // other code (see its WILL/DO accept lists) - unreachable in
                // practice, but the u8 payload still needs a catch-all.
                _ => None,
            },
            // Finding 4: the three options with a mirror to clear forward through;
            // everything else nothing reacts to, same as OptionEnabled.
            TelnetEvent::OptionDisabled(opt) => match opt {
                TELNET_OPT_GMCP | TELNET_OPT_MSDP | TELNET_OPT_NAWS => {
                    forward_via_telnet(target, TelnetEvent::OptionDisabled(opt))
                }
                // MSSP has no mirror to clear on disable beyond mssp_data itself, which
                // ProtocolState::clear handles independently of this event. MSP has no
                // mirror at all to clear - msp_enabled is a TelnetConfig/WorldSettings
                // toggle, not a negotiated-state flag.
                TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_TTYPE | TELNET_OPT_CHARSET
                | TELNET_OPT_MCCP2 | TELNET_OPT_MSSP | TELNET_OPT_MSP => None,
                _ => None,
            },
            TelnetEvent::WontEchoPromptHint => forward_via_telnet(target, TelnetEvent::WontEchoPromptHint),
            TelnetEvent::EchoOff => forward_via_telnet(target, TelnetEvent::EchoOff),
            TelnetEvent::EchoOn => forward_via_telnet(target, TelnetEvent::EchoOn),
            TelnetEvent::MspTrigger(trigger) => forward_via_telnet(target, TelnetEvent::MspTrigger(trigger)),
        },
        TelnetTarget::Multiuser { world_index, username } => match ev {
            TelnetEvent::TelnetDetected => {
                Some(AppEvent::MultiuserTelnetDetected(*world_index, username.clone()))
            }
            TelnetEvent::Prompt(bytes) => {
                Some(AppEvent::MultiuserPrompt(*world_index, username.clone(), bytes))
            }
            // Job 9 (T3.2): every variant below now forwards exactly like the World arm
            // above (identical OptionEnabled/OptionDisabled code filter included) -
            // App::handle_multiuser_telnet_event resolves target into a
            // UserConnection::protocol and calls the same ProtocolState core.
            TelnetEvent::NawsRequested => forward_via_telnet(target, TelnetEvent::NawsRequested),
            TelnetEvent::TtypeRequested => forward_via_telnet(target, TelnetEvent::TtypeRequested),
            TelnetEvent::CharsetRequest(charsets) => {
                forward_via_telnet(target, TelnetEvent::CharsetRequest(charsets))
            }
            TelnetEvent::GmcpMessage(package, json) => {
                forward_via_telnet(target, TelnetEvent::GmcpMessage(package, json))
            }
            TelnetEvent::MsdpVariable(variable, value) => {
                forward_via_telnet(target, TelnetEvent::MsdpVariable(variable, value))
            }
            TelnetEvent::MsspData(pairs) => forward_via_telnet(target, TelnetEvent::MsspData(pairs)),
            TelnetEvent::CompressionStarted => forward_via_telnet(target, TelnetEvent::CompressionStarted),
            TelnetEvent::CompressionEnded => forward_via_telnet(target, TelnetEvent::CompressionEnded),
            TelnetEvent::CompressionFailed(reason) => {
                forward_via_telnet(target, TelnetEvent::CompressionFailed(reason))
            }
            TelnetEvent::ProtocolError(_) => None,
            TelnetEvent::OptionEnabled(opt) => match opt {
                TELNET_OPT_GMCP | TELNET_OPT_MSDP => {
                    forward_via_telnet(target, TelnetEvent::OptionEnabled(opt))
                }
                TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_NAWS | TELNET_OPT_TTYPE
                | TELNET_OPT_CHARSET | TELNET_OPT_MCCP2 | TELNET_OPT_MSSP | TELNET_OPT_MSP => None,
                _ => None,
            },
            TelnetEvent::OptionDisabled(opt) => match opt {
                TELNET_OPT_GMCP | TELNET_OPT_MSDP | TELNET_OPT_NAWS => {
                    forward_via_telnet(target, TelnetEvent::OptionDisabled(opt))
                }
                TELNET_OPT_SGA | TELNET_OPT_EOR | TELNET_OPT_TTYPE | TELNET_OPT_CHARSET
                | TELNET_OPT_MCCP2 | TELNET_OPT_MSSP | TELNET_OPT_MSP => None,
                _ => None,
            },
            TelnetEvent::WontEchoPromptHint => forward_via_telnet(target, TelnetEvent::WontEchoPromptHint),
            TelnetEvent::EchoOff => forward_via_telnet(target, TelnetEvent::EchoOff),
            TelnetEvent::EchoOn => forward_via_telnet(target, TelnetEvent::EchoOn),
            // MspTrigger still forwards (the handler, not the reader, must own the
            // "no per-connection host to play audio on" decision - plan Job 9's
            // per-variant table) even though App::handle_multiuser_telnet_event's
            // core call is followed by an explicit no-op for it.
            TelnetEvent::MspTrigger(trigger) => forward_via_telnet(target, TelnetEvent::MspTrigger(trigger)),
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
/// every read, that releases `TelnetSession::take_idle_flushable_text()`
/// after `IDLE_FLUSH_INTERVAL` of silence. Without this, a prompt on a server
/// that never sends GA/EOR (MUSH/MOO) would sit invisibly in `pending_text`
/// until more output happened to arrive. T2.2: unlike the old
/// `take_pending_text`, this does not necessarily drain everything — an
/// in-progress MSP trigger, an unterminated ANSI escape, or a truncated UTF-8
/// code point stays held (see `idle_release_len`), so a following read or a
/// later idle flush can still complete it correctly instead of it having
/// already been shown broken.
pub fn spawn_telnet_reader(
    mut read_half: StreamReader,
    cmd_tx: mpsc::Sender<WriteCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    target: TelnetTarget,
    conn_id: u64,
    cfg: TelnetConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let wants_idle_prompt = cfg.wants_prompt_auto_login;
        let mut session = TelnetSession::new(cfg);
        let mut buffer = vec![0u8; READ_BUFFER_SIZE];

        let idle_timer = tokio::time::sleep(FAR_FUTURE);
        tokio::pin!(idle_timer);
        // Bounds how many idle-flush prompts this connection will ever report. Auto-login
        // consumes at most three (MooPrompt), and a couple spare covers a mistyped
        // password being re-prompted; after that the reader stops reporting entirely, so a
        // long session cannot keep paying for events App would only discard.
        let mut idle_prompt_budget: u32 = IDLE_PROMPT_BUDGET;

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

                            // T1.1/T1.14/D1: a poisoned MCCP2 stream is fatal - noted here
                            // (rather than acted on inline) so the ordinary per-event
                            // forwarding below still runs first, exactly like every other
                            // event this call produced.
                            let mut compression_failed: Option<String> = None;
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
                                if let TelnetEvent::CompressionFailed(ref reason) = ev {
                                    crate::debug_log(true, &format!(
                                        "telnet_reader ({target:?}): MCCP2 stream corrupt: {reason}"
                                    ));
                                    compression_failed = Some(reason.clone());
                                }
                                if let Some(app_ev) = to_app_event(&target, ev) {
                                    let _ = event_tx.send(app_ev).await;
                                }
                            }

                            if !outcome.text.is_empty() {
                                let _ = event_tx.send(make_server_data_event(&target, outcome.text)).await;
                            }

                            if let Some(reason) = compression_failed {
                                // D1: never fall back to parsing a poisoned stream as
                                // plaintext - disconnect with one clear message, the same
                                // fatal sequence the Ok(0)/clean-EOF arm above uses: flush
                                // whatever text was still held back, then the message,
                                // then Disconnected, then return.
                                let eof = session.flush_eof();
                                if !eof.text.is_empty() {
                                    let _ = event_tx.send(make_server_data_event(&target, eof.text)).await;
                                }
                                let msg = format!("MCCP2 stream corrupt ({reason}); disconnecting.\n");
                                let _ = event_tx.send(make_server_data_event(&target, msg.into_bytes())).await;
                                let _ = event_tx.send(make_disconnected_event(&target, conn_id)).await;
                                return;
                            }

                            // Mandatory idle flush: reset on every read (see
                            // IDLE_FLUSH_INTERVAL's doc comment / the plan's
                            // "Hard requirement discovered in Job 2").
                            idle_timer.as_mut().reset(tokio::time::Instant::now() + IDLE_FLUSH_INTERVAL);
                        }
                        Err(e) => {
                            // T1.15: flush whatever text was still held back before
                            // reporting the error, exactly like the Ok(0) clean-EOF arm
                            // above already does - otherwise the last prompt/line is lost
                            // on any read error.
                            let eof = session.flush_eof();
                            if !eof.text.is_empty() {
                                let _ = event_tx.send(make_server_data_event(&target, eof.text)).await;
                            }
                            let msg = format!("Read error: {}", e);
                            let _ = event_tx.send(make_server_data_event(&target, msg.into_bytes())).await;
                            let _ = event_tx.send(make_disconnected_event(&target, conn_id)).await;
                            return;
                        }
                    }
                }
                _ = &mut idle_timer => {
                    let text = session.take_idle_flushable_text();
                    if !text.is_empty() {
                        // A release that does NOT end in a newline is a trailing partial
                        // line the server stopped mid-way through — i.e. a prompt on a MUD
                        // that never sends GA/EOR (see IDLE_FLUSH_INTERVAL's doc comment,
                        // which is why this flush exists at all). Report it so auto-login
                        // can advance; App applies the real gating and ignores it outside
                        // the login window. The text is still emitted below exactly as
                        // before, so display is untouched.
                        let prompt_ev = if wants_idle_prompt {
                            idle_prompt_event(&target, &text, &mut idle_prompt_budget)
                        } else {
                            None
                        };
                        let _ = event_tx.send(make_server_data_event(&target, text)).await;
                        // Emitted *after* the text, so the existing event ordering every
                        // other consumer already relies on is untouched; this only adds a
                        // trailing trigger.
                        if let Some(ev) = prompt_ev {
                            let _ = event_tx.send(ev).await;
                        }
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

    /// Default test config. `spawn_telnet_reader` is fully reactive — it never sends
    /// anything unprompted — so every test in this module can safely use
    /// `TelnetConfig::default()` without racing an opening offer.
    fn test_cfg() -> TelnetConfig {
        TelnetConfig::default()
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

        /// Like `spawn_with_conn_id`, but with an explicit `TelnetConfig` — used by
        /// tests that need a non-default config (e.g. `msp_enabled`, `is_tls`).
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
        // Plan Job 1, T1.1/T1.14/D1: forwards the same way, so
        // App::handle_telnet_event can clear World::mccp2_active.
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CompressionFailed("corrupt".to_string())),
            Some(AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionFailed(r)))
                if n == "w" && r == "corrupt"
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

    /// Job 9 (T3.2): rewritten from a "drops everything" table into a "forwards
    /// everything" table — every variant below `TelnetDetected`/`Prompt` (which keep
    /// their own `(usize, String)`-shaped `AppEvent`s) now maps to
    /// `Some(Telnet(Multiuser{3,"bob"}, ev))`, exactly like the `World` target above,
    /// including the identical `OptionEnabled`/`OptionDisabled` option-code filter.
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
        assert!(matches!(
            to_app_event(&t, TelnetEvent::NawsRequested),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, ref username }, TelnetEvent::NawsRequested)) if username == "bob"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::TtypeRequested),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::TtypeRequested))
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CharsetRequest(vec!["UTF-8".to_string()])),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::CharsetRequest(cs)))
                if cs == vec!["UTF-8".to_string()]
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::GmcpMessage("Core.Hello".to_string(), "{}".to_string())),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::GmcpMessage(p, j)))
                if p == "Core.Hello" && j == "{}"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::MsdpVariable("HP".to_string(), "100".to_string())),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::MsdpVariable(v, val)))
                if v == "HP" && val == "100"
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::MsspData(vec![("NAME".to_string(), "Test MUD".to_string())])),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::MsspData(pairs)))
                if pairs == vec![("NAME".to_string(), "Test MUD".to_string())]
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CompressionStarted),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::CompressionStarted))
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CompressionEnded),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::CompressionEnded))
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::CompressionFailed("corrupt".to_string())),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::CompressionFailed(r)))
                if r == "corrupt"
        ));
        // No AppEvent::Telnet payload built for ProtocolError, same as World.
        assert!(to_app_event(&t, TelnetEvent::ProtocolError("boom".to_string())).is_none());

        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionEnabled(TELNET_OPT_GMCP)),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::OptionEnabled(opt)))
                if opt == TELNET_OPT_GMCP
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionEnabled(TELNET_OPT_MSDP)),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::OptionEnabled(opt)))
                if opt == TELNET_OPT_MSDP
        ));
        for opt in [
            TELNET_OPT_SGA, TELNET_OPT_EOR, TELNET_OPT_NAWS, TELNET_OPT_TTYPE,
            TELNET_OPT_CHARSET, TELNET_OPT_MCCP2, TELNET_OPT_MSSP,
        ] {
            assert!(to_app_event(&t, TelnetEvent::OptionEnabled(opt)).is_none());
        }
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_GMCP)),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::OptionDisabled(opt)))
                if opt == TELNET_OPT_GMCP
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_MSDP)),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::OptionDisabled(opt)))
                if opt == TELNET_OPT_MSDP
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::OptionDisabled(TELNET_OPT_NAWS)),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::OptionDisabled(opt)))
                if opt == TELNET_OPT_NAWS
        ));
        for opt in [
            TELNET_OPT_SGA, TELNET_OPT_EOR, TELNET_OPT_TTYPE, TELNET_OPT_CHARSET, TELNET_OPT_MCCP2, TELNET_OPT_MSSP,
        ] {
            assert!(to_app_event(&t, TelnetEvent::OptionDisabled(opt)).is_none());
        }
        assert!(matches!(
            to_app_event(&t, TelnetEvent::WontEchoPromptHint),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::WontEchoPromptHint))
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::EchoOff),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::EchoOff))
        ));
        assert!(matches!(
            to_app_event(&t, TelnetEvent::EchoOn),
            Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 3, .. }, TelnetEvent::EchoOn))
        ));
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
    async fn multiuser_target_forwards_events_like_world_does() {
        // Job 9 (T3.2): inverted from "drops unsupported events" - after this job
        // NawsRequested (and every other variant below TelnetDetected/Prompt) *is*
        // forwarded on a Multiuser target, the exact opposite of what this test used to
        // assert.
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

        // TelnetDetected keeps its own (usize, String)-shaped AppEvent...
        assert!(matches!(
            h.next_event().await,
            AppEvent::MultiuserTelnetDetected(5, ref u) if u == "alice"
        ));
        // ...but NawsRequested now IS forwarded - the next event is
        // Telnet(Multiuser{5,"alice"}, NawsRequested), not the Prompt.
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 5, ref username }, TelnetEvent::NawsRequested)
                if username == "alice"
        ));
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

    /// A `WILL GMCP` clone of `will_gmcp_emits_gmcp_negotiated` below, for the
    /// `Multiuser` target (plan Job 9's test list): GMCP negotiation now reaches a
    /// multiuser connection the same way it reaches a `World` one.
    #[tokio::test]
    async fn multiuser_will_gmcp_emits_option_enabled() {
        let mut h = Harness::spawn(TelnetTarget::Multiuser {
            world_index: 7,
            username: "carol".to_string(),
        });
        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP]).await;

        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(bytes, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_GMCP]);
            }
            _ => panic!("expected a Raw wire write"),
        }
        assert!(matches!(
            h.next_event().await,
            AppEvent::MultiuserTelnetDetected(7, ref u) if u == "carol"
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 7, ref username }, TelnetEvent::OptionEnabled(opt))
                if username == "carol" && opt == TELNET_OPT_GMCP
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

    /// A MUD that never sends GA/EOR (Aardwolf's login prompt is the reference case)
    /// must still drive auto-login. The idle flush already released the text so the
    /// prompt is visible; this asserts it now also reports an `IdlePrompt` so
    /// `prompt_count` can advance. The ServerData still follows — display is unchanged.
    #[tokio::test(start_paused = true)]
    async fn idle_flush_reports_unmarked_prompt_for_auto_login() {
        // Only a prompt-auto-login world reports these at all.
        let cfg = TelnetConfig { wants_prompt_auto_login: true, ..test_cfg() };
        let mut h = Harness::spawn_with_cfg(TelnetTarget::World("w".to_string()), 1, cfg);
        h.send(b"What be thy name, adventurer? ").await;
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;

        // The text still comes first and unchanged — display behaviour is untouched.
        let ev = h.next_event().await;
        assert!(
            matches!(&ev, AppEvent::ServerData(n, _) if n == "w"),
            "the idle flush must still emit the text as output, first and unchanged"
        );
        assert!(matches!(
            h.next_event().await,
            AppEvent::IdlePrompt(n, b)
                if n == "w" && b == b"What be thy name, adventurer? ".to_vec()
        ), "an unmarked trailing partial line must then be reported for auto-login");
    }

    // `idle_prompt_event` is pure, so its rules are tested directly rather than by
    // driving the reader — the reader emits a complete line the moment it arrives, so
    // event ordering there depends on how the input happens to be split.

    #[test]
    fn idle_prompt_event_takes_only_the_trailing_fragment_and_trims_cr() {
        let t = TelnetTarget::World("w".to_string());
        let mut budget = 3;
        // Aardwolf terminates lines with \n\r, so the fragment after the last \n would
        // otherwise carry a leading carriage return.
        let ev = idle_prompt_event(&t, b"banner\n\rWhat be thy name? ", &mut budget);
        assert!(matches!(ev, Some(AppEvent::IdlePrompt(ref n, ref b))
            if n == "w" && b == b"What be thy name? "), "only the fragment, CR trimmed");
    }

    #[test]
    fn idle_prompt_event_ignores_a_newline_terminated_release() {
        let t = TelnetTarget::World("w".to_string());
        let mut budget = 3;
        assert!(idle_prompt_event(&t, b"ordinary output\n", &mut budget).is_none());
        assert_eq!(budget, 3, "a non-prompt must not spend budget");
    }

    #[test]
    fn idle_prompt_event_ignores_whitespace_only_fragment() {
        let t = TelnetTarget::World("w".to_string());
        let mut budget = 3;
        assert!(idle_prompt_event(&t, b"line\n   ", &mut budget).is_none());
        assert_eq!(budget, 3);
    }

    #[test]
    fn idle_prompt_event_is_bounded_by_budget() {
        let t = TelnetTarget::World("w".to_string());
        let mut budget = 2;
        assert!(idle_prompt_event(&t, b"a> ", &mut budget).is_some());
        assert!(idle_prompt_event(&t, b"b> ", &mut budget).is_some());
        assert!(idle_prompt_event(&t, b"c> ", &mut budget).is_none(),
            "reporting stops once the per-connection budget is spent");
    }

    #[test]
    fn idle_prompt_event_skips_multiuser_target() {
        let t = TelnetTarget::Multiuser { world_index: 0, username: "u".to_string() };
        let mut budget = 3;
        assert!(idle_prompt_event(&t, b"Login: ", &mut budget).is_none(),
            "multiuser has its own auto-login flow and is left alone here");
    }

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
    // T2.2: the idle flush must not release text the session isn't sure is
    // safe yet - an in-progress ANSI CSI, an incomplete UTF-8 code point, or
    // an open MSP trigger marker all stay held past the idle flush and get
    // completed by whatever arrives next, rather than being shown broken (or,
    // for MSP, losing the trigger) after 150ms of silence.
    // ==================================================================

    #[tokio::test(start_paused = true)]
    async fn idle_flush_holds_back_incomplete_csi_across_two_sends() {
        let mut h = Harness::spawn(TelnetTarget::World("w".to_string()));

        h.send(b"prompt> \x1b[3").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            matches!(h.event_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "nothing should be released before the idle flush fires"
        );
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"prompt> ".to_vec()
        ), "the incomplete CSI must stay held while the prompt text in front of it is released");

        // The rest of the escape arrives on the next read; the idle timer was
        // re-armed by that read, so a second idle period completes it.
        h.send(b"1m").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"\x1b[31m".to_vec()
        ), "no stray '[3' - the completed escape sequence must come out as one whole unit");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_flush_holds_back_incomplete_utf8_across_two_sends() {
        let mut h = Harness::spawn(TelnetTarget::World("w".to_string()));

        h.send(b"caf\xc3").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(h.event_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"caf".to_vec()
        ), "the truncated UTF-8 lead byte must stay held while 'caf' is released");

        h.send(b"\xa9").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"\xc3\xa9".to_vec()
        ), "the completed code point must come out as one whole unit");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_flush_holds_back_open_msp_marker_across_two_sends() {
        let mut h = Harness::spawn(TelnetTarget::World("w".to_string()));

        h.send(b"Look !!SOUND(bell").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(h.event_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"Look ".to_vec()
        ), "the open marker (and 'bell', its in-progress filename) must stay held");

        // The rest of the trigger arrives on the next read: the marker
        // resolves into an MspTrigger event, and the trailing "\r\n" (with
        // nothing left to hold back) is emitted immediately - no idle wait
        // needed for it.
        h.send(b".wav)\r\n").await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::MspTrigger(t))
                if n == "w" && t.name == "bell.wav" && !t.is_music
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"\r\n".to_vec()
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn idle_flush_releases_ordinary_prompt_ending_in_bang_whole() {
        // A lone trailing '!' is only held back by feed() itself (which has
        // a next read that might still turn it into "!!SOUND("); after an
        // idle period with no such read coming, msp_holdback_start's
        // min_prefix of 2 lets it through so an ordinary prompt like this
        // isn't withheld forever.
        let mut h = Harness::spawn(TelnetTarget::World("w".to_string()));

        h.send(b"Hello!").await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(h.event_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        tokio::time::advance(IDLE_FLUSH_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"Hello!".to_vec()
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
    // Plan Job 1 (investigate-differences-between-tinyfugu-fluffy-stallman.md),
    // T1.1/T1.2/T1.14/D1 — the reader's fatal disconnect sequence for a poisoned
    // MCCP2 stream.
    // ==================================================================

    #[tokio::test]
    async fn world_target_disconnects_on_mccp2_garbage_after_activation() {
        // D1: a poisoned zlib stream disconnects with one clear message - it never falls
        // back to parsing raw compressed bytes as telnet.
        let mut h = Harness::spawn(TelnetTarget::World("corruptworld".to_string()));

        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_MCCP2]).await;
        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => assert_eq!(bytes, vec![TELNET_IAC, TELNET_DO, TELNET_OPT_MCCP2]),
            _ => panic!("expected a Raw wire write"),
        }
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "corruptworld"
        ));

        let mut activation = vec![TELNET_IAC, TELNET_SB, TELNET_OPT_MCCP2, TELNET_IAC, TELNET_SE];
        activation.extend_from_slice(&[0xAAu8; 64]); // not a valid zlib header
        h.send(&activation).await;

        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionStarted) if n == "corruptworld"
        ));
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::CompressionFailed(_)) if n == "corruptworld"
        ));
        match h.next_event().await {
            AppEvent::ServerData(n, b) => {
                assert_eq!(n, "corruptworld");
                let text = String::from_utf8_lossy(&b);
                assert!(text.contains("MCCP2 stream corrupt"), "got: {text}");
                assert!(text.contains("disconnecting"), "got: {text}");
            }
            _ => panic!("expected ServerData with the corruption message"),
        }
        assert!(matches!(h.next_event().await, AppEvent::Disconnected(n, _) if n == "corruptworld"));

        // The reader task returned: the channel closes, nothing more is ever sent.
        assert!(h.event_rx.recv().await.is_none(), "reader task must have returned");
    }

    #[tokio::test]
    async fn err_arm_flushes_pending_text_before_reporting_the_read_error() {
        // T1.15: the Err(e) arm must flush_eof() before reporting the error, exactly like
        // the Ok(0) clean-EOF arm already does - otherwise a held-back prompt/line is lost
        // on any read error. "Login: " has no trailing newline and no GA/EOR/WONT-ECHO
        // boundary, so it sits in pending_text (not emitted as ServerData yet) until the
        // Err arm's flush_eof() releases it.
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(Ok(b"Login: ".to_vec()));
        queue.push_back(Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset",
        )));

        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let _handle = spawn_telnet_reader(
            StreamReader::Scripted(queue),
            cmd_tx,
            event_tx,
            TelnetTarget::World("w".to_string()),
            1,
            test_cfg(),
        );

        async fn next(rx: &mut mpsc::Receiver<AppEvent>) -> AppEvent {
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timed out waiting for AppEvent")
                .expect("event channel closed")
        }

        assert!(matches!(
            next(&mut event_rx).await,
            AppEvent::ServerData(n, b) if n == "w" && b == b"Login: ".to_vec()
        ));
        match next(&mut event_rx).await {
            AppEvent::ServerData(n, b) => {
                assert_eq!(n, "w");
                assert_eq!(String::from_utf8_lossy(&b), "Read error: connection reset");
            }
            _ => panic!("expected the read-error ServerData"),
        }
        assert!(matches!(next(&mut event_rx).await, AppEvent::Disconnected(n, _) if n == "w"));
    }

    // ==================================================================
    // Clay is fully reactive: spawn_telnet_reader must never send anything
    // unprompted, only in response to what the server sends first.
    // ==================================================================

    #[tokio::test]
    async fn spawn_telnet_reader_sends_nothing_unprompted() {
        // With nothing sent from the "server" side yet, the reader must not write
        // anything to the wire on its own - no opening offer, no unsolicited bytes at
        // all. Confirm this by having the server speak first and checking that the
        // reply to *that* is the very first (and only) wire write.
        let mut h = Harness::spawn_with_cfg(
            TelnetTarget::World("reactiveworld".to_string()),
            1,
            TelnetConfig::default(),
        );

        h.send(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA]).await;
        match h.next_cmd().await {
            WriteCommand::Raw(bytes) => {
                assert_eq!(
                    bytes,
                    vec![TELNET_IAC, TELNET_DO, TELNET_OPT_SGA],
                    "the reader's first wire write must be the reply to the server's own \
                     WILL SGA, not an unprompted offer preceding it"
                );
            }
            other => panic!("expected a Raw wire write, got {other:?}"),
        }

        // The IAC byte just seen also fires TelnetDetected - drain it before checking
        // for the plain text below.
        assert!(matches!(
            h.next_event().await,
            AppEvent::Telnet(TelnetTarget::World(n), TelnetEvent::TelnetDetected) if n == "reactiveworld"
        ));

        // The reader still works normally afterward - plain text with no IAC byte at
        // all is passed through untouched.
        h.send(b"Welcome\r\n").await;
        assert!(matches!(
            h.next_event().await,
            AppEvent::ServerData(n, b) if n == "reactiveworld" && b == b"Welcome\r\n".to_vec()
        ));
    }
}
