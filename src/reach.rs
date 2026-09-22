//! Reachability: everything Clay knows about how *other* devices can reach this
//! instance's web/WebSocket server — LAN and VPN addresses, the public address,
//! the Windows Firewall verdict, the router (UPnP) port mapping, and the exact
//! strings to type on the other end.
//!
//! The information is built once, server-side (`App::build_reachability_info`),
//! shipped to every UI as `GlobalSettingsMsg.reachability_json`, and *rendered*
//! there — never recomputed. `/reach`, the TUI "Remote Access" popup, the web/GUI
//! dialog and Android therefore all show the same text, produced by
//! [`format_reach_lines`] or from the blob's own fields.
//!
//! Nothing in this module talks to the network except [`local_ipv4_addrs`]
//! (which runs `ipconfig`/`ifconfig`/`hostname -I` locally) and
//! [`lan_ip_toward`] (a UDP `connect()` that sends no packet). The firewall
//! query lives in `firewall.rs`, the UPnP client in `portmap.rs`, and the opt-in
//! public-IP lookup in `App::request_public_ip_lookup`.

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// Windows Defender Firewall verdict for *this executable*. "What wins for
/// clay.exe", not "does Clay's own named rule exist": a Block rule Windows
/// auto-creates when the user clicks Cancel on the first-run alert beats any
/// Allow rule, so it must be reported as `Blocked` even when our rule is there.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "kind")]
pub enum FirewallStatus {
    /// Not Windows — Clay does not manage a host firewall on this platform.
    NotApplicable,
    /// Windows, but no query has run yet.
    #[default]
    Unchecked,
    /// A query is in flight.
    Checking,
    /// No inbound rule mentions this executable: Windows blocks other devices.
    Missing,
    /// An enabled Allow rule for this executable exists and no Block rule does.
    /// `program_matches` is false when the rule points at a different location
    /// of clay.exe (the binary was moved) — it will not apply to this one.
    Allowed { program_matches: bool },
    /// An enabled Block rule for this executable exists (the Cancel-on-alert trap).
    Blocked,
    /// The query ran but could not be interpreted (netsh missing, timed out,
    /// localized output with no recognizable tokens, ...).
    Unknown { detail: String },
}

/// State of the router-side UPnP IGD port mapping for `http_port`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "kind")]
pub enum PortMapStatus {
    /// Mapping disabled, or enabled but not started (web server off).
    #[default]
    Off,
    /// SSDP discovery / SOAP request in flight.
    Searching,
    /// The router forwards `external_ip:external_port` to `lan_ip:<http_port>`.
    Mapped {
        external_ip: String,
        external_port: u16,
        gateway: String,
        lan_ip: String,
        lease_secs: u32,
        /// Unix seconds when the mapping was (re)established.
        since_secs: u64,
    },
    /// No UPnP-capable gateway answered the SSDP search.
    NoGateway,
    /// The router accepted the mapping but its own WAN address is private
    /// (RFC 1918 or RFC 6598 carrier-grade NAT): a second NAT sits between it
    /// and the internet, so the mapping cannot be reached from outside.
    DoubleNat { external_ip: String, external_port: u16 },
    /// The gateway refused or the request failed.
    Failed { error: String },
}

/// One "type this on the other device" line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ClientHint {
    pub label: String,
    pub value: String,
}

impl ClientHint {
    fn new(label: &str, value: impl Into<String>) -> Self {
        ClientHint { label: label.to_string(), value: value.into() }
    }
}

/// The whole reachability picture, as shipped in `GlobalSettingsMsg.reachability_json`.
/// Every text field is pre-rendered so UIs never need to know the enum semantics.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ReachabilityInfo {
    pub http_enabled: bool,
    pub port: u16,
    /// Sanitized stealth path (`"clay"` by default, empty = legacy `/`).
    pub web_path: String,
    /// Non-loopback IPv4 addresses of this machine, VPN addresses excluded.
    pub lan_ips: Vec<String>,
    /// `https://<lan_ip>:<port>/<web_path>/` for each of `lan_ips`.
    pub lan_urls: Vec<String>,
    /// Local addresses in 100.64.0.0/10 — on a *local* interface that range is
    /// almost always Tailscale or a similar mesh VPN, reachable from any device
    /// on the same VPN with no port forwarding at all.
    pub vpn_addrs: Vec<String>,
    /// From the router (UPnP `GetExternalIPAddress`) or an explicit lookup; never probed automatically.
    pub public_ip: Option<String>,
    /// `"upnp"`, `"lookup"`, or `""`.
    pub public_ip_source: String,
    pub public_ip_error: Option<String>,
    pub firewall: FirewallStatus,
    pub firewall_text: String,
    /// True when the server runs on Windows (the only platform with a firewall action).
    pub is_windows: bool,
    pub port_map_enabled: bool,
    pub port_map: PortMapStatus,
    pub port_map_text: String,
    pub client_hints: Vec<ClientHint>,
}

