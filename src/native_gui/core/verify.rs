//! `clay [--native-gui=host] --native-gui-verify[=SECONDS]`: run the native GUI's client
//! with no window for a while, then check it holds exactly what the server has.
//!
//! The Rust counterpart of `reference/ws-verify-client.py`, but driving the real client
//! core, so a pass says the GUI's own buffer is right, not just the wire:
//!
//! - **The stream**, as it arrives: a batch's `end_seq` matches its line count, a seq is
//!   never re-sent with different text, and never delivered twice.
//! - **The buffer**, at the end: lines in strictly ascending seq order with no repeats,
//!   one unbroken range of delivered seqs per world, and no gap given up (`ReportGap`).
//!   The client's other diagnostics (`ReportDuplicate`, `ReportOutOfOrder`,
//!   `ReportSeqMismatch`) are listed: they mean the server sent something twice or out of
//!   order and the client coped.
//! - **Against the server**: the newest lines the server holds, fetched at the end,
//!   compared seq by seq with the client's copy (text included).
//!
//! `CLAY_VERIFY_ACTIONS="5:drop;12:/reload"` schedules actions by second: `drop` closes
//! the connection from this side (the client reconnects and resumes), anything else is
//! sent to the server as a command.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures::channel::mpsc;

use super::client::{Effect, Input, Status};
use super::session::Session;
use crate::websocket::WsMessage;

/// Request ids from here up are the verifier's own (the core counts up from 1).
const OWN_REQUESTS: u64 = 1 << 62;
const FINAL_COUNT: usize = 5000;
const FINAL_WAIT: Duration = Duration::from_secs(15);

#[derive(Default)]
struct Tally {
    messages: usize,
    server_data: usize,
    connections: usize,
    span_mismatches: Vec<String>,
    refiled: Vec<String>,
    redelivered: usize,
    /// Diagnostics the client sent, by type.
    reports: HashMap<&'static str, usize>,
    /// Delivered text by (world, seq).
    texts: HashMap<(usize, u64), String>,
    /// `CLAY_VERIFY_VERBOSE`: print each diagnostic and every ServerData's seq span.
    verbose: bool,
}

impl Tally {
    fn stream(&mut self, msg: &WsMessage) {
        self.messages += 1;
        let WsMessage::ServerData { world_index, data, seq, end_seq, flush, .. } = msg else { return };
        self.server_data += 1;
        if self.verbose {
            println!("verify: ServerData world {world_index} seq {seq} end_seq {end_seq:?} flush {flush} data {:?}", data.chars().take(60).collect::<String>());
        }
        if *flush {
            self.texts.retain(|(w, _), _| w != world_index);
        }
        if *seq == 0 && end_seq.is_none() {
            return; // no real seq
        }
        let body = data.strip_suffix('\n').map(|d| d.strip_suffix('\r').unwrap_or(d)).unwrap_or(data);
        let lines: Vec<&str> = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        let end = end_seq.unwrap_or(seq + lines.len() as u64 - 1);
        if end + 1 - seq != lines.len() as u64 {
            self.span_mismatches.push(format!("world {world_index} seq {seq}..={end} carries {} lines", lines.len()));
        }
        for (k, text) in lines.iter().enumerate() {
            match self.texts.insert((*world_index, seq + k as u64), text.to_string()) {
                Some(old) if old != *text => self.refiled.push(format!("world {world_index} seq {}: {old:?} then {text:?}", seq + k as u64)),
                Some(_) => self.redelivered += 1,
                None => {}
            }
        }
    }

    fn effects(&mut self, fx: &[Effect]) {
        for e in fx {
            let Effect::Send(m) = e else { continue };
            if self.verbose && matches!(**m, WsMessage::ReportGap { .. } | WsMessage::ReportDuplicate { .. } | WsMessage::ReportSeqMismatch { .. } | WsMessage::ReportOutOfOrder { .. }) {
                println!("verify: client sent {m:?}");
            }
            let kind = match &**m {
                WsMessage::ReportGap { .. } => "ReportGap",
                WsMessage::ReportDuplicate { .. } => "ReportDuplicate",
                WsMessage::ReportSeqMismatch { .. } => "ReportSeqMismatch",
                WsMessage::ReportOutOfOrder { .. } => "ReportOutOfOrder",
                _ => continue,
            };
            *self.reports.entry(kind).or_default() += 1;
        }
    }
}

