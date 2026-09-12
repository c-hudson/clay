//! Per-connection telnet protocol mirrors, extracted out of `World` (plan
//! `investigate-differences-between-tinyfugu-fluffy-stallman.md`, Job 9 / T3.2).
//!
//! Before this job, every one of these fields lived directly on `World`, which is why
//! multiuser mode — whose per-user live connection state lives on `UserConnection`, not
//! the shared `World` (see `UserConnection`'s doc comment) — had nowhere to put NAWS,
//! TTYPE/MTTS, CHARSET, GMCP, MSDP, MSSP, MSP, or ECHO-masking state for a connecting
//! user: `telnet_reader::to_app_event`'s `Multiuser` arm mapped fifteen of seventeen
//! `TelnetEvent` variants to `None`, and `run_multiuser_server` had no
//! `AppEvent::Telnet` arm at all. `daemon.rs:2965` reading `world.echo_masked` (always
//! `false` there) instead of a real per-connection mirror meant multiuser passwords
//! showed in cleartext.
//!
//! `ProtocolState` is embedded as `World::protocol` and `UserConnection::protocol`, and
//! `apply_telnet_event` is the *one* place every per-event rule is written — exhaustive
//! over `TelnetEvent` with no wildcard arm, mirroring the same anti-drift discipline
//! `telnet_reader::to_app_event` and `App::handle_telnet_event` already use. Two design
//! amendments (plan Job 9):
//!
//! 1. **The core returns its broadcasts** (`ProtocolOutcome`) rather than performing them.
//!    Performing them here would need a live `&App`/`&WebSocketServer`, which
//!    `ProtocolState` deliberately never holds - and more importantly, *scope* (every
//!    client, for a `World`; only the owner, for multiuser - CLAUDE.md's `world.owner ==
//!    username` rule) is a decision only the caller can make correctly.
//! 2. **The caller computes the NAWS size** (`ProtocolCtx::naws_size`) rather than the
//!    core reading it from anywhere shared: the `World` path's source is App-wide
//!    (`App::get_minimum_dimensions`), while the multiuser path's must be scoped to one
//!    user's own WS clients (`App::user_min_dimensions`) so one user's window size can
//!    never leak into another's NAWS.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::encoding::Encoding;
use crate::telnet::{
    build_charset_accepted, build_charset_rejected, build_gmcp_message, build_msdp_set,
    build_naws_subnegotiation, TelnetEvent, WriteCommand, TELNET_OPT_GMCP, TELNET_OPT_MSDP,
    TELNET_OPT_NAWS,
};
use crate::websocket::WsMessage;
use crate::{stats, WorldSettings, MAX_MSDP_AUTO_REPORT, VERSION};

/// The fifteen protocol mirrors moved off `World` (plan Job 9's field-move table).
///
/// Fields that stayed on `World` (not duplicated here): `active_media` (host-audio
/// restart map - meaningless without a host to restart audio on), `mirrored_stats`/
/// `stats_line_shown` (the console status line's own bookkeeping), `gmcp_user_enabled`
/// (a world-level UI toggle, not negotiated protocol state), `telnet_mode` (both `World`
/// and `UserConnection` own their own copy rather than sharing one), and `mcp` (fed by
/// raw text via `mcp::McpState::feed_line`, not a `TelnetEvent` at all).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProtocolState {
    /// Encoding negotiated via TELNET CHARSET (RFC 2066); `None` until a CHARSET
    /// exchange completes, in which case `effective_encoding` falls back to
    /// `WorldSettings::encoding`.
    pub negotiated_encoding: Option<Encoding>,
    /// True if this connection uses WONT ECHO as its prompt-boundary heuristic
    /// (auto-detected) - see `TelnetEvent::WontEchoPromptHint`'s doc comment.
    pub uses_wont_echo_prompt: bool,
    /// True once this connection has delivered a prompt via a real telnet marker
    /// (GA/EOR/WONT-ECHO). On such a world the marker is authoritative and the
    /// idle-flush prompt inference must stay out of the way: a trailing partial line
    /// there is ordinary mid-line output, not a prompt, and treating it as one
    /// overwrites the real prompt with whatever happened to arrive last — a bare
    /// colour reset after a prompt would replace `> ` with an invisible ANSI-only
    /// string. The inference exists only for MUDs that never mark their prompts.
    pub seen_prompt_marker: bool,
    /// True if NAWS was negotiated on this connection.
    pub naws_enabled: bool,
    /// Last window size actually sent via NAWS, to avoid resending an unchanged size.
    pub naws_sent_size: Option<(u16, u16)>,
    /// True if GMCP was negotiated with the server.
    pub gmcp_enabled: bool,
    /// True if MSDP was negotiated with the server.
    pub msdp_enabled: bool,
    /// True while an MCCP2 zlib stream is active on this connection - see `World`'s
    /// former doc comment on this field (preserved on `apply_telnet_event`'s
    /// `CompressionStarted`/`CompressionEnded`/`CompressionFailed` arms) for the
    /// hot-reload bailout this guards.
    pub mccp2_active: bool,
    /// True from `TelnetEvent::EchoOff` until `EchoOn`/`clear()` - the password-masking
    /// flag. This is the field T3.2's "passwords in cleartext" bug is about: before this
    /// job, multiuser had no equivalent of it at all.
    pub echo_masked: bool,
    /// GMCP packages this connection told the server it supports.
    pub gmcp_supported_packages: Vec<String>,
    /// MSDP variable -> JSON value.
    pub msdp_variables: HashMap<String, String>,
    /// GMCP package -> last JSON data.
    pub gmcp_data: HashMap<String, String>,
    /// Status display model, derived from `Char.*` GMCP packages and MSDP variables -
    /// see `stats::WorldStats`.
    pub stats: stats::WorldStats,
    /// Set whenever `stats` changes; cleared by the periodic dirty-stats flush once the
    /// current state has been broadcast (mud-status-display.md Job 2's coalescing).
    pub stats_dirty: bool,
    /// MSSP (option 70) server status as ordered name/value pairs - see
    /// `telnet::parse_mssp_pairs`.
    pub mssp_data: Vec<(String, String)>,
    /// MCMP default URL from `Client.Media.Default`.
    pub mcmp_default_url: String,
}