// ---------------------------------------------------------------------------
// Address classification
// ---------------------------------------------------------------------------

/// RFC 6598 carrier-grade NAT range, 100.64.0.0/10.
pub fn is_cgnat_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

/// Addresses that cannot be an internet-facing WAN address: RFC 1918 private
/// ranges, RFC 6598 CGNAT, link-local, loopback, unspecified. A router reporting
/// one of these as its external IP is itself behind another NAT.
pub fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private() || is_cgnat_v4(ip) || ip.is_link_local() || ip.is_loopback() || ip.is_unspecified()
}

// ---------------------------------------------------------------------------
// Pure formatting helpers
// ---------------------------------------------------------------------------

/// `https://<ip>:<port>/<web_path>/`, or `https://<ip>:<port>/` when the stealth
/// path is empty (legacy mode). An IPv6 literal is bracketed.
pub fn web_url(ip: &str, port: u16, web_path: &str) -> String {
    let host = if ip.contains(':') { format!("[{ip}]") } else { ip.to_string() };
    let path = web_path.trim_matches('/');
    if path.is_empty() {
        format!("https://{host}:{port}/")
    } else {
        format!("https://{host}:{port}/{path}/")
    }
}

/// `<ip>` when the port is the default 9000 (every Clay client appends it), else `<ip>:<port>`.
pub fn host_port_hint(ip: &str, port: u16) -> String {
    if port == 9000 { ip.to_string() } else { format!("{ip}:{port}") }
}

/// Human-readable one-liner for the firewall verdict.
pub fn firewall_text(status: &FirewallStatus) -> String {
    match status {
        FirewallStatus::NotApplicable => "n/a - Clay manages the firewall on Windows only".to_string(),
        FirewallStatus::Unchecked => "not checked yet".to_string(),
        FirewallStatus::Checking => "checking...".to_string(),
        FirewallStatus::Missing => {
            "no Windows Firewall rule for Clay - other devices are blocked; use Add Firewall Rule".to_string()
        }
        FirewallStatus::Allowed { program_matches: true } => {
            "allowed (inbound Windows Firewall rule present for this program)".to_string()
        }
        FirewallStatus::Allowed { program_matches: false } => {
            "rule exists but points at a different clay.exe location - use Add Firewall Rule to fix it".to_string()
        }
        FirewallStatus::Blocked => {
            "BLOCKED by a Windows Firewall rule (usually from clicking Cancel on the first-run alert); \
             use Add Firewall Rule to replace it"
                .to_string()
        }
        FirewallStatus::Unknown { detail } => format!("unknown ({detail})"),
    }
}

/// Human-readable one-liner for the router mapping state.
pub fn port_map_text(status: &PortMapStatus, enabled: bool) -> String {
    match status {
        PortMapStatus::Off if !enabled => "off (enable Port Mapping (UPnP) to forward the port)".to_string(),
        PortMapStatus::Off => "enabled, waiting for the web server to start".to_string(),
        PortMapStatus::Searching => "searching for a UPnP gateway...".to_string(),
        PortMapStatus::Mapped { external_ip, external_port, gateway, lan_ip, lease_secs, .. } => {
            let lease = if *lease_secs == 0 {
                "permanent lease".to_string()
            } else {
                format!("{}-minute lease, renewed automatically", lease_secs / 60)
            };
            format!("active via router {gateway}: {external_ip}:{external_port} -> {lan_ip} ({lease})")
        }
        PortMapStatus::NoGateway => {
            "no UPnP gateway answered - enable UPnP on the router, or forward the port to this machine manually"
                .to_string()
        }
        PortMapStatus::DoubleNat { external_ip, external_port } => format!(
            "mapped, but the router's WAN address {external_ip} is private: it sits behind another NAT \
             (or carrier-grade NAT), so the internet cannot reach port {external_port}. \
             Use an SSH tunnel (--ssh) or a VPN such as Tailscale instead"
        ),
        PortMapStatus::Failed { error } => format!("failed: {error}"),
    }
}

