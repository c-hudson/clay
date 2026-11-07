//! `/info`: what goes on behind the scenes, for the curious - a world's connection and
//! telnet negotiation, its buffers, and Clay's own process, memory and web clients.
//!
//! The server builds one `InfoReport` (`App::build_info_report`); the console prints it
//! with `report_lines`, the GUIs and the web page draw it in a window (`RequestInfo` /
//! `WsMessage::InfoReport`), and each client adds its own section about itself
//! (This GUI / This page / This console). Every figure is a snapshot: running `/info`
//! again gives a new one.

use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::protocol_state::{DiagRecord, NoteDir, OptionSides, ProtocolState};
use crate::telnet::{
    telnet_option_name, TELNET_OPT_CHARSET, TELNET_OPT_ECHO, TELNET_OPT_EOR, TELNET_OPT_GMCP,
    TELNET_OPT_MCCP2, TELNET_OPT_MSDP, TELNET_OPT_MSP, TELNET_OPT_MSSP, TELNET_OPT_NAWS,
    TELNET_OPT_SGA, TELNET_OPT_TTYPE,
};
use crate::{App, World, WorldType};

/// The whole report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InfoReport {
    pub title: String,
    pub sections: Vec<InfoSection>,
}

/// One heading's worth: key/value rows, then free lines (the negotiation log).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InfoSection {
    pub title: String,
    #[serde(default)]
    pub rows: Vec<InfoRow>,
    #[serde(default)]
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InfoRow {
    pub key: String,
    pub value: String,
    /// "good", "bad" or "warn": how a window colours the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
}

impl InfoSection {
    pub fn new(title: impl Into<String>) -> Self {
        InfoSection { title: title.into(), ..InfoSection::default() }
    }

    pub fn row(&mut self, key: &str, value: impl Into<String>) -> &mut Self {
        self.rows.push(InfoRow { key: key.to_string(), value: value.into(), tone: None });
        self
    }

    pub fn toned(&mut self, key: &str, value: impl Into<String>, tone: &str) -> &mut Self {
        self.rows.push(InfoRow { key: key.to_string(), value: value.into(), tone: Some(tone.to_string()) });
        self
    }
}

/// The report as text: the console prints it with colour (`ansi`), the GUIs' Copy button
/// and an older client get it plain.
pub fn report_lines(report: &InfoReport, ansi: bool) -> Vec<String> {
    let (bold, head, reset) = if ansi { ("\x1b[1m", "\x1b[1;36m", "\x1b[0m") } else { ("", "", "") };
    let mut out = vec![format!("{bold}{}{reset}", report.title)];
    for section in &report.sections {
        out.push(String::new());
        out.push(format!("{head}{}{reset}", section.title));
        let width = section.rows.iter().map(|r| r.key.chars().count()).max().unwrap_or(0).min(24);
        for row in &section.rows {
            let colour = match (ansi, row.tone.as_deref()) {
                (true, Some("good")) => "\x1b[32m",
                (true, Some("bad")) => "\x1b[31m",
                (true, Some("warn")) => "\x1b[33m",
                _ => "",
            };
            let end = if colour.is_empty() { "" } else { reset };
            out.push(format!("  {:<width$}  {colour}{}{end}", row.key, row.value));
        }
        for line in &section.lines {
            out.push(format!("  {line}"));
        }
    }
    out
}

/// "512 B", "12.3 KB", "4.5 MB", "1.20 GB".
pub fn fmt_bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.1} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / (K * K))
    } else {
        format!("{:.2} GB", f / (K * K * K))
    }
}

/// "5s", "3m 4s", "1h 2m", "2d 3h".
pub fn fmt_duration(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if secs >= 3600 {
        format!("{h}h {m}m")
    } else if secs >= 60 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn ago(at: Option<Instant>) -> String {
    match at {
        Some(t) => format!("{} ago", fmt_duration(t.elapsed().as_secs())),
        None => "never".to_string(),
    }
}

/// Seconds since `t`, 0 for a time in the future.
fn since(t: SystemTime) -> u64 {
    SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0)
}

/// Local "YYYY-MM-DD HH:MM" for a time in Unix seconds.
fn local_date_time(epoch_secs: u64) -> String {
    let lt = crate::util::local_time_from_epoch(epoch_secs as i64);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", lt.year, lt.month, lt.day, lt.hour, lt.minute)
}

/// Local "HH:MM:SS.mmm" for a note's time.
fn local_clock_ms(at_ms: u64) -> String {
    let lt = crate::util::local_time_from_epoch((at_ms / 1000) as i64);
    format!("{:02}:{:02}:{:02}.{:03}", lt.hour, lt.minute, lt.second, at_ms % 1000)
}

/// "1 room", "3 rooms".
fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// MUD-supplied text, made safe to show and capped.
fn shown(text: &str) -> String {
    crate::stats::sanitize_and_cap(text, 80)
}