/// Everything `ProtocolState::apply_telnet_event` needs from its caller but must not own
/// itself (see this module's doc comment, amendments 1 and 2).
pub struct ProtocolCtx<'a> {
    /// This world's index in `App::worlds` - stamped onto every `WsMessage` the core
    /// builds (`GmcpData`, `MsdpData`, `McmpMedia`, `EchoMaskChanged`), all of which carry
    /// `world_index: usize`.
    pub world_index: usize,
    /// This connection's outbound telnet command channel, if currently connected. `None`
    /// means every wire-reply branch below silently sends nothing, exactly like the
    /// pre-Job-9 handlers already did when `command_tx` was `None`.
    pub command_tx: Option<&'a mpsc::Sender<WriteCommand>>,
    /// The owning world's settings - read-only here (encoding choice, `gmcp_packages`).
    pub settings: &'a WorldSettings,
    /// NAWS window size to report if `NawsRequested` needs answering right now. `None`
    /// means "no viewer has reported a size yet," matching `App::get_minimum_dimensions`'s
    /// existing `None` case.
    pub naws_size: Option<(u16, u16)>,
}

/// What `apply_telnet_event` produced, for the caller to route (amendment 1). The core
/// never touches `App`/`WebSocketServer` directly, so the same rule builds exactly one
/// `WsMessage` list regardless of whether the caller then fans it out to every client
/// (`App::ws_broadcast`, the `World` path) or just the owner
/// (`WebSocketServer::broadcast_to_owner`, the multiuser path).
#[derive(Default)]
pub struct ProtocolOutcome {
    pub broadcasts: Vec<WsMessage>,
    /// True when `stats`/`stats_dirty` changed as a result of this event. The `World`
    /// path uses this to set `App::needs_output_redraw` for the console status line
    /// (mud-status-display.md D4); multiuser has no console and does not read it - its
    /// per-user dirty flush is driven by `stats_dirty` itself, not this flag.
    pub stats_changed: bool,
}

impl ProtocolState {
    /// Effective decode/encode charset: negotiated CHARSET if any, else the configured
    /// per-world default. `ProtocolState` holds no `WorldSettings` of its own, so unlike
    /// `World::effective_encoding` (which reads `self.settings` directly), the caller
    /// passes it in.
    pub fn effective_encoding(&self, settings: &WorldSettings) -> Encoding {
        self.negotiated_encoding.unwrap_or(settings.encoding)
    }