/// The "type this on the other device" lines, derived from the addresses and
/// mapping state. Order: browser, Clay GUI, Clay console, Android, then the
/// internet-facing variants when a working mapping exists, then VPN and SSH.
pub fn build_client_hints(info: &ReachabilityInfo) -> Vec<ClientHint> {
    let mut hints = Vec::new();
    if let Some(ip) = info.lan_ips.first() {
        hints.push(ClientHint::new("Browser (LAN)", web_url(ip, info.port, &info.web_path)));
        hints.push(ClientHint::new("Clay GUI", format!("clay --gui={}", host_port_hint(ip, info.port))));
        hints.push(ClientHint::new("Clay console", format!("clay --console={}", host_port_hint(ip, info.port))));
        hints.push(ClientHint::new("Android app", format!("Host {ip}, Port {}", info.port)));
    }
    if let PortMapStatus::Mapped { external_ip, external_port, .. } = &info.port_map {
        hints.push(ClientHint::new(
            "From the internet",
            format!("clay --gui={}", host_port_hint(external_ip, *external_port)),
        ));
        hints.push(ClientHint::new(
            "Browser (internet)",
            web_url(external_ip, *external_port, &info.web_path),
        ));
        hints.push(ClientHint::new(
            "Android (Remote Host)",
            format!("Remote Host {external_ip}, Port {external_port}"),
        ));
    } else if let PortMapStatus::DoubleNat { .. } = &info.port_map {
        hints.push(ClientHint::new(
            "From the internet",
            "not reachable this way - the router is itself behind NAT (see Router line)",
        ));
    }
    for vpn in &info.vpn_addrs {
        hints.push(ClientHint::new(
            "Via VPN (Tailscale)",
            format!("clay --gui={}", host_port_hint(vpn, info.port)),
        ));
    }
    hints.push(ClientHint::new("Via an SSH host", "clay --gui=user@host --ssh"));
    hints
}

/// Explains the last hint; kept out of the hint value so the web dialog's Copy
/// button copies a command, not a sentence.
pub const SSH_HINT_NOTE: &str = "(--ssh: any SSH login you have; no port forwarding needed)";

/// The console/TUI report. One string per output line; no trailing newline.
pub fn format_reach_lines(info: &ReachabilityInfo) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Remote access to this Clay".to_string());
    let path_display = if info.web_path.is_empty() { "/".to_string() } else { format!("/{}/", info.web_path) };
    if info.http_enabled {
        lines.push(format!("  Web server: on, port {}, path {}", info.port, path_display));
    } else {
        lines.push("  Web server: OFF (enable it in /web)".to_string());
    }
    if info.lan_urls.is_empty() {
        lines.push("  LAN:        no non-loopback IPv4 address found".to_string());
    } else {
        for (i, url) in info.lan_urls.iter().enumerate() {
            let label = if i == 0 { "  LAN:        " } else { "              " };
            lines.push(format!("{label}{url}"));
        }
    }
    if !info.vpn_addrs.is_empty() {
        lines.push(format!(
            "  VPN:        {}  (100.64/10 address - Tailscale or similar; reachable from that VPN without port forwarding)",
            info.vpn_addrs.join(", ")
        ));
    }
    let public = match (&info.public_ip, &info.public_ip_error) {
        (Some(ip), _) => match info.public_ip_source.as_str() {
            "upnp" => format!("{ip} (reported by the router)"),
            "lookup" => format!("{ip} (looked up)"),
            _ => ip.clone(),
        },
        (None, Some(err)) => format!("lookup failed: {err}"),
        (None, None) => "unknown (use Look Up Public IP or /reach --lookup)".to_string(),
    };
    lines.push(format!("  Public IP:  {public}"));
    lines.push(format!("  Firewall:   {}", info.firewall_text));
    lines.push(format!("  Router:     {}", info.port_map_text));
    lines.push("Connect from another device:".to_string());
    for hint in &info.client_hints {
        lines.push(format!("  {:<22} {}", format!("{}:", hint.label), hint.value));
    }
    lines.push(format!("  {SSH_HINT_NOTE}"));
    lines.push("See the Clay docs, Web Interface -> \"Reaching Clay from outside your network\".".to_string());
    lines
}

// ---------------------------------------------------------------------------
// Local address discovery
// ---------------------------------------------------------------------------

