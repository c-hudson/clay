//! Windows Defender Firewall integration — the "Add Firewall Rule" half of the
//! Remote Access feature (reach.rs). Windows blocks inbound connections by
//! default, so other devices cannot reach Clay's web server until a rule allows
//! this executable. Two pieces:
//!
//! * [`query_firewall_status`] — non-elevated, fast: lists every inbound rule
//!   with `netsh` and decides *what wins for this executable*. That is deliberately
//!   not "does Clay's own named rule exist": when a user clicks Cancel on Windows'
//!   first-run "allow this app?" alert, Windows creates a **Block** rule named
//!   after the exe, and a Block rule beats any Allow rule — so it must be reported
//!   as `Blocked` even if our Allow rule is also there.
//! * [`add_firewall_rule`] — elevated, one UAC prompt: deletes every inbound rule
//!   for this executable (that stale Block rule included) and adds a per-program
//!   Allow rule. Per-program rather than per-port so a port change never breaks
//!   it; `profile=any` because home networks are frequently classified "Public",
//!   and Clay's stealth path, password, allow list and ban list are the real gate.
//!
//! Everything but the two `#[cfg(windows)]` entry points is platform-independent
//! and unit-tested on Linux; the Win32 calls are hand-rolled `extern "system"`
//! declarations in the same style as `platform.rs` (no `windows` crate).

use crate::reach::FirewallStatus;

/// Name of the inbound rule Clay creates.
pub const RULE_NAME: &str = "Clay MUD Client";

/// Words `netsh` uses for an enabled/allowing rule in the Windows display
/// languages we know. Matching is case-insensitive and best-effort: anything
/// unrecognised degrades to `Unknown`, never to a false `Allowed`.
const YES_WORDS: &[&str] = &["yes", "ja", "oui", "sí", "si", "sì", "sim"];
const NO_WORDS: &[&str] = &["no", "nein", "non", "nee", "não", "nao"];
const ALLOW_WORDS: &[&str] = &["allow", "zulassen", "autoriser", "permitir", "consenti", "toestaan"];
const BLOCK_WORDS: &[&str] = &["block", "blockieren", "bloquer", "bloquear", "blocca", "blokkeren"];

/// Decide the firewall verdict for `exe_path` from the output of
/// `netsh advfirewall firewall show rule name=all dir=in verbose`.
///
/// Language-tolerant by construction: a rule "mentions" the executable when any
/// line's value equals the path (compared case-insensitively with `/` and `\`
/// unified), which needs no field labels at all. Within such a rule block the
/// *first* yes/no-valued line is `Enabled:` (netsh prints fields in a fixed
/// order) and any allow/block-valued line is `Action:`.
pub fn parse_netsh_rules(output: &str, exe_path: &str) -> FirewallStatus {
    let target = normalize_path(exe_path);
    if target.is_empty() {
        return FirewallStatus::Unknown { detail: "executable path unknown".to_string() };
    }
    let mut saw_allow = false;
    let mut saw_block = false;
    let mut saw_unparsed = false;
    let output = output.replace('\r', "");
    for block in output.split("\n\n") {
        let mentions = block.lines().any(|l| value_of(l).map(|v| normalize_path(v) == target).unwrap_or(false));
        if !mentions {
            continue;
        }
        let enabled = block
            .lines()
            .filter_map(value_of)
            .find_map(|v| {
                let v = v.to_lowercase();
                if YES_WORDS.contains(&v.as_str()) { Some(true) } else if NO_WORDS.contains(&v.as_str()) { Some(false) } else { None }
            })
            .unwrap_or(true);
        if !enabled {
            continue;
        }
        let action = block.lines().filter_map(value_of).find_map(|v| {
            let v = v.to_lowercase();
            if ALLOW_WORDS.contains(&v.as_str()) { Some(true) } else if BLOCK_WORDS.contains(&v.as_str()) { Some(false) } else { None }
        });
        match action {
            Some(true) => saw_allow = true,
            Some(false) => saw_block = true,
            None => saw_unparsed = true,
        }
    }
    if saw_block {
        FirewallStatus::Blocked
    } else if saw_allow {
        FirewallStatus::Allowed { program_matches: true }
    } else if saw_unparsed {
        FirewallStatus::Unknown {
            detail: "a rule for this program exists but its Action could not be read (unsupported Windows display language?)".to_string(),
        }
    } else {
        FirewallStatus::Missing
    }
}

/// `"Label:   value"` → `Some("value")` (trimmed); lines without a colon → `None`.
/// A drive letter also has a colon, so the split is on the *first* colon that is
/// followed by whitespace or end-of-line, which is how netsh lays labels out.
fn value_of(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut idx = None;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' && bytes.get(i + 1).map(|c| c.is_ascii_whitespace()).unwrap_or(true) {
            idx = Some(i);
            break;
        }
    }
    let i = idx?;
    Some(line[i + 1..].trim())
}

fn normalize_path(p: &str) -> String {
    p.trim().trim_matches('"').replace('/', "\\").to_lowercase()
}