fn world_type_label(t: &WorldType) -> &'static str {
    match t {
        WorldType::Mud => "MUD",
        WorldType::MudTimedPrompt => "MUD (timed prompts)",
        WorldType::Slack => "Slack",
        WorldType::Discord => "Discord",
        WorldType::Shell => "Shell",
    }
}

/// The MTTS bits a `MTTS <n>` TTYPE answer stands for.
fn mtts_bits(n: u64) -> String {
    const BITS: [(u64, &str); 12] = [
        (1, "ANSI"), (2, "VT100"), (4, "UTF-8"), (8, "256 colours"), (16, "mouse"),
        (32, "OSC colours"), (64, "screen reader"), (128, "proxy"), (256, "truecolour"),
        (512, "MNES"), (1024, "MSLP"), (2048, "SSL"),
    ];
    BITS.iter().filter(|(bit, _)| n & bit != 0).map(|(_, name)| *name).collect::<Vec<_>>().join(", ")
}

/// The negotiation log's lines, newest last.
fn log_lines(record: &DiagRecord) -> Vec<String> {
    let mut lines = Vec::new();
    if record.dropped > 0 {
        lines.push(format!("({} earlier not kept)", record.dropped));
    }
    for note in &record.notes {
        let times = if note.repeats > 1 { format!("  (x{})", note.repeats) } else { String::new() };
        let text = match note.dir {
            NoteDir::Server if note.reply.is_empty() => format!("<- {}", note.what),
            NoteDir::Server => format!("<- {}   -> {}", note.what, note.reply),
            NoteDir::Clay => format!("-> {}", note.what),
        };
        lines.push(format!("{}  {text}{times}", local_clock_ms(note.at_ms)));
    }
    lines
}

/// What the TTYPE answers were, from the log: "CLAY, XTERM-256COLOR, MTTS 2831 (ANSI, ...)".
fn ttype_answers(record: &DiagRecord) -> Option<String> {
    let mut answers: Vec<String> = Vec::new();
    for note in &record.notes {
        for part in note.reply.split(", ") {
            if let Some(answer) = part.strip_prefix("SB TTYPE IS ") {
                let answer = match answer.strip_prefix("MTTS ").and_then(|n| n.trim().parse::<u64>().ok()) {
                    Some(n) => format!("MTTS {n} ({})", mtts_bits(n)),
                    None => answer.to_string(),
                };
                if !answers.contains(&answer) {
                    answers.push(answer);
                }
            }
        }
    }
    (!answers.is_empty()).then(|| answers.join(", "))
}

/// One option's row: both sides' state, then what Clay knows about it.
fn option_value(opt: u8, sides: Option<OptionSides>, offered: bool) -> (String, &'static str) {
    match sides {
        Some(OptionSides { server: true, clay: true }) => ("on, both sides".to_string(), "good"),
        Some(OptionSides { server: true, clay: false }) => ("on (the server does it)".to_string(), "good"),
        Some(OptionSides { server: false, clay: true }) => ("on (Clay does it)".to_string(), "good"),
        _ if offered => ("off (refused or withdrawn)".to_string(), "warn"),
        _ if opt == TELNET_OPT_ECHO => ("off (Clay echoes what you type)".to_string(), ""),
        _ => ("not offered".to_string(), ""),
    }
}