    /// Send a NAWS subnegotiation on `command_tx` iff NAWS is enabled on this connection
    /// and `size` differs from the last size actually sent. Shared by the core's own
    /// `NawsRequested` arm below and by both callers' "a client's window just changed"
    /// paths (`App::send_naws_if_changed`, `App::send_naws_to_all_multiuser_worlds`) -
    /// the one place that knows what "changed" means for NAWS.
    pub fn send_naws_if_changed(
        &mut self,
        size: Option<(u16, u16)>,
        command_tx: Option<&mpsc::Sender<WriteCommand>>,
    ) -> bool {
        if !self.naws_enabled {
            return false;
        }
        let Some((width, height)) = size else { return false };
        if self.naws_sent_size == Some((width, height)) {
            return false;
        }
        let Some(tx) = command_tx else { return false };
        let _ = tx.try_send(WriteCommand::Raw(build_naws_subnegotiation(width, height)));
        self.naws_sent_size = Some((width, height));
        true
    }

    /// Reset every mirror a fresh connection must not inherit from the previous one -
    /// the mirror half of `World::clear_connection_state` (plan Job 9), including
    /// "mark stats dirty iff there was something to clear" so a client that already saw
    /// the old connection's vitals is told to stop showing them.
    pub fn clear(&mut self) {
        self.negotiated_encoding = None;
        self.naws_enabled = false;
        self.naws_sent_size = None;
        self.gmcp_enabled = false;
        self.msdp_enabled = false;
        self.mccp2_active = false;
        self.echo_masked = false;
        self.gmcp_data.clear();
        self.msdp_variables.clear();
        self.mssp_data.clear();
        if !self.stats.is_empty() {
            self.stats_dirty = true;
        }
        self.stats.clear();
        self.mcmp_default_url.clear();
        self.gmcp_supported_packages.clear();
        self.uses_wont_echo_prompt = false;
    }

    /// Apply one `TelnetEvent` to this connection's protocol mirrors. Exhaustive over
    /// `TelnetEvent` with no wildcard arm (this module's doc comment) - a future
    /// `TelnetEvent` variant will not compile here until this match says what every
    /// connection (single-user and multiuser alike) does with it.
    pub fn apply_telnet_event(&mut self, ev: TelnetEvent, ctx: &ProtocolCtx) -> ProtocolOutcome {
        match ev {
            // Not a protocol mirror at all - `World`/`UserConnection` each own their own
            // `telnet_mode` bool directly (see this module's doc comment).
            TelnetEvent::TelnetDetected => ProtocolOutcome::default(),
            // Has its own `AppEvent::Prompt`/`AppEvent::MultiuserPrompt` path entirely
            // outside this event stream - nothing for the core to do.
            TelnetEvent::Prompt(_) => {
                // Record that this world marks its prompts — see `seen_prompt_marker`.
                self.seen_prompt_marker = true;
                ProtocolOutcome::default()
            }
            TelnetEvent::NawsRequested => {
                self.naws_enabled = true;
                self.send_naws_if_changed(ctx.naws_size, ctx.command_tx);
                ProtocolOutcome::default()
            }
            // The session already answered on the wire (Job 12/MTTS) - nothing left to do.
            TelnetEvent::TtypeRequested => ProtocolOutcome::default(),
            TelnetEvent::CharsetRequest(charsets) => {
                self.apply_charset_request(&charsets, ctx);
                ProtocolOutcome::default()
            }
            TelnetEvent::GmcpMessage(package, json_data) => {
                self.apply_gmcp_message(&package, &json_data, ctx)
            }
            TelnetEvent::MsdpVariable(variable, value_json) => {
                self.apply_msdp_variable(&variable, &value_json, ctx)
            }
            // MSSP is a complete snapshot every time it arrives (not incremental), so
            // replace rather than merge - see the field's doc comment.
            TelnetEvent::MsspData(pairs) => {
                self.mssp_data = pairs;
                ProtocolOutcome::default()
            }
            // Job 10a's reload-guard mirror - maintained identically for both `World` and
            // multiuser connections (plan Job 9: "stored in both").
            TelnetEvent::CompressionStarted => {
                self.mccp2_active = true;
                ProtocolOutcome::default()
            }
            TelnetEvent::CompressionEnded => {
                self.mccp2_active = false;
                ProtocolOutcome::default()
            }
            TelnetEvent::CompressionFailed(_reason) => {
                self.mccp2_active = false;
                ProtocolOutcome::default()
            }
            TelnetEvent::ProtocolError(_) => ProtocolOutcome::default(),
            TelnetEvent::OptionEnabled(opt) => {
                match opt {
                    TELNET_OPT_GMCP => self.apply_gmcp_negotiated(ctx),
                    TELNET_OPT_MSDP => self.apply_msdp_negotiated(ctx),
                    _ => {}
                }
                ProtocolOutcome::default()
            }
            // Finding 4: the server withdrew an option it had previously had accepted -
            // clear the matching mirror rather than leaving it stale. Only options with
            // such a mirror do anything here.
            TelnetEvent::OptionDisabled(opt) => {
                match opt {
                    TELNET_OPT_GMCP => self.gmcp_enabled = false,
                    TELNET_OPT_MSDP => self.msdp_enabled = false,
                    TELNET_OPT_NAWS => self.naws_enabled = false,
                    _ => {}
                }
                ProtocolOutcome::default()
            }
            // "Stored in both" (plan Job 9): multiuser has no consumer for this (no
            // partial-line prompt timeout there - that lives in `App::process_server_data`,
            // a `World`-only code path) but the flag itself is harmless bookkeeping either
            // way, so it is set uniformly rather than only on `World`. Documented follow-up
            // if multiuser ever grows that timeout.
            TelnetEvent::WontEchoPromptHint => {
                self.uses_wont_echo_prompt = true;
                ProtocolOutcome::default()
            }
            TelnetEvent::EchoOff => self.apply_echo_mask(true, ctx),
            TelnetEvent::EchoOn => self.apply_echo_mask(false, ctx),
            // Not in the core (plan Job 9): whether/how to play a sound is a caller
            // decision (`World` plays via `App::handle_msp_trigger`; multiuser has no
            // per-connection host to play audio on and is an explicit no-op) - kept as an
            // event the core forwards nothing for, rather than folded away, so the
            // *handler* makes that call, not the reader (see `to_app_event`'s doc comment).
            TelnetEvent::MspTrigger(_) => ProtocolOutcome::default(),
        }
    }