/// Query the verdict for the running executable. Windows only; elsewhere
/// `NotApplicable`. Synchronous (about a second on a machine with many rules) —
/// call it from `spawn_blocking`.
#[cfg(windows)]
pub fn query_firewall_status() -> FirewallStatus {
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => return FirewallStatus::Unknown { detail: format!("cannot determine executable path: {e}") },
    };
    match crate::util::run_command_capture(
        &netsh_path(),
        &["advfirewall", "firewall", "show", "rule", "name=all", "dir=in", "verbose"],
        15,
    ) {
        None => FirewallStatus::Unknown { detail: "netsh unavailable or timed out".to_string() },
        Some((code, out)) if out.trim().is_empty() && code != 0 && code != 1 => {
            FirewallStatus::Unknown { detail: format!("netsh exited with code {code}") }
        }
        Some((_, out)) => parse_netsh_rules(&out, &exe),
    }
}

#[cfg(not(windows))]
pub fn query_firewall_status() -> FirewallStatus {
    FirewallStatus::NotApplicable
}

/// Replace whatever inbound rules mention this executable with one per-program
/// Allow rule, through a single UAC prompt. Blocks until the elevated `netsh`
/// finishes (call from `spawn_blocking`). Windows only.
#[cfg(windows)]
pub fn add_firewall_rule() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot determine executable path: {e}"))?;
    let exe = exe.to_string_lossy().to_string();
    if exe.contains('"') {
        return Err("executable path contains a quote character".to_string());
    }
    let netsh = netsh_path();
    // `&`, not `&&`: `delete rule` exits 1 when nothing matched, which is the
    // normal first-run case, and the add must still run.
    let params = format!(
        "/C {netsh} advfirewall firewall delete rule name=all dir=in program=\"{exe}\" & \
         {netsh} advfirewall firewall add rule name=\"{RULE_NAME}\" dir=in action=allow \
         program=\"{exe}\" protocol=TCP enable=yes profile=any"
    );
    run_elevated(&cmd_path(), &params)
}

#[cfg(not(windows))]
pub fn add_firewall_rule() -> Result<(), String> {
    Err("Windows only".to_string())
}

/// Full paths so an elevated launch never resolves `cmd`/`netsh` through PATH.
#[cfg(windows)]
fn system32(file: &str) -> String {
    match std::env::var("SystemRoot") {
        Ok(root) if !root.trim().is_empty() => format!("{}\\System32\\{file}", root.trim_end_matches('\\')),
        _ => file.to_string(),
    }
}
#[cfg(windows)]
fn netsh_path() -> String {
    system32("netsh.exe")
}
#[cfg(windows)]
fn cmd_path() -> String {
    system32("cmd.exe")
}