/// The two sections every telnet connection has: its options and the negotiation log.
fn telnet_sections(protocol: &ProtocolState, record: Option<&DiagRecord>, mcp: Option<&crate::mcp::McpState>, settings: &crate::WorldSettings) -> Vec<InfoSection> {
    let empty = DiagRecord::default();
    let record = record.unwrap_or(&empty);
    let mut options = InfoSection::new("Telnet options");
    if record.reloaded {
        options.row("Note", "negotiated before the last /reload (the new process keeps what was agreed)");
    }
    let mentioned = |opt: u8| {
        let name = telnet_option_name(opt);
        record.notes.iter().any(|n| n.what.ends_with(&format!(" {name}")) || n.what.starts_with(&format!("SB {name} ")))
    };
    let known = [
        TELNET_OPT_ECHO, TELNET_OPT_SGA, TELNET_OPT_TTYPE, TELNET_OPT_EOR, TELNET_OPT_NAWS,
        TELNET_OPT_CHARSET, TELNET_OPT_MSDP, TELNET_OPT_MSSP, TELNET_OPT_MCCP2, TELNET_OPT_MSP,
        TELNET_OPT_GMCP,
    ];
    let mut listed: Vec<u8> = known.to_vec();
    listed.extend(record.options.keys().copied().filter(|o| !known.contains(o)));
    for opt in listed {
        let sides = record.options.get(&opt).copied();
        let (mut value, mut tone) = option_value(opt, sides, mentioned(opt));
        let mut detail: Vec<String> = Vec::new();
        match opt {
            TELNET_OPT_ECHO if protocol.echo_masked => {
                value = "on (the server echoes: input is masked)".to_string();
                tone = "warn";
            }
            TELNET_OPT_NAWS => {
                if let Some((w, h)) = protocol.naws_sent_size {
                    detail.push(format!("last sent {w}x{h}"));
                }
            }
            TELNET_OPT_TTYPE => {
                if let Some(answers) = ttype_answers(record) {
                    detail.push(format!("answered {answers}"));
                }
            }
            TELNET_OPT_CHARSET => {
                if let Some(enc) = protocol.negotiated_encoding {
                    detail.push(format!("agreed {}", enc.iana_name()));
                }
            }
            TELNET_OPT_MSDP if protocol.msdp_enabled || !protocol.msdp_variables.is_empty() => {
                detail.push(format!("{}, {}", count(protocol.msdp_variables.len(), "variable"), count(protocol.msdp_updates as usize, "update")));
            }
            TELNET_OPT_MSSP if !protocol.mssp_data.is_empty() => {
                detail.push(format!("{} (see /mssp)", count(protocol.mssp_data.len(), "variable")));
            }
            TELNET_OPT_MCCP2 => {
                if !settings.mccp2_enabled {
                    detail.push("turned off for this world".to_string());
                } else if protocol.mccp2_active {
                    detail.push("compressing now".to_string());
                }
            }
            TELNET_OPT_MSP if !settings.msp_enabled => detail.push("sound triggers off for this world".to_string()),
            TELNET_OPT_GMCP if protocol.gmcp_enabled => {
                if !protocol.gmcp_supported_packages.is_empty() {
                    detail.push(format!("announced {}", shown(&protocol.gmcp_supported_packages.join(", "))));
                }
                let total: u64 = protocol.gmcp_received.values().sum();
                detail.push(format!("{} received", count(total as usize, "message")));
            }
            _ => {}
        }
        // GMCP and MSDP turn on in the App's mirror even when a reload's table doesn't say.
        if sides.is_none() && ((opt == TELNET_OPT_GMCP && protocol.gmcp_enabled) || (opt == TELNET_OPT_MSDP && protocol.msdp_enabled)) {
            value = "on (the server does it)".to_string();
            tone = "good";
        }
        if !detail.is_empty() {
            value = format!("{value}; {}", detail.join("; "));
        }
        let key = format!("{} ({opt})", telnet_option_name(opt));
        if tone.is_empty() {
            options.row(&key, value);
        } else {
            options.toned(&key, value, tone);
        }
    }
    if let Some(mcp) = mcp {
        options.row("MCP", if mcp.has_key() { "negotiated (out of band)" } else { "not in use" });
    }
    if !protocol.gmcp_received.is_empty() {
        let mut counts: Vec<(&String, &u64)> = protocol.gmcp_received.iter().collect();
        counts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let top: Vec<String> = counts.iter().take(12).map(|(p, n)| format!("{} x{n}", shown(p))).collect();
        options.row("GMCP received", top.join(", "));
    }

    let mut log = InfoSection::new("Negotiation log");
    log.lines = log_lines(record);
    if log.lines.is_empty() {
        log.lines.push("nothing recorded for this connection".to_string());
    }
    vec![options, log]
}

/// How many bytes `lines` hold, roughly: each line's text plus its own size.
fn lines_bytes(lines: &[crate::OutputLine]) -> u64 {
    lines.iter().map(|l| (std::mem::size_of::<crate::OutputLine>() + l.text.capacity()) as u64).sum()
}

/// A file's size and its SQLite write-ahead log's, if there is one.
fn db_size(path: &std::path::Path) -> Option<u64> {
    let main = std::fs::metadata(path).ok()?.len();
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    Some(main + std::fs::metadata(std::path::PathBuf::from(wal)).map(|m| m.len()).unwrap_or(0))
}

/// The media cache's size and file count, counting no more than `MEDIA_WALK_CAP` entries
/// (the event loop waits on this walk).
fn media_size(dir: &std::path::Path) -> Option<(u64, usize, bool)> {
    const MEDIA_WALK_CAP: usize = 5000;
    let mut stack = vec![dir.to_path_buf()];
    let (mut bytes, mut files, mut seen) = (0u64, 0usize, 0usize);
    let mut any = false;
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        any = true;
        for entry in entries.flatten() {
            seen += 1;
            if seen > MEDIA_WALK_CAP {
                return Some((bytes, files, true));
            }
            match entry.metadata() {
                Ok(m) if m.is_dir() => stack.push(entry.path()),
                Ok(m) => {
                    bytes += m.len();
                    files += 1;
                }
                Err(_) => {}
            }
        }
    }
    any.then_some((bytes, files, false))
}