    /// `EchoOff`/`EchoOn`: the cleartext-password fix (T3.2). Only broadcasts on an actual
    /// change - ECHO is answered unconditionally every time it's seen, with no Q-method
    /// dedup, so a chatty server repeating `WILL`/`WONT ECHO` must not flicker every
    /// client's masking on and off.
    fn apply_echo_mask(&mut self, masked: bool, ctx: &ProtocolCtx) -> ProtocolOutcome {
        let mut out = ProtocolOutcome::default();
        if self.echo_masked != masked {
            self.echo_masked = masked;
            out.broadcasts.push(WsMessage::EchoMaskChanged { world_index: ctx.world_index, masked });
        }
        out
    }

    /// Ported verbatim from the former `App::handle_charset_requested` (see its doc
    /// comment for the full priority-order rationale: an explicit non-UTF-8 per-world
    /// choice is confirmed-or-rejected rather than silently overridden by UTF-8).
    fn apply_charset_request(&mut self, charsets: &[String], ctx: &ProtocolCtx) {
        let explicit_encoding = ctx.settings.encoding;
        if explicit_encoding != Encoding::Utf8 {
            let offered =
                charsets.iter().any(|name| Encoding::from_iana_name(name) == Some(explicit_encoding));
            if let Some(tx) = ctx.command_tx {
                if offered {
                    let response = build_charset_accepted(explicit_encoding.iana_name());
                    let _ = tx.try_send(WriteCommand::Raw(response));
                    let _ = tx.try_send(WriteCommand::SetEncoding(explicit_encoding));
                    self.negotiated_encoding = Some(explicit_encoding);
                } else {
                    let _ = tx.try_send(WriteCommand::Raw(build_charset_rejected()));
                }
            }
            return;
        }

        // Priority order: UTF-8 > Latin1 > Fansi.
        let mut best: Option<(Encoding, &str)> = None;
        for name in charsets {
            if let Some(enc) = Encoding::from_iana_name(name) {
                match enc {
                    Encoding::Utf8 => {
                        best = Some((enc, "UTF-8"));
                        break;
                    }
                    _ => {
                        if best.is_none() {
                            best = Some((
                                enc,
                                match enc {
                                    Encoding::Latin1 => "ISO-8859-1",
                                    Encoding::Fansi => "IBM437",
                                    Encoding::Utf8 => unreachable!(),
                                },
                            ));
                        }
                    }
                }
            }
        }

        if let Some(tx) = ctx.command_tx {
            if let Some((enc, iana_name)) = best {
                let response = build_charset_accepted(iana_name);
                let _ = tx.try_send(WriteCommand::Raw(response));
                let _ = tx.try_send(WriteCommand::SetEncoding(enc));
                self.negotiated_encoding = Some(enc);
            } else {
                let response = build_charset_rejected();
                let _ = tx.try_send(WriteCommand::Raw(response));
            }
        }
    }