/// `ShellExecuteExW` with the `runas` verb: one UAC prompt, hidden window, wait
/// for exit, report the exit code. Hand-rolled Win32 declarations (platform.rs
/// style — no `windows` crate).
#[cfg(windows)]
fn run_elevated(file: &str, params: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    // SHELLEXECUTEINFOW, 112 bytes on x64 (repr(C) reproduces the padding after
    // nShow and dwHotKey; hIcon/hMonitor share one union slot).
    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: isize,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: isize,
        lp_id_list: *mut core::ffi::c_void,
        lp_class: *const u16,
        hkey_class: isize,
        dw_hot_key: u32,
        h_icon_or_monitor: isize,
        h_process: isize,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, co_init: u32) -> i32;
        fn CoUninitialize();
    }
    extern "system" {
        fn WaitForSingleObject(handle: isize, millis: u32) -> u32;
        fn GetExitCodeProcess(handle: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetLastError() -> u32;
    }
    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
    const SEE_MASK_FLAG_NO_UI: u32 = 0x0000_0400;
    const SW_HIDE: i32 = 0;
    const WAIT_OBJECT_0: u32 = 0;
    const ERROR_CANCELLED: u32 = 1223;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const COINIT_DISABLE_OLE1DDE: u32 = 0x4;
    const TIMEOUT_MS: u32 = 120_000;

    let verb = wide("runas");
    let file_w = wide(file);
    let params_w = wide(params);
    // SAFETY: plain Win32 calls with valid, NUL-terminated wide strings that
    // outlive the call; the handle is closed on every path.
    unsafe {
        // ShellExecuteEx wants COM initialised on the calling thread. S_OK/S_FALSE
        // mean we own the init; RPC_E_CHANGED_MODE (negative) means someone else
        // already did in another mode — carry on, but don't uninitialise theirs.
        let co_owned = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) >= 0;
        let mut info = ShellExecuteInfoW {
            cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
            f_mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            hwnd: 0,
            lp_verb: verb.as_ptr(),
            lp_file: file_w.as_ptr(),
            lp_parameters: params_w.as_ptr(),
            lp_directory: std::ptr::null(),
            n_show: SW_HIDE,
            h_inst_app: 0,
            lp_id_list: std::ptr::null_mut(),
            lp_class: std::ptr::null(),
            hkey_class: 0,
            dw_hot_key: 0,
            h_icon_or_monitor: 0,
            h_process: 0,
        };
        let launched = ShellExecuteExW(&mut info) != 0;
        let result = if !launched {
            let err = GetLastError();
            if err == ERROR_CANCELLED {
                Err("UAC prompt declined".to_string())
            } else {
                Err(format!("ShellExecuteEx failed (Windows error {err})"))
            }
        } else if info.h_process == 0 {
            Err("elevated process handle unavailable".to_string())
        } else {
            let waited = WaitForSingleObject(info.h_process, TIMEOUT_MS);
            let r = if waited != WAIT_OBJECT_0 {
                Err("timed out waiting for netsh".to_string())
            } else {
                let mut code: u32 = 0;
                if GetExitCodeProcess(info.h_process, &mut code) == 0 {
                    Err("could not read the netsh exit code".to_string())
                } else if code != 0 {
                    Err(format!("netsh exited with code {code}"))
                } else {
                    Ok(())
                }
            };
            CloseHandle(info.h_process);
            r
        };
        if co_owned {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\Alice\clay\clay.exe";

    fn rule(name: &str, enabled: &str, program: &str, action: &str) -> String {
        format!(
            "Rule Name:                            {name}\r\n\
             ----------------------------------------------------------------------\r\n\
             Enabled:                              {enabled}\r\n\
             Direction:                            In\r\n\
             Profiles:                             Domain,Private,Public\r\n\
             Grouping:                             \r\n\
             LocalIP:                              Any\r\n\
             RemoteIP:                             Any\r\n\
             Protocol:                             TCP\r\n\
             LocalPort:                            Any\r\n\
             RemotePort:                           Any\r\n\
             Edge traversal:                       No\r\n\
             Program:                              {program}\r\n\
             InterfaceTypes:                       Any\r\n\
             Security:                             NotRequired\r\n\
             Rule source:                          Local Setting\r\n\
             Action:                               {action}\r\n\r\n"
        )
    }

    #[test]
    fn allow_only_is_allowed() {
        let out = rule("Core Networking - DNS", "Yes", "System", "Allow")
            + &rule(RULE_NAME, "Yes", EXE, "Allow");
        assert_eq!(parse_netsh_rules(&out, EXE), FirewallStatus::Allowed { program_matches: true });
    }

    #[test]
    fn auto_block_rule_beats_our_allow_rule() {
        // Clicking Cancel on the first-run alert leaves a Block rule named after the exe.
        let out = rule(RULE_NAME, "Yes", EXE, "Allow") + &rule("clay.exe", "Yes", EXE, "Block");
        assert_eq!(parse_netsh_rules(&out, EXE), FirewallStatus::Blocked);
    }

    #[test]
    fn disabled_block_rule_does_not_count() {
        let out = rule(RULE_NAME, "Yes", EXE, "Allow") + &rule("clay.exe", "No", EXE, "Block");
        assert_eq!(parse_netsh_rules(&out, EXE), FirewallStatus::Allowed { program_matches: true });
    }

    #[test]
    fn no_rule_for_us_is_missing() {
        let out = rule("Core Networking - DNS", "Yes", "System", "Allow")
            + &rule("Other app", "Yes", r"C:\Program Files\Other\other.exe", "Block");
        assert_eq!(parse_netsh_rules(&out, EXE), FirewallStatus::Missing);
        assert_eq!(parse_netsh_rules("\r\nNo rules match the specified criteria.\r\n", EXE), FirewallStatus::Missing);
        assert_eq!(parse_netsh_rules("", EXE), FirewallStatus::Missing);
    }

    #[test]
    fn path_comparison_ignores_case_and_slashes() {
        let out = rule(RULE_NAME, "Yes", r"c:\users\alice\CLAY\clay.exe", "Allow");
        assert_eq!(parse_netsh_rules(&out, "C:/Users/Alice/clay/clay.exe"), FirewallStatus::Allowed { program_matches: true });
    }

    #[test]
    fn localized_output_is_understood_or_degrades_to_unknown() {
        let de = format!(
            "Regelname:                            {RULE_NAME}\r\n\
             ----------------------------------------------------------------------\r\n\
             Aktiviert:                            Ja\r\n\
             Richtung:                             Eingehend\r\n\
             Profile:                              Domäne,Privat,Öffentlich\r\n\
             Programm:                             {EXE}\r\n\
             Aktion:                               Zulassen\r\n\r\n"
        );
        assert_eq!(parse_netsh_rules(&de, EXE), FirewallStatus::Allowed { program_matches: true });
        let unknown_lang = format!("Foo:   Igen\r\nBar:   {EXE}\r\nBaz:   Engedélyez\r\n\r\n");
        assert!(matches!(parse_netsh_rules(&unknown_lang, EXE), FirewallStatus::Unknown { .. }));
    }

    #[test]
    fn value_of_handles_drive_letters() {
        assert_eq!(value_of(r"Program:    C:\x\clay.exe"), Some(r"C:\x\clay.exe"));
        assert_eq!(value_of("Enabled:"), Some(""));
        assert_eq!(value_of("no colon here"), None);
    }
}