/// The process section's rows, for the server's report and each client's own.
pub fn process_rows(section: &mut InfoSection, stats: &crate::platform::ProcessStats) {
    section.row("Process id", stats.pid.to_string());
    if let Some(cpu) = stats.cpu_secs {
        section.row("CPU time", format!("{cpu:.1}s"));
    }
    let mut ram = Vec::new();
    if let Some(r) = stats.resident {
        ram.push(fmt_bytes(r));
    }
    if let Some(p) = stats.peak_resident {
        ram.push(format!("peak {}", fmt_bytes(p)));
    }
    section.row("Memory in use", if ram.is_empty() { "not available on this system".to_string() } else { ram.join(", ") });
    if let Some(v) = stats.virtual_size {
        let label = if cfg!(windows) { "Private memory" } else { "Address space" };
        section.row(label, fmt_bytes(v));
    }
    if let Some(t) = stats.threads {
        section.row("Threads", t.to_string());
    }
    if let Some(f) = stats.open_files {
        section.row(if cfg!(windows) { "Open handles" } else { "Open files" }, f.to_string());
    }
}

/// Uptime rows: since when this run of Clay (through reloads) and this process went.
pub fn uptime_rows(section: &mut InfoSection, process_started_secs: u64) {
    let (run_started, restarts) = crate::platform::run_started();
    let now = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if run_started > 0 {
        section.row(
            "Running for",
            format!("{} (since {})", fmt_duration(now.saturating_sub(run_started)), local_date_time(run_started)),
        );
    }
    if restarts > 0 && process_started_secs > 0 {
        section.row(
            "Since the last reload",
            format!("{} ({restarts} reload{} or restart{})", fmt_duration(now.saturating_sub(process_started_secs)),
                if restarts == 1 { "" } else { "s" }, if restarts == 1 { "" } else { "s" }),
        );
    }
}

/// A client's section about its own process, for a GUI or console attached to another
/// machine's Clay: its build, process, memory and how long it has run.
pub fn own_process_section(title: &str) -> InfoSection {
    let mut s = InfoSection::new(title);
    s.row("Version", crate::version_detail());
    process_rows(&mut s, &crate::platform::process_stats());
    uptime_rows(&mut s, crate::STARTUP_TIME.load(std::sync::atomic::Ordering::Relaxed));
    s
}