    /// Ported from the former `App::handle_gmcp_negotiated`: announce `Core.Hello` and
    /// `Core.Supports.Set` - this is what makes a GMCP server send anything at all.
    fn apply_gmcp_negotiated(&mut self, ctx: &ProtocolCtx) {
        self.gmcp_enabled = true;
        let packages: Vec<String> = ctx
            .settings
            .gmcp_packages
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.gmcp_supported_packages = packages.clone();
        if let Some(tx) = ctx.command_tx {
            let hello = build_gmcp_message(
                "Core.Hello",
                &format!("{{\"client\":\"Clay\",\"version\":\"{}\"}}", VERSION),
            );
            let _ = tx.try_send(WriteCommand::Raw(hello));
            let json_list: Vec<String> = packages.iter().map(|p| format!("\"{}\"", p)).collect();
            let supports =
                build_gmcp_message("Core.Supports.Set", &format!("[{}]", json_list.join(",")));
            let _ = tx.try_send(WriteCommand::Raw(supports));
        }
    }

    /// Ported from the former `App::handle_msdp_negotiated` (mud-status-display.md Job 6):
    /// ask what the server can report, since MSDP never volunteers data unprompted.
    fn apply_msdp_negotiated(&mut self, ctx: &ProtocolCtx) {
        self.msdp_enabled = true;
        if let Some(tx) = ctx.command_tx {
            let _ = tx.try_send(WriteCommand::Raw(build_msdp_set("LIST", "REPORTABLE_VARIABLES")));
        }
    }

    /// Ported from the former `App::handle_gmcp_received`, minus the World-only extras
    /// (host audio, TF hooks, `needs_output_redraw`) - those stay in the caller (plan
    /// Job 9's per-variant table).
    fn apply_gmcp_message(&mut self, package: &str, json_data: &str, ctx: &ProtocolCtx) -> ProtocolOutcome {
        let mut out = ProtocolOutcome::default();
        self.gmcp_data.insert(package.to_string(), json_data.to_string());
        self.stats.update_from_gmcp(package, json_data);
        if stats::package_has_prefix(package, "Char.") {
            self.stats_dirty = true;
            out.stats_changed = true;
        }
        if package.eq_ignore_ascii_case("Client.Media.Default") {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_data) {
                if let Some(url) = parsed.get("url").and_then(|v| v.as_str()) {
                    self.mcmp_default_url = url.to_string();
                }
            }
        }
        out.broadcasts.push(WsMessage::GmcpData {
            world_index: ctx.world_index,
            package: package.to_string(),
            data: json_data.to_string(),
        });
        if stats::package_has_prefix(package, "Client.Media.") {
            let action = package.rsplit('.').next().unwrap_or("Play").to_string();
            out.broadcasts.push(WsMessage::McmpMedia {
                world_index: ctx.world_index,
                action,
                data: json_data.to_string(),
                default_url: self.mcmp_default_url.clone(),
            });
        }
        out
    }

    /// Ported from the former `App::handle_msdp_received`, minus the World-only extras
    /// (TF hook, `needs_output_redraw`).
    fn apply_msdp_variable(&mut self, variable: &str, value_json: &str, ctx: &ProtocolCtx) -> ProtocolOutcome {
        let mut out = ProtocolOutcome::default();
        self.msdp_variables.insert(variable.to_string(), value_json.to_string());
        self.stats.update_from_msdp(variable, value_json);
        if !stats::is_msdp_meta_variable(variable) {
            self.stats_dirty = true;
            out.stats_changed = true;
        }
        if variable.eq_ignore_ascii_case("REPORTABLE_VARIABLES") {
            self.request_msdp_reports(value_json, ctx);
        }
        out.broadcasts.push(WsMessage::MsdpData {
            world_index: ctx.world_index,
            variable: variable.to_string(),
            value: value_json.to_string(),
        });
        out
    }

    /// Ported verbatim from the former `App::request_msdp_reports` - see its doc comment
    /// for why this is capped and meta-variable-filtered (a MUD server is untrusted
    /// input).
    fn request_msdp_reports(&self, reportable_json: &str, ctx: &ProtocolCtx) {
        let Ok(serde_json::Value::Array(names)) = serde_json::from_str::<serde_json::Value>(reportable_json)
        else {
            return;
        };
        let Some(tx) = ctx.command_tx else { return };
        for name in names
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|name| !stats::is_msdp_meta_variable(name))
            .take(MAX_MSDP_AUTO_REPORT)
        {
            let _ = tx.try_send(WriteCommand::Raw(build_msdp_set("REPORT", name)));
        }
    }
}