/// Every usable IPv4 address of this machine (loopback, link-local and
/// unspecified dropped, order preserved, de-duplicated). Starts from
/// `get_local_ip_addresses()` — the default-route address (plus `hostname -I`
/// on Linux) — and appends what `ipconfig` (Windows) / `ifconfig` (macOS) list,
/// so a second NIC or a Tailscale interface shows up there too.
///
/// Deliberately a *separate* function from `get_local_ip_addresses()`: that one
/// feeds the self-signed certificate's SANs and its change detection, and
/// widening it would regenerate the cert and trip every client's TOFU pin.
pub fn local_ipv4_addrs() -> Vec<String> {
    let mut raw: Vec<String> = crate::get_local_ip_addresses();
    raw.extend(platform_extra_v4());
    dedupe_usable_v4(raw)
}

/// Extra addresses from the platform's interface lister; empty on Linux, where
/// `get_local_ip_addresses()` already runs `hostname -I` and lists everything.
#[cfg(windows)]
fn platform_extra_v4() -> Vec<String> {
    crate::util::run_command_with_timeout("ipconfig", &[], 3)
        .map(|out| parse_ipconfig_v4(&out))
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn platform_extra_v4() -> Vec<String> {
    crate::util::run_command_with_timeout("ifconfig", &[], 3)
        .map(|out| parse_ifconfig_v4(&out))
        .unwrap_or_default()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_extra_v4() -> Vec<String> {
    Vec::new()
}

/// Keep parseable, non-loopback, non-link-local, non-unspecified IPv4 strings, once each.
pub fn dedupe_usable_v4(raw: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in raw {
        let s = s.trim();
        if let Ok(ip) = s.parse::<Ipv4Addr>() {
            if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
                continue;
            }
            let canon = ip.to_string();
            if !out.contains(&canon) {
                out.push(canon);
            }
        }
    }
    out
}

/// Pull IPv4 addresses out of `ipconfig` output. Localized Windows builds label
/// the line differently ("IPv4 Address", "IPv4-Adresse", "Adresse IPv4", ...)
/// but "IPv4" survives, so: any line containing `IPv4`, last dotted-quad token wins.
pub fn parse_ipconfig_v4(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|l| l.contains("IPv4"))
        .filter_map(|l| {
            l.split(|c: char| c.is_whitespace() || c == ':' || c == '(' || c == ')')
                .rfind(|t| t.parse::<Ipv4Addr>().is_ok())
                .map(str::to_string)
        })
        .collect()
}

/// Pull IPv4 addresses out of BSD/macOS `ifconfig` output: the token after `inet`.
pub fn parse_ifconfig_v4(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|l| {
            let mut toks = l.split_whitespace();
            while let Some(t) = toks.next() {
                if t == "inet" {
                    return toks.next().filter(|ip| ip.parse::<Ipv4Addr>().is_ok()).map(str::to_string);
                }
            }
            None
        })
        .collect()
}

/// Ask a public "what is my IP" service for our internet-facing address. Opt-in
/// only — it contacts a third party — hence a button / `/reach --lookup`, never a
/// timer. The reply must parse as an IP address or it is reported as garbage.
pub async fn lookup_public_ip() -> Result<String, String> {
    const URL: &str = "https://checkip.amazonaws.com";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;
    let body = client
        .get(URL)
        .send()
        .await
        .map_err(|e| format!("{e}"))?
        .text()
        .await
        .map_err(|e| format!("{e}"))?;
    let text = body.trim();
    text.parse::<std::net::IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| format!("unexpected reply from {URL}: {}", crate::util::truncate_str(text, 40)))
}