impl App {
    /// `/info [world]`'s world: the named one, or `current`.
    pub(crate) fn info_world(&self, name: Option<&str>, current: usize) -> Result<usize, String> {
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => self.find_world_index(name).ok_or_else(|| format!("No world named '{name}'.")),
            None if current < self.worlds.len() => Ok(current),
            None => Err("There is no world to report on.".to_string()),
        }
    }

    /// The whole report for world `idx` (single-user). `asker` is the client that asked,
    /// marked in the client list; `None` for the console.
    pub(crate) fn build_info_report(&self, idx: usize, asker: Option<u64>) -> InfoReport {
        let world = &self.worlds[idx];
        let mut report = InfoReport { title: format!("Info: {}", world.name), sections: Vec::new() };
        report.sections.push(self.world_section(world));
        let record = world.protocol.diag.as_ref().map(|d| d.snapshot());
        if world.settings.world_type.is_mud() && !world.is_activity {
            report.sections.extend(telnet_sections(&world.protocol, record.as_ref(), Some(&world.mcp), &world.settings));
            report.sections.push(prompt_section(&world.protocol, &world.prompt, world.prompt_timed, world.prompt_count, &world.settings));
        }
        report.sections.push(buffer_section(world));
        report.sections.push(self.clay_section());
        report.sections.push(self.memory_section());
        report.sections.push(self.web_section(asker));
        report
    }

    /// The report for a multiuser user: only their own connection to world `idx` - the
    /// process, its memory and the other users' clients are the operator's.
    pub(crate) fn build_multiuser_info_report(&self, idx: usize, username: &str) -> InfoReport {
        let world = &self.worlds[idx];
        let conn = self.user_connections.get(&(idx, username.to_string()));
        let mut report = InfoReport { title: format!("Info: {}", world.name), sections: Vec::new() };
        let mut s = InfoSection::new("World");
        s.row("Type", world_type_label(&world.settings.world_type));
        let connected = conn.is_some_and(|c| c.connected);
        if connected {
            s.toned("State", "connected", "good");
        } else {
            s.row("State", "not connected");
        }
        s.row("Server", format!("{}:{}{}", world.settings.hostname, world.settings.port, if world.settings.use_ssl { " (TLS)" } else { "" }));
        if let Some(c) = conn {
            let record = c.protocol.diag.as_ref().map(|d| d.snapshot());
            if let Some(r) = &record {
                if let Some(peer) = &r.peer {
                    s.row("Address", peer.clone());
                }
                if let Some(v) = &r.tls_version {
                    s.row("Encryption", v.clone());
                }
                s.row("Traffic", format!("received {}, sent {}", fmt_bytes(r.wire_in), fmt_bytes(r.wire_out)));
            }
            if let Some(at) = c.connected_at.filter(|_| c.connected) {
                s.row("Connected for", fmt_duration(since(at)));
            }
            s.row("Last received", ago(c.last_receive_time));
            s.row("Last sent", ago(c.last_send_time));
            report.sections.push(s);
            if world.settings.world_type.is_mud() {
                report.sections.extend(telnet_sections(&c.protocol, record.as_ref(), None, &world.settings));
                report.sections.push(prompt_section(&c.protocol, &c.prompt, false, c.prompt_count, &world.settings));
            }
        } else {
            report.sections.push(s);
        }
        let mut clay = InfoSection::new("Clay");
        clay.row("Version", crate::version_detail());
        report.sections.push(clay);
        report
    }

    fn world_section(&self, world: &World) -> InfoSection {
        let mut s = InfoSection::new("World");
        let settings = &world.settings;
        if world.is_activity {
            s.row("Type", "Activity (other worlds' lines as they arrive)");
            return s;
        }
        s.row("Type", world_type_label(&settings.world_type));
        if world.connected {
            s.toned("State", "connected", "good");
        } else {
            s.row("State", if world.was_connected { "disconnected" } else { "not connected" });
        }
        if !settings.world_type.is_mud() {
            if settings.world_type == WorldType::Shell {
                if let Some(screen) = &world.shell_screen {
                    s.row("Terminal", format!("{} columns x {} rows", screen.cols, screen.rows));
                }
            }
            if let Some(at) = world.connected_at.filter(|_| world.connected) {
                s.row("Connected for", fmt_duration(since(at)));
            }
            s.row("Last received", ago(world.last_receive_time.filter(|_| world.connected)));
            return s;
        }
        let host_port = format!("{}:{}", settings.hostname, settings.port);
        s.row("Server", format!("{host_port}{}", if settings.use_ssl { " (TLS)" } else { "" }));
        let record = world.protocol.diag.as_ref().map(|d| d.snapshot());
        if world.connected {
            if let Some(peer) = record.as_ref().and_then(|r| r.peer.clone()) {
                s.row("Address", peer);
            }
            let encryption = if !world.is_tls {
                "none (plain telnet)".to_string()
            } else if let Some(pid) = world.proxy_pid.filter(|p| *p != 0) {
                format!("TLS through the TLS proxy (process {pid}; it doesn't report its cipher)")
            } else {
                let version = record.as_ref().and_then(|r| r.tls_version.clone()).unwrap_or_else(|| "TLS".to_string());
                match &world.tls_cipher {
                    Some(c) => format!("{version}, {c}"),
                    None => version,
                }
            };
            if world.is_tls {
                s.toned("Encryption", encryption, "good");
            } else {
                s.row("Encryption", encryption);
            }
            if let Some(at) = world.connected_at {
                s.row("Connected for", format!("{} (connection #{})", fmt_duration(since(at)), world.connection_id));
            }
        }
        if settings.use_ssl {
            let host = settings.hostname.split(',').next().unwrap_or("").trim();
            match crate::persistence::get_pin(&format!("{}:{}", host, settings.port)) {
                Some(pin) => {
                    let short = if pin.len() > 16 { format!("{}...{}", &pin[..8], &pin[pin.len() - 8..]) } else { pin };
                    s.row("Certificate pin", format!("SHA-256 {short} (trusted on first use)"));
                }
                None => {
                    s.row("Certificate pin", "none yet (pinned at the first TLS connect)");
                }
            }
        }
        let negotiated = world.protocol.negotiated_encoding;
        let effective = world.effective_encoding();
        s.row("Encoding", match negotiated {
            Some(n) => format!("{} (agreed by CHARSET; the world is set to {})", n.iana_name(), settings.encoding.iana_name()),
            None => format!("{} (as set)", effective.iana_name()),
        });
        let login = if settings.user.is_empty() {
            "none (no character name set)".to_string()
        } else if settings.password.is_empty() {
            format!("{} (character name only)", settings.auto_connect_type.name())
        } else {
            format!("{} (character name and password set)", settings.auto_connect_type.name())
        };
        s.row("Auto login", login);
        if !settings.login_command.trim().is_empty() {
            s.row("Login command", format!("{} line(s)", settings.login_commands().len()));
        }
        if world.connected {
            let keepalive = settings.keep_alive_type.name();
            s.row("Keepalive", format!("{keepalive}, every 5 minutes of quiet; last sent {}", ago(world.last_nop_time)));
            s.row("Last received", ago(world.last_receive_time));
            s.row("Last sent", ago(world.last_send_time));
            s.row("Telnet", if world.telnet_mode { "yes (the server has sent telnet commands)" } else { "no telnet seen (plain text)" });
            if let Some(r) = &record {
                s.row("Traffic", format!("received {}, sent {}", fmt_bytes(r.wire_in), fmt_bytes(r.wire_out)));
                if r.mccp_in > 0 {
                    let ratio = r.mccp_out as f64 / r.mccp_in.max(1) as f64;
                    s.toned("Compression", format!("MCCP2: {} compressed became {} ({ratio:.1}x)", fmt_bytes(r.mccp_in), fmt_bytes(r.mccp_out)), "good");
                }
            }
        }
        s
    }

    fn clay_section(&self) -> InfoSection {
        let mut s = InfoSection::new("Clay");
        s.row("Version", crate::version_detail());
        if let Some(mode) = crate::crashlog::mode() {
            s.row("Running as", mode);
        }
        // Android has no reload, so no `platform::get_executable_path`.
        if let Ok(path) = std::env::current_exe() {
            s.row("Program", path.display().to_string().trim_end_matches(" (deleted)").to_string());
        }
        let process_started = crate::STARTUP_TIME.load(std::sync::atomic::Ordering::Relaxed);
        uptime_rows(&mut s, process_started);
        let crashes = crate::platform::CRASH_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        if crashes > 0 {
            s.toned("Crash restarts", format!("{crashes} in the last ten minutes"), "bad");
        }
        s.row("Settings", crate::get_settings_path().display().to_string());
        s
    }

    fn memory_section(&self) -> InfoSection {
        let mut s = InfoSection::new("Memory");
        process_rows(&mut s, &crate::platform::process_stats());
        let (mut lines, mut bytes) = (0usize, 0u64);
        for w in &self.worlds {
            lines += w.output_lines.len() + w.pending_lines.len();
            bytes += lines_bytes(&w.output_lines) + lines_bytes(&w.pending_lines);
        }
        s.row("Output kept", format!("{} in {}, about {}", count(lines, "line"), count(self.worlds.len(), "world"), fmt_bytes(bytes)));
        let tf = &self.tf_engine;
        let triggers = tf.macros.iter().filter(|m| m.trigger.is_some()).count();
        let hooks = tf.macros.iter().filter(|m| m.hook.is_some() || !m.extra_hooks.is_empty()).count();
        let keys = tf.macros.iter().filter(|m| m.keybinding.is_some()).count();
        s.row("TinyFugue", format!(
            "{} macros ({triggers} triggers, {hooks} hooks, {keys} key macros), {} key bindings, {} variables, {} timers",
            tf.macros.len(), tf.keybindings.len(), tf.global_vars.len(), tf.processes.len(),
        ));
        s.row("Actions", self.settings.actions.len().to_string());
        if self.spell_checker.has_dictionary() {
            s.row("Spell dictionary", format!("{} words", self.spell_checker.word_count()));
        }
        match &self.scrollback {
            Some(db) => {
                let stats = db.stats();
                let size = db_size(db.path()).map(fmt_bytes).unwrap_or_else(|| "?".to_string());
                let depth = stats.depth.load(std::sync::atomic::Ordering::Relaxed);
                let dropped = stats.dropped.load(std::sync::atomic::Ordering::Relaxed);
                let failed = stats.write_failed.load(std::sync::atomic::Ordering::Relaxed);
                let mut value = format!("{size}, {depth} lines waiting to be written");
                if dropped > 0 || failed > 0 {
                    value.push_str(&format!(", {dropped} dropped, {failed} failed"));
                    s.toned("Archive (scrollback.db)", value, "bad");
                } else {
                    s.row("Archive (scrollback.db)", value);
                }
            }
            None => {
                s.row("Archive (scrollback.db)", "off");
            }
        }
        if let Some(size) = db_size(&crate::clay_config_path("maps.db")) {
            let rooms: usize = self.worlds.iter().map(|w| w.map.graph.len()).sum();
            s.row("Maps (maps.db)", format!("{}, {} loaded", fmt_bytes(size), count(rooms, "room")));
        }
        if let Some((bytes, files, capped)) = media_size(&self.media_cache_dir) {
            let more = if capped { "+" } else { "" };
            s.row("Media cache", format!("{}{more} in {files}{more} files", fmt_bytes(bytes)));
        }
        s
    }

    fn web_section(&self, asker: Option<u64>) -> InfoSection {
        let mut s = InfoSection::new("Web server and clients");
        let Some(ws) = &self.ws_server else {
            s.row("Web server", "not running");
            return s;
        };
        s.row("Port", self.web_port().to_string());
        s.row("Allow list", if self.settings.websocket_allow_list.trim().is_empty() { "none (password from anywhere)" } else { "set" });
        s.row("Auth key", if self.settings.websocket_auth_key.is_some() { "set" } else { "none" });
        let clients: Vec<(u64, String)> = {
            let guard = ws.clients.read().unwrap_or_else(|e| e.into_inner());
            let mut list: Vec<(u64, String)> = guard.iter().filter(|(_, c)| c.authenticated).map(|(id, c)| {
                let kind = match c.client_type {
                    crate::websocket::RemoteClientType::Web => "web page",
                    crate::websocket::RemoteClientType::RemoteGUI => "GUI",
                    crate::websocket::RemoteClientType::RemoteConsole => "console",
                    crate::websocket::RemoteClientType::Android => "Android app",
                };
                let world = self.ws_client_worlds.get(id)
                    .and_then(|v| self.worlds.get(v.world_index))
                    .map(|w| format!(", on {}", w.name))
                    .unwrap_or_default();
                let free = c.tx.capacity();
                let queued = crate::websocket::WS_CLIENT_CHANNEL_CAPACITY.saturating_sub(free);
                let me = if Some(*id) == asker { " (this window)" } else { "" };
                let paused = if c.paused { ", paused" } else { "" };
                (*id, format!(
                    "{kind} from {}, for {}{world}, {queued} message(s) queued{paused}{me}",
                    c.ip_address, fmt_duration(c.connected_at.elapsed().as_secs()),
                ))
            }).collect();
            list.sort_by_key(|(id, _)| *id);
            list
        };
        s.row("Clients", clients.len().to_string());
        for (id, text) in clients {
            s.row(&format!("Client {id}"), text);
        }
        s
    }

    /// `/info` typed at a client whose `/info` reached the server (an older client, an
    /// action or a macro): the report as text, to that client only.
    pub(crate) fn ws_info_text(&self, client_id: u64, current: usize, name: Option<&str>) {
        let text = match self.info_world(name, current) {
            Ok(idx) => report_lines(&self.build_info_report(idx, Some(client_id)), true).join("\n"),
            Err(e) => e,
        };
        self.ws_tell(client_id, current.min(self.worlds.len().saturating_sub(1)), &text);
    }

    /// `RequestInfo`: the report for the client's window.
    pub(crate) fn ws_info_report(&self, client_id: u64, world_index: Option<usize>, current: usize) {
        let idx = world_index.filter(|i| *i < self.worlds.len()).unwrap_or(current);
        if idx >= self.worlds.len() {
            return;
        }
        let report = self.build_info_report(idx, Some(client_id));
        self.ws_send_to_client(client_id, crate::websocket::WsMessage::InfoReport { world_index: idx, report });
    }
}