/// Run for `secs`, check, report. Returns the process exit code.
pub fn run(mut session: Session, mut events: mpsc::UnboundedReceiver<Input>, secs: u64) -> i32 {
    let start = Instant::now();
    let mut actions = scheduled_actions();
    let mut tally = Tally { verbose: std::env::var_os("CLAY_VERIFY_VERBOSE").is_some(), ..Tally::default() };
    let mut finals: HashMap<usize, Vec<crate::websocket::TimestampedLine>> = HashMap::new();
    let mut final_asked: Option<(Instant, usize)> = None;
    let _ = session.start();
    println!("verify: {} for {secs} s", session.target.key());
    loop {
        match events.try_recv() {
            Ok(ev) => {
                if matches!(ev, Input::Opened) {
                    tally.connections += 1;
                }
                if let Input::Message(m) = &ev {
                    if let WsMessage::ScrollbackLines { world_index, lines, request_id: Some(id), .. } = &**m {
                        if *id >= OWN_REQUESTS {
                            finals.insert(*world_index, lines.clone());
                            continue;
                        }
                    }
                    tally.stream(m);
                }
                if let Input::Closed(reason) = &ev {
                    println!("verify: connection closed at {:.1} s: {reason}", start.elapsed().as_secs_f32());
                    // Final-check requests sent into a closing connection: ask again once
                    // it's back.
                    if final_asked.is_some() && finals.len() < session.core.worlds.len() {
                        final_asked = None;
                        finals.clear();
                    }
                }
                let fx = session.core.handle(ev, Instant::now());
                tally.effects(&fx);
                let _ = session.apply(fx);
            }
            Err(mpsc::TryRecvError::Closed) => break,
            Err(mpsc::TryRecvError::Empty) => {
                let fx = session.core.tick(Instant::now());
                tally.effects(&fx);
                let _ = session.apply(fx);
                match session.core.status() {
                    Status::NeedLogin { error, .. } => return fail(&format!("login needed: {}", error.clone().unwrap_or_else(|| "set CLAY_PASSWORD".into()))),
                    Status::CertMismatch(m) => return fail(&format!("the certificate of {} has changed (refused)", m.host)),
                    Status::Stopped(why) => return fail(why),
                    _ => {}
                }
                let elapsed = start.elapsed();
                while actions.first().is_some_and(|(at, _)| elapsed >= *at) {
                    let (_, action) = actions.remove(0);
                    println!("verify: {:.1} s: {action}", elapsed.as_secs_f32());
                    if action == "drop" {
                        let _ = session.apply(vec![Effect::Disconnect]);
                    } else {
                        let world_index = session.core.current;
                        let _ = session.send(WsMessage::SendCommand { world_index, command: action });
                    }
                }
                match final_asked {
                    None if elapsed >= Duration::from_secs(secs) && session.core.is_online() => {
                        for i in 0..session.core.worlds.len() {
                            let _ = session.send(WsMessage::RequestScrollback {
                                world_index: i,
                                count: FINAL_COUNT,
                                before_seq: None,
                                after_seq: None,
                                request_id: Some(OWN_REQUESTS + i as u64),
                            });
                        }
                        final_asked = Some((Instant::now(), session.core.worlds.len()));
                    }
                    Some((at, n)) if finals.len() >= n || at.elapsed() >= FINAL_WAIT => break,
                    _ => {}
                }
                if elapsed >= Duration::from_secs(secs) + FINAL_WAIT * 2 {
                    return fail("never got back online to check");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    report(&session, &tally, &finals)
}

fn scheduled_actions() -> Vec<(Duration, String)> {
    let spec = std::env::var("CLAY_VERIFY_ACTIONS").unwrap_or_default();
    let mut out: Vec<(Duration, String)> = spec
        .split(';')
        .filter_map(|item| {
            let (at, what) = item.split_once(':')?;
            Some((Duration::from_secs_f32(at.trim().parse().ok()?), what.trim().to_string()))
        })
        .collect();
    out.sort_by_key(|a| a.0);
    out
}

fn report(session: &Session, t: &Tally, finals: &HashMap<usize, Vec<crate::websocket::TimestampedLine>>) -> i32 {
    let mut problems: Vec<String> = Vec::new();
    println!(
        "verify: {} messages, {} ServerData, {} connection(s), {} re-delivered seq(s)",
        t.messages, t.server_data, t.connections, t.redelivered
    );
    problems.extend(t.span_mismatches.iter().take(10).map(|s| format!("span mismatch: {s}")));
    problems.extend(t.refiled.iter().take(10).map(|s| format!("seq refiled: {s}")));
    // A gap given up is output lost for good. A duplicate dropped or a late batch put in
    // its place is the client doing its job on a server that sent something twice or out
    // of order: worth showing, not a failure (the buffer checks below catch real damage).
    for (kind, n) in &t.reports {
        if *kind == "ReportGap" {
            problems.push(format!("the client gave up {n} gap(s) (ReportGap)"));
        } else {
            println!("verify: note: the client sent {n} {kind} (handled)");
        }
    }
    for (i, w) in session.core.worlds.iter().enumerate() {
        let seqs: Vec<u64> = w.lines.iter().filter_map(|l| l.seq).collect();
        let ordered = seqs.windows(2).all(|p| p[0] < p[1]);
        let ranges = w.seen.ranges();
        let mut line = format!("verify: world {i} {:?}: {} lines, delivered {:?}", w.name, w.lines.len(), ranges.iter().take(4).collect::<Vec<_>>());
        if !ordered {
            problems.push(format!("world {i}: lines out of seq order or repeated"));
        }
        let unnumbered: Vec<&str> = w.lines.iter().filter(|l| l.seq.is_none()).map(|l| l.text.as_str()).collect();
        if !unnumbered.is_empty() {
            line.push_str(&format!(", {} without a seq", unnumbered.len()));
            if t.verbose {
                for text in unnumbered.iter().take(12) {
                    println!("verify:   unnumbered: {text:?}");
                }
            }
        }
        if ranges.len() > 1 {
            problems.push(format!("world {i}: holes in the delivered seqs: {:?}", &ranges[..ranges.len().min(6)]));
        }
        if let Some(s) = seqs.iter().find(|s| !w.seen.contains(**s)) {
            problems.push(format!("world {i}: holds seq {s} it doesn't record as delivered"));
        }
        match finals.get(&i) {
            Some(server) => {
                let held: HashMap<u64, &str> = w.lines.iter().filter_map(|l| Some((l.seq?, l.text.as_str()))).collect();
                let oldest = seqs.first().copied().unwrap_or(u64::MAX);
                let newest = w.seen.max_seen();
                let (mut missing, mut differ, mut compared) = (0, 0, 0);
                let mut first_bad = None;
                for l in server.iter().filter(|l| !l.from_archive && l.seq >= oldest && l.seq <= newest) {
                    compared += 1;
                    match held.get(&l.seq) {
                        None => {
                            missing += 1;
                            first_bad.get_or_insert(format!("seq {} missing: {:?}", l.seq, l.text));
                        }
                        Some(text) if *text != l.text.trim_end_matches('\r') && *text != l.text => {
                            differ += 1;
                            first_bad.get_or_insert(format!("seq {}: server {:?}, client {:?}", l.seq, l.text, text));
                        }
                        _ => {}
                    }
                }
                line.push_str(&format!(", {compared} checked against the server"));
                if t.verbose {
                    let theirs: std::collections::HashSet<u64> = server.iter().map(|l| l.seq).collect();
                    let server_max = server.iter().map(|l| l.seq).max();
                    let extra: Vec<(u64, &str)> = w.lines.iter().filter_map(|l| Some((l.seq?, l.text.as_str()))).filter(|(s, _)| !theirs.contains(s)).take(12).collect();
                    println!("verify:   server reply: {} lines, newest seq {server_max:?}; held but not in it: {extra:?}", server.len());

                }
                if missing + differ > 0 {
                    problems.push(format!("world {i}: {missing} line(s) missing, {differ} different ({})", first_bad.unwrap_or_default()));
                }
            }
            None => problems.push(format!("world {i}: the server never answered the final check")),
        }
        println!("{line}");
    }
    if problems.is_empty() {
        println!("VERIFY OK");
        0
    } else {
        for p in &problems {
            println!("  - {p}");
        }
        println!("VERIFY FAIL ({} problem(s))", problems.len());
        1
    }
}

fn fail(why: &str) -> i32 {
    println!("VERIFY FAIL: {why}");
    1
}