/// The local IPv4 address the OS would use to reach `target` — the UDP
/// `connect()` trick (no packet is sent). Used to pick the interface facing the
/// UPnP gateway, since a multi-homed host must hand the router *that* address.
pub fn lan_ip_toward(target: Ipv4Addr) -> Option<Ipv4Addr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect((target, 9)).ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn private_range_table() {
        for (addr, private) in [
            ("10.0.0.1", true),
            ("172.16.0.1", true),
            ("172.31.255.255", true),
            ("172.32.0.1", false),
            ("192.168.1.1", true),
            ("100.64.0.1", true),
            ("100.127.255.254", true),
            ("100.128.0.1", false),
            ("100.63.255.255", false),
            ("169.254.10.10", true),
            ("127.0.0.1", true),
            ("0.0.0.0", true),
            ("203.0.113.5", false),
            ("8.8.8.8", false),
        ] {
            assert_eq!(is_private_v4(ip(addr)), private, "{addr}");
        }
        assert!(is_cgnat_v4(ip("100.100.1.1")));
        assert!(!is_cgnat_v4(ip("10.0.0.1")));
    }

    #[test]
    fn web_url_forms() {
        assert_eq!(web_url("192.168.1.20", 9000, "clay"), "https://192.168.1.20:9000/clay/");
        assert_eq!(web_url("192.168.1.20", 9001, ""), "https://192.168.1.20:9001/");
        assert_eq!(web_url("192.168.1.20", 9000, "/mud/"), "https://192.168.1.20:9000/mud/");
        assert_eq!(web_url("fd00::1", 9000, "clay"), "https://[fd00::1]:9000/clay/");
    }

    #[test]
    fn host_port_hint_omits_default_port() {
        assert_eq!(host_port_hint("192.168.1.20", 9000), "192.168.1.20");
        assert_eq!(host_port_hint("192.168.1.20", 9001), "192.168.1.20:9001");
    }

    #[test]
    fn firewall_texts_are_distinct_and_actionable() {
        let blocked = firewall_text(&FirewallStatus::Blocked);
        assert!(blocked.contains("BLOCKED"));
        assert!(blocked.contains("Add Firewall Rule"));
        assert!(firewall_text(&FirewallStatus::Missing).contains("Add Firewall Rule"));
        assert!(firewall_text(&FirewallStatus::Allowed { program_matches: true }).starts_with("allowed"));
        assert!(firewall_text(&FirewallStatus::Allowed { program_matches: false }).contains("different"));
        assert!(firewall_text(&FirewallStatus::Unknown { detail: "netsh timed out".into() }).contains("netsh timed out"));
        assert!(firewall_text(&FirewallStatus::NotApplicable).contains("Windows only"));
    }

    fn mapped() -> PortMapStatus {
        PortMapStatus::Mapped {
            external_ip: "203.0.113.5".into(),
            external_port: 9000,
            gateway: "192.168.1.1".into(),
            lan_ip: "192.168.1.20".into(),
            lease_secs: 3600,
            since_secs: 1_700_000_000,
        }
    }

    fn base_info() -> ReachabilityInfo {
        let mut info = ReachabilityInfo {
            http_enabled: true,
            port: 9000,
            web_path: "clay".into(),
            lan_ips: vec!["192.168.1.20".into()],
            lan_urls: vec!["https://192.168.1.20:9000/clay/".into()],
            firewall: FirewallStatus::NotApplicable,
            ..Default::default()
        };
        info.firewall_text = firewall_text(&info.firewall);
        info
    }

    #[test]
    fn report_for_mapped_router() {
        let mut info = base_info();
        info.port_map_enabled = true;
        info.port_map = mapped();
        info.port_map_text = port_map_text(&info.port_map, true);
        info.public_ip = Some("203.0.113.5".into());
        info.public_ip_source = "upnp".into();
        info.client_hints = build_client_hints(&info);
        let lines = format_reach_lines(&info);
        let text = lines.join("\n");
        assert!(text.contains("Web server: on, port 9000, path /clay/"));
        assert!(text.contains("LAN:        https://192.168.1.20:9000/clay/"));
        assert!(text.contains("Public IP:  203.0.113.5 (reported by the router)"));
        assert!(text.contains("Router:     active via router 192.168.1.1: 203.0.113.5:9000 -> 192.168.1.20"));
        assert!(text.contains("clay --gui=192.168.1.20"));
        assert!(text.contains("From the internet:"));
        assert!(text.contains("clay --gui=203.0.113.5"));
        assert!(text.contains("Android app:"));
        assert!(text.contains("--ssh"));
        assert!(text.contains(SSH_HINT_NOTE));
        assert!(!text.contains("VPN:"));
        assert!(lines.iter().all(|l| !l.ends_with('\n')));
    }

    #[test]
    fn report_for_double_nat_names_the_problem() {
        let mut info = base_info();
        info.port_map_enabled = true;
        info.port_map = PortMapStatus::DoubleNat { external_ip: "100.70.1.2".into(), external_port: 9000 };
        info.port_map_text = port_map_text(&info.port_map, true);
        info.vpn_addrs = vec!["100.101.102.103".into()];
        info.client_hints = build_client_hints(&info);
        let text = format_reach_lines(&info).join("\n");
        assert!(text.contains("100.70.1.2 is private"));
        assert!(text.contains("carrier-grade NAT"));
        assert!(text.contains("Tailscale"));
        assert!(text.contains("VPN:        100.101.102.103"));
        assert!(text.contains("clay --gui=100.101.102.103"));
        assert!(text.contains("not reachable this way"));
        assert!(text.contains("Public IP:  unknown"));
    }

    #[test]
    fn report_when_web_server_off() {
        let mut info = base_info();
        info.http_enabled = false;
        info.port_map_text = port_map_text(&info.port_map, false);
        let text = format_reach_lines(&info).join("\n");
        assert!(text.contains("Web server: OFF"));
        assert!(text.contains("Router:     off (enable Port Mapping"));
    }

    #[test]
    fn port_map_texts() {
        assert!(port_map_text(&PortMapStatus::Off, true).contains("waiting"));
        assert!(port_map_text(&PortMapStatus::NoGateway, true).contains("manually"));
        assert!(port_map_text(&PortMapStatus::Failed { error: "boom".into() }, true).contains("boom"));
        let permanent = PortMapStatus::Mapped {
            external_ip: "1.2.3.4".into(),
            external_port: 9000,
            gateway: "g".into(),
            lan_ip: "l".into(),
            lease_secs: 0,
            since_secs: 0,
        };
        assert!(port_map_text(&permanent, true).contains("permanent"));
    }

    #[test]
    fn json_shape_is_internally_tagged() {
        let mut info = base_info();
        info.port_map = mapped();
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"port_map\":{\"kind\":\"Mapped\""));
        assert!(json.contains("\"firewall\":{\"kind\":\"NotApplicable\"}"));
        let back: ReachabilityInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.port_map, mapped());
        // An empty blob (older server) must still parse.
        let empty: ReachabilityInfo = serde_json::from_str("{}").unwrap_or_default();
        assert_eq!(empty.port_map, PortMapStatus::Off);
    }

    #[test]
    fn ipconfig_parsing_is_locale_tolerant() {
        let en = "Ethernet adapter Ethernet:\r\n\r\n   Connection-specific DNS Suffix  . : lan\r\n   \
                  IPv6 Address. . . . . . . . . . . : fd00::1\r\n   \
                  IPv4 Address. . . . . . . . . . . : 192.168.1.20\r\n   \
                  Subnet Mask . . . . . . . . . . . : 255.255.255.0\r\n\
                  Unknown adapter Tailscale:\r\n   IPv4 Address. . . . . . . . . . . : 100.101.102.103(Preferred)\r\n";
        assert_eq!(parse_ipconfig_v4(en), vec!["192.168.1.20", "100.101.102.103"]);
        let de = "   IPv4-Adresse  . . . . . . . . . . : 10.0.0.7\r\n   Subnetzmaske  . . . . . . . . . . : 255.0.0.0\r\n";
        assert_eq!(parse_ipconfig_v4(de), vec!["10.0.0.7"]);
        assert!(parse_ipconfig_v4("   Subnet Mask . . . : 255.255.255.0").is_empty());
    }

    #[test]
    fn ifconfig_parsing() {
        let out = "lo0: flags=8049<UP,LOOPBACK> mtu 16384\n\tinet 127.0.0.1 netmask 0xff000000\n\
                   en0: flags=8863<UP,BROADCAST> mtu 1500\n\tinet6 fe80::1%en0 prefixlen 64\n\
                   \tinet 192.168.1.30 netmask 0xffffff00 broadcast 192.168.1.255\n\
                   utun3: flags=8051<UP,POINTOPOINT> mtu 1280\n\tinet 100.101.102.104 --> 100.101.102.104 netmask 0xffffffff\n";
        assert_eq!(parse_ifconfig_v4(out), vec!["127.0.0.1", "192.168.1.30", "100.101.102.104"]);
    }

    #[test]
    fn dedupe_drops_loopback_and_duplicates() {
        let raw = vec![
            "192.168.1.20".to_string(),
            "127.0.0.1".to_string(),
            "169.254.1.1".to_string(),
            " 192.168.1.20 ".to_string(),
            "not-an-ip".to_string(),
            "0.0.0.0".to_string(),
            "10.0.0.5".to_string(),
        ];
        assert_eq!(dedupe_usable_v4(raw), vec!["192.168.1.20", "10.0.0.5"]);
    }

    #[test]
    fn lan_ip_toward_gives_a_v4_or_nothing() {
        // No network assumptions: either the OS picks a source address or it can't.
        if let Some(v4) = lan_ip_toward(ip("192.168.255.254")) {
            assert!(!v4.is_unspecified());
        }
    }
}