/// The prompt section: how this connection's prompts are found.
fn prompt_section(protocol: &ProtocolState, prompt: &str, timed: bool, count: usize, settings: &crate::WorldSettings) -> InfoSection {
    let mut s = InfoSection::new("Prompts");
    let how = if protocol.seen_prompt_marker {
        "the server marks them (GA/EOR)".to_string()
    } else if settings.world_type == WorldType::MudTimedPrompt {
        format!("no GA/EOR seen; a line left open {} ms is taken for one", settings.prompt_wait_ms)
    } else {
        "no GA/EOR seen; none are guessed (world type MUD)".to_string()
    };
    s.row("Found by", how);
    s.row("Prompts so far", count.to_string());
    let text = crate::util::strip_ansi_codes(prompt);
    if !text.trim().is_empty() {
        s.row("Current prompt", format!("{}{}", shown(&text), if timed { " (a guess)" } else { "" }));
    }
    s
}

/// The world's own buffers.
fn buffer_section(world: &World) -> InfoSection {
    let mut s = InfoSection::new("This world's buffers");
    s.row("Lines kept", format!("{} (about {})", world.output_lines.len(), fmt_bytes(lines_bytes(&world.output_lines))));
    if !world.pending_lines.is_empty() {
        s.toned("Held by More", world.pending_lines.len().to_string(), "warn");
    }
    s.row("Unseen", world.unseen_lines.to_string());
    s.row("Line numbers", format!("next {}, epoch {}", world.next_seq, world.seq_epoch));
    let ranges = world.broadcast_ledger.lock().map(|l| l.len()).unwrap_or(0);
    if ranges > 1 {
        s.toned("Sent to clients", format!("{ranges} separate ranges (a gap: see /dump)"), "bad");
    } else {
        s.row("Sent to clients", "every line");
    }
    let rooms = world.map.graph.len();
    if rooms > 0 {
        s.row("Map", count(rooms, "room"));
    }
    if world.log_handle.is_some() {
        s.row("Log file", world.get_log_path().display().to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol_state::{ConnDiag, Note};

    #[test]
    fn report_lines_aligns_rows_and_colours_only_with_ansi() {
        let mut s = InfoSection::new("World");
        s.row("Type", "MUD").toned("State", "connected", "good");
        s.lines.push("a log line".to_string());
        let report = InfoReport { title: "Info: x".to_string(), sections: vec![s] };
        assert_eq!(report_lines(&report, false), vec![
            "Info: x".to_string(),
            String::new(),
            "World".to_string(),
            "  Type   MUD".to_string(),
            "  State  connected".to_string(),
            "  a log line".to_string(),
        ]);
        let coloured = report_lines(&report, true);
        assert!(coloured[4].contains("\x1b[32mconnected\x1b[0m"), "{:?}", coloured[4]);
        assert!(coloured[2].starts_with("\x1b[1;36m"));
    }

    #[test]
    fn sizes_and_durations_read_naturally() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(fmt_duration(5), "5s");
        assert_eq!(fmt_duration(184), "3m 4s");
        assert_eq!(fmt_duration(3720), "1h 2m");
        assert_eq!(fmt_duration(2 * 86_400 + 3 * 3600), "2d 3h");
    }

    #[test]
    fn the_ttype_answers_name_the_mtts_bits() {
        let mut record = DiagRecord::default();
        for answer in ["CLAY", "XTERM-256COLOR", "MTTS 2317"] {
            record.notes.push_back(Note {
                at_ms: 0, dir: NoteDir::Server, what: "SB TTYPE SEND".into(),
                reply: format!("SB TTYPE IS {answer}"), repeats: 1,
            });
        }
        let answers = ttype_answers(&record).unwrap();
        assert_eq!(answers, "CLAY, XTERM-256COLOR, MTTS 2317 (ANSI, UTF-8, 256 colours, truecolour, SSL)");
    }

    #[test]
    fn options_show_both_sides_and_the_log_reads_in_order() {
        let diag = ConnDiag::restored(DiagRecord::default());
        diag.note(0, NoteDir::Server, "WILL GMCP".into(), "DO GMCP".into(), Some((TELNET_OPT_GMCP, OptionSides { server: true, clay: false })));
        diag.note(0, NoteDir::Server, "DO NAWS".into(), "WILL NAWS".into(), Some((TELNET_OPT_NAWS, OptionSides { server: false, clay: true })));
        diag.note(0, NoteDir::Server, "WILL MXP".into(), "DONT MXP".into(), Some((91, OptionSides::default())));
        diag.note(0, NoteDir::Clay, "SB NAWS 80x24".into(), String::new(), None);
        let record = diag.snapshot();
        let protocol = ProtocolState { gmcp_enabled: true, naws_enabled: true, naws_sent_size: Some((80, 24)), ..ProtocolState::default() };
        let sections = telnet_sections(&protocol, Some(&record), None, &crate::WorldSettings::default());
        let options = &sections[0];
        let row = |key: &str| options.rows.iter().find(|r| r.key.starts_with(key)).map(|r| r.value.clone()).unwrap_or_default();
        assert!(row("GMCP").starts_with("on (the server does it)"), "{}", row("GMCP"));
        assert_eq!(row("NAWS"), "on (Clay does it); last sent 80x24");
        assert_eq!(row("MXP"), "off (refused or withdrawn)");
        assert_eq!(row("SGA"), "not offered");
        let log = &sections[1].lines;
        assert_eq!(log.len(), 4);
        assert!(log[0].ends_with("<- WILL GMCP   -> DO GMCP"), "{}", log[0]);
        assert!(log[3].ends_with("-> SB NAWS 80x24"), "{}", log[3]);
    }

    #[test]
    fn the_report_never_shows_a_password() {
        let mut app = App::new();
        app.worlds.push(crate::World::new("Somewhere"));
        app.worlds[0].settings.hostname = "mud.example".into();
        app.worlds[0].settings.port = "4000".into();
        app.worlds[0].settings.user = "hero".into();
        app.worlds[0].settings.password = "s3cr3t-pw".into();
        let text = report_lines(&app.build_info_report(0, None), false).join("\n");
        assert!(!text.contains("s3cr3t-pw"), "{text}");
        assert!(text.contains("character name and password set"), "{text}");
        assert!(text.contains("mud.example:4000"), "{text}");
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn process_stats_read_this_process() {
        let stats = crate::platform::process_stats();
        assert_eq!(stats.pid, std::process::id());
        assert!(stats.resident.unwrap_or(0) > 0, "{stats:?}");
        assert!(stats.peak_resident.unwrap_or(0) >= stats.resident.unwrap_or(0), "{stats:?}");
        assert!(stats.threads.unwrap_or(0) >= 1, "{stats:?}");
        assert!(stats.open_files.unwrap_or(0) >= 1, "{stats:?}");
        assert!(stats.cpu_secs.is_some(), "{stats:?}");
    }

    #[test]
    fn a_multiuser_report_leaves_out_the_operators_sections() {
        let mut app = App::new();
        app.worlds.push(crate::World::new("Somewhere"));
        app.worlds[0].settings.hostname = "mud.example".into();
        app.worlds[0].settings.port = "4000".into();
        let report = app.build_multiuser_info_report(0, "alice");
        let titles: Vec<&str> = report.sections.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"World"), "{titles:?}");
        for hidden in ["Memory", "Web server and clients", "This world's buffers"] {
            assert!(!titles.contains(&hidden), "{hidden} in {titles:?}");
        }
    }
}
