//! The parity checklist (`PARITY.toml`) against its sources of truth: every message the
//! web client handles, every action its dispatcher runs and every `WsMessage` variant
//! has a row, and no row names something that's gone. Like `test_command_parity_js_vs_rust`
//! for the console, this is what keeps the native GUI from silently missing a feature the
//! web GUI gains.

use std::collections::{BTreeMap, BTreeSet};

const PARITY: &str = include_str!("PARITY.toml");
const APP_JS: &str = include_str!("../web/app.js");
const WEBSOCKET_RS: &str = include_str!("../websocket.rs");

const SECTIONS: &[&str] =
    &["connection", "output", "status", "navigation", "input", "dialogs", "windows", "editors", "media", "settings", "misc", "wire"];
const STATUSES: &[&str] = &["done", "todo", "waived", "see"];

#[derive(Debug, Default, Clone)]
pub(crate) struct Item {
    pub id: String,
    pub section: String,
    pub phase: u32,
    pub status: String,
    pub note: String,
}

/// `[[item]]` blocks of `key = "string"` / `key = number` lines.
pub(crate) fn items() -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for line in PARITY.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[item]]" {
            out.push(Item::default());
            continue;
        }
        let (k, v) = line.split_once('=').unwrap_or_else(|| panic!("PARITY.toml: not key = value: {line}"));
        let (k, v) = (k.trim(), v.trim());
        let item = out.last_mut().expect("a key before the first [[item]]");
        let text = || v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or_else(|| panic!("{k} wants a string: {line}")).to_string();
        match k {
            "id" => item.id = text(),
            "section" => item.section = text(),
            "status" => item.status = text(),
            "note" => item.note = text(),
            "feature" | "source" | "test" => {}
            "phase" => item.phase = v.parse().unwrap_or_else(|_| panic!("phase wants a number: {line}")),
            _ => panic!("PARITY.toml: unknown key {k}"),
        }
    }
    out
}

/// The `case '...':` labels in an app.js function (its end found by indentation, since
/// braces inside strings and regexes defeat counting them).
fn js_cases(name: &str) -> BTreeSet<String> {
    let lines: Vec<&str> = APP_JS.lines().collect();
    let start = lines.iter().position(|l| l.trim_start().starts_with(&format!("function {name}("))).expect(name);
    let indent = lines[start].len() - lines[start].trim_start().len();
    let close = format!("{}}}", " ".repeat(indent));
    let end = (start + 1..lines.len()).find(|&i| lines[i].trim_end() == close).expect("function end");
    let re = regex::Regex::new(r"case\s+'([A-Za-z0-9_]+)'\s*:").unwrap();
    lines[start..=end].iter().flat_map(|l| re.captures_iter(l).map(|c| c[1].to_string()).collect::<Vec<_>>()).collect()
}

fn ws_variants() -> BTreeSet<String> {
    let start = WEBSOCKET_RS.find("pub enum WsMessage {").expect("WsMessage");
    let end = start + WEBSOCKET_RS[start..].find("\n}\n").expect("end of WsMessage");
    let re = regex::Regex::new(r"(?m)^    ([A-Z][A-Za-z0-9]*)\b").unwrap();
    re.captures_iter(&WEBSOCKET_RS[start..end]).map(|c| c[1].to_string()).collect()
}

#[test]
fn every_web_message_action_and_wire_variant_has_a_row() {
    let items = items();
    let ids: BTreeMap<&str, &Item> = items.iter().map(|i| (i.id.as_str(), i)).collect();
    assert_eq!(ids.len(), items.len(), "duplicate ids in PARITY.toml");
    let expected: BTreeSet<String> = js_cases("handleMessage")
        .into_iter()
        .map(|m| format!("msg:{m}"))
        .chain(js_cases("dispatchActionImpl").into_iter().map(|a| format!("action:{a}")))
        .chain(ws_variants().into_iter().map(|v| format!("wire:{v}")))
        .collect();
    let missing: Vec<&String> = expected.iter().filter(|id| !ids.contains_key(id.as_str())).collect();
    assert!(missing.is_empty(), "the web GUI does these and PARITY.toml has no row for them: {missing:?}");
    let stale: Vec<&str> = ids
        .keys()
        .filter(|id| ["msg:", "action:", "wire:"].iter().any(|p| id.starts_with(p)) && !expected.contains(**id))
        .copied()
        .collect();
    assert!(stale.is_empty(), "PARITY.toml rows for things that no longer exist: {stale:?}");
    for i in &items {
        assert!(SECTIONS.contains(&i.section.as_str()), "{}: unknown section {}", i.id, i.section);
        assert!(STATUSES.contains(&i.status.as_str()), "{}: unknown status {}", i.id, i.status);
        assert!(i.status != "waived" || !i.note.is_empty(), "{}: a waived row says why", i.id);
    }
}

/// The phase gate: run with `cargo test parity_phase_gate -- --ignored` (PHASE=n,
/// default the current phase) at the end of a phase. Everything that phase or an earlier
/// one promised must be done (or waived).
#[test]
#[ignore]
fn parity_phase_gate() {
    let phase: u32 = std::env::var("PHASE").ok().and_then(|p| p.parse().ok()).unwrap_or(2);
    let open: Vec<String> = items()
        .into_iter()
        .filter(|i| i.phase >= 1 && i.phase <= phase && i.status == "todo")
        .map(|i| format!("{} ({}, phase {})", i.id, i.section, i.phase))
        .collect();
    assert!(open.is_empty(), "{} item(s) due by phase {phase} still open:\n{}", open.len(), open.join("\n"));
}
