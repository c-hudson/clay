//! `ClientCore` against scripted server messages: the web client's rules, one at a time.

use super::*;
use crate::websocket::{TimestampedLine, WorldStateMsg, WsMessage};

const CHALLENGE: &str = "abc123";

/// A world as the real server describes it (`App::build_initial_state`), so these tests
/// can't drift from the wire format; then the parts a test cares about.
fn world_msg(index: usize, name: &str, seqs: std::ops::Range<u64>, epoch: u64) -> WorldStateMsg {
    let mut app = crate::App::new();
    app.worlds = vec![crate::World::new(name)];
    let WsMessage::InitialState { mut worlds, .. } = app.build_initial_state(0) else { panic!("InitialState") };
    let mut w = worlds.remove(0);
    w.index = index;
    w.output_lines_ts = seqs.clone().map(|s| tl(s, &format!("line {s}"))).collect();
    w.total_output_lines = w.output_lines_ts.len();
    w.total_visible_lines = Some(w.output_lines_ts.len());
    w.seq_epoch = epoch;
    w.next_seq = seqs.end;
    w
}

fn settings() -> crate::websocket::GlobalSettingsMsg {
    crate::App::new().build_global_settings_msg()
}

fn tl(seq: u64, text: &str) -> TimestampedLine {
    TimestampedLine {
        text: text.into(),
        ts: 1_000,
        gagged: false,
        from_server: true,
        seq,
        highlight_color: None,
        from_archive: false,
        archive_sourced: false,
        viewed: true,
        display_id: None,
    }
}

fn data(world: usize, seq: u64, text: &str) -> WsMessage {
    let lines = text.split_terminator('\n').count().max(1) as u64;
    WsMessage::ServerData {
        world_index: world,
        data: text.into(),
        is_viewed: true,
        ts: 1_000,
        from_server: true,
        seq,
        end_seq: Some(seq + lines - 1),
        flush: false,
        gagged: false,
        archive_sourced: false,
        highlight_colors: Vec::new(),
    }
}

struct H {
    core: ClientCore,
    now: Instant,
    effects: Vec<Effect>,
}

impl H {
    fn new(creds: Credentials) -> Self {
        let now = Instant::now();
        H { core: ClientCore::new(creds, "uid-1".into(), now), now, effects: Vec::new() }
    }

    fn password() -> Self {
        Self::new(Credentials { password: Some("pw".into()), ..Default::default() })
    }

    fn keep(&mut self, e: Vec<Effect>) -> &mut Self {
        self.effects.extend(e);
        self
    }

    fn input(&mut self, i: Input) -> &mut Self {
        let e = self.core.handle(i, self.now);
        self.keep(e)
    }

    fn msg(&mut self, m: WsMessage) -> &mut Self {
        self.input(Input::Message(Box::new(m)))
    }

    fn advance(&mut self, d: Duration) -> &mut Self {
        self.now += d;
        let e = self.core.tick(self.now);
        self.keep(e)
    }

    /// Everything sent since the last call.
    fn sent(&mut self) -> Vec<WsMessage> {
        std::mem::take(&mut self.effects)
            .into_iter()
            .filter_map(|e| if let Effect::Send(m) = e { Some(*m) } else { None })
            .collect()
    }

    fn drain(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn connect_and_login(&mut self) -> &mut Self {
        let e = self.core.start(self.now);
        self.keep(e);
        self.input(Input::Opened);
        self.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: CHALLENGE.into() });
        self.msg(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false })
    }

    fn initial(&mut self, worlds: Vec<WorldStateMsg>) -> &mut Self {
        self.msg(WsMessage::InitialState {
            worlds,
            settings: settings(),
            current_world_index: 0,
            actions: Vec::new(),
            splash_lines: Vec::new(),
            server_version: String::new(),
            android_app_version: String::new(),
            your_display_id: 7,
            scrollback_push: false,
            emoji_json: String::new(),
            emoji_categories_json: String::new(),
            tf_status: Vec::new(),
        })
    }

    fn texts(&self, w: usize) -> Vec<String> {
        self.core.worlds[w].lines.iter().map(|l| l.text.clone()).collect()
    }

    fn seqs(&self, w: usize) -> Vec<u64> {
        self.core.worlds[w].lines.iter().filter_map(|l| l.seq).collect()
    }
}

fn auth_request(sent: &[WsMessage]) -> &WsMessage {
    sent.iter().find(|m| matches!(m, WsMessage::AuthRequest { .. })).expect("an AuthRequest")
}

// ---- logging in ----

#[test]
fn logs_in_with_the_password_answering_the_challenge() {
    let mut h = H::password();
    let e = h.core.start(h.now);
    assert!(e.iter().any(|e| matches!(e, Effect::Connect)));
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: CHALLENGE.into() });
    let sent = h.sent();
    let WsMessage::AuthRequest { password_hash, auth_key, challenge_response, client_uid, client_version, .. } = auth_request(&sent) else {
        unreachable!()
    };
    assert_eq!(password_hash, &hash_with_challenge(&hash_password("pw"), CHALLENGE));
    assert!(auth_key.is_none());
    assert!(challenge_response);
    assert_eq!(client_uid, "uid-1");
    assert_eq!(client_version, crate::VERSION);
    h.msg(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false });
    assert!(matches!(h.sent().as_slice(), [WsMessage::ClientTypeDeclaration { client_type: RemoteClientType::RemoteGUI }]));
    assert_eq!(h.core.status(), &Status::Online);
}

#[test]
fn a_stored_key_goes_first_and_a_refused_key_falls_back_to_the_password() {
    let mut h = H::new(Credentials { password: Some("pw".into()), auth_key: Some("k3y".into()), ..Default::default() });
    h.core.start(h.now);
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: CHALLENGE.into() });
    let sent = h.sent();
    let WsMessage::AuthRequest { password_hash, auth_key, .. } = auth_request(&sent) else { unreachable!() };
    assert!(password_hash.is_empty());
    assert_eq!(auth_key.as_deref(), Some(hash_with_challenge("k3y", CHALLENGE).as_str()), "the raw key never goes on the wire");
    h.msg(WsMessage::AuthResponse { success: false, error: Some("Invalid key".into()), username: None, multiuser_mode: false });
    let sent = h.sent();
    let WsMessage::AuthRequest { password_hash, auth_key, .. } = auth_request(&sent) else { unreachable!() };
    assert_eq!(password_hash, &hash_with_challenge(&hash_password("pw"), CHALLENGE));
    assert!(auth_key.is_none());
}

#[test]
fn without_credentials_it_asks_and_then_logs_in_on_the_same_connection() {
    let mut h = H::new(Credentials::default());
    h.core.start(h.now);
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: CHALLENGE.into() });
    assert!(h.sent().is_empty());
    assert!(matches!(h.core.status(), Status::NeedLogin { multiuser: false, error: None }));
    let e = h.core.login(Credentials { password: Some("pw".into()), ..Default::default() }, h.now);
    h.keep(e);
    assert!(matches!(auth_request(&h.sent()), WsMessage::AuthRequest { .. }));
}

#[test]
fn a_refused_password_asks_again_and_the_login_screen_is_not_reconnected_under_the_user() {
    let mut h = H::password();
    h.core.start(h.now);
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: CHALLENGE.into() });
    h.msg(WsMessage::AuthResponse { success: false, error: Some("Invalid password".into()), username: None, multiuser_mode: false });
    assert!(matches!(h.core.status(), Status::NeedLogin { error: Some(_), .. }));
    // The server closes the unauthenticated socket; nothing reconnects by itself.
    h.input(Input::Closed("timeout".into()));
    h.drain();
    h.advance(Duration::from_secs(60));
    assert!(!h.drain().iter().any(|e| matches!(e, Effect::Connect)));
    // The user logs in: a fresh connection.
    let e = h.core.login(Credentials { password: Some("pw2".into()), ..Default::default() }, h.now);
    assert!(e.iter().any(|e| matches!(e, Effect::Connect)));
}

#[test]
fn no_challenge_means_no_credentials_on_the_wire() {
    let mut h = H::password();
    h.core.start(h.now);
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: String::new() });
    let effects = h.drain();
    assert!(!effects.iter().any(|e| matches!(e, Effect::Send(m) if matches!(**m, WsMessage::AuthRequest { .. }))));
    assert!(effects.iter().any(|e| matches!(e, Effect::Disconnect)));
    assert!(matches!(h.core.status(), Status::Stopped(_)));
}

#[test]
fn the_servers_key_is_kept_for_next_time() {
    let mut h = H::password();
    h.connect_and_login();
    let mut s = settings();
    s.auth_key = "abcdef".into();
    h.msg(WsMessage::GlobalSettingsUpdated { settings: s, input_height: 1 });
    assert!(h.drain().iter().any(|e| matches!(e, Effect::StoreAuthKey(k) if k == "abcdef")));
}

// ---- output ----

#[test]
fn initial_state_lines_and_their_seqs_are_held_and_the_world_marked_seen() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    assert_eq!(h.seqs(0), vec![0, 1, 2, 3, 4]);
    assert_eq!(h.core.worlds[0].seen.ranges(), &[(0, 4)]);
    assert_eq!(h.core.display_id, 7);
    assert!(h.sent().iter().any(|m| matches!(m, WsMessage::MarkWorldSeen { world_index: 0, previous_world_index: None })));
}

#[test]
fn a_repeated_batch_is_dropped_and_reported() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.msg(data(0, 5, "five\nsix\n"));
    h.msg(data(0, 5, "five\nsix\n"));
    assert_eq!(h.seqs(0), vec![0, 1, 2, 3, 4, 5, 6]);
    assert!(h.sent().iter().any(|m| matches!(m, WsMessage::ReportDuplicate { line_seq: 5, .. })));
}

#[test]
fn every_line_is_kept_and_the_unshown_ones_are_marked_for_display() {
    // An escape-only line and an idler marker are never shown, but they are kept (so the
    // buffer matches the server's) and their seqs are recorded like any other.
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..1, 9)]);
    h.msg(data(0, 1, "a\n\x1b[0m\n###_idler_message_x_###\n\nb\n"));
    assert_eq!(h.texts(0), vec!["line 0", "a", "\x1b[0m", "###_idler_message_x_###", "", "b"]);
    let shown: Vec<String> = h.texts(0).into_iter().filter(|t| !hidden_line(t)).collect();
    assert_eq!(shown, vec!["line 0", "a", "", "b"], "blank lines show");
    assert_eq!(h.core.worlds[0].seen.ranges(), &[(0, 5)]);
}

#[test]
fn a_partial_line_waits_for_the_rest() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..1, 9)]);
    let mut m = data(0, 1, "whole\nhal");
    if let WsMessage::ServerData { end_seq, .. } = &mut m {
        *end_seq = None;
    }
    h.msg(m);
    assert_eq!(h.texts(0), vec!["line 0", "whole"]);
    h.msg(data(0, 2, "f done\n"));
    assert_eq!(h.texts(0), vec!["line 0", "whole", "half done"]);
    assert_eq!(h.core.worlds[0].seen.ranges(), &[(0, 2)]);
}

#[test]
fn a_late_batch_fills_its_gap_in_order() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..3, 9)]);
    h.msg(data(0, 5, "five\n"));
    h.msg(data(0, 3, "three\nfour\n"));
    assert_eq!(h.seqs(0), vec![0, 1, 2, 3, 4, 5]);
    assert!(h.sent().iter().any(|m| matches!(m, WsMessage::ReportOutOfOrder { line_seq: 3, recovered_count: 2, .. })));
    assert_eq!(h.core.worlds[0].seen.ranges(), &[(0, 5)]);
}

#[test]
fn a_flush_starts_the_world_over() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..3, 9)]);
    let mut m = data(0, 3, "after\n");
    if let WsMessage::ServerData { flush, .. } = &mut m {
        *flush = true;
    }
    h.msg(m);
    assert_eq!(h.texts(0), vec!["after"]);
    assert_eq!(h.core.worlds[0].seen.ranges(), &[(3, 3)]);
}

// ---- keeping in step ----

#[test]
fn keepalive_acks_each_worlds_contiguous_frontier() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9), world_msg(1, "B", 0..0, 9)]);
    h.msg(data(0, 7, "seven\n")); // a hole at 5..6
    h.sent();
    h.advance(Duration::from_secs(1));
    let sent = h.sent();
    assert!(sent.iter().any(|m| matches!(m, WsMessage::Ping)));
    let acked = sent.iter().find_map(|m| if let WsMessage::PongCheck { acked, .. } = m { Some(acked.clone()) } else { None });
    assert_eq!(acked, Some(vec![(0, 4)]), "world B holds nothing and isn't listed");
    h.msg(WsMessage::PingCheck { nonce: 42 });
    assert!(h.sent().iter().any(|m| matches!(m, WsMessage::PongCheck { nonce: 42, .. })));
}

#[test]
fn a_resync_request_asks_for_everything_after_the_given_seq() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.sent();
    h.msg(WsMessage::ResyncRequired { world_index: 0, from_seq: 3 });
    let sent = h.sent();
    let Some(WsMessage::RequestScrollback { after_seq, before_seq, request_id, .. }) =
        sent.iter().find(|m| matches!(m, WsMessage::RequestScrollback { .. }))
    else {
        panic!("a gap-fill request: {sent:?}")
    };
    assert_eq!((*after_seq, *before_seq), (Some(3), None));
    assert!(request_id.is_some_and(|id| id > 0), "request id 0 is the server's own replay");
}

#[test]
fn a_hole_the_server_cant_fill_is_given_up_reported_and_passed() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.msg(data(0, 10, "ten\n")); // hole 5..9
    h.msg(WsMessage::ResyncRequired { world_index: 0, from_seq: 4 });
    let id = |sent: &[WsMessage]| {
        sent.iter().rev().find_map(|m| if let WsMessage::RequestScrollback { request_id, .. } = m { *request_id } else { None }).unwrap()
    };
    let first = id(&h.sent());
    let empty = |id| WsMessage::ScrollbackLines { world_index: 0, lines: Vec::new(), backfill_complete: false, clamped_by_pending: false, request_id: Some(id) };
    h.msg(empty(first));
    let second = id(&h.sent());
    h.msg(empty(second));
    let sent = h.sent();
    assert!(sent.iter().any(|m| matches!(m, WsMessage::ReportGap { hole_start: 5, hole_end: 9, attempts: 2, .. })), "{sent:?}");
    assert_eq!(h.core.worlds[0].seen.frontier(), 10);
    // And it carries on from past the hole.
    assert!(sent.iter().any(|m| matches!(m, WsMessage::RequestScrollback { after_seq: Some(10), .. })));
}

#[test]
fn a_reconnect_resumes_from_the_frontier_and_keeps_the_buffer() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.msg(data(0, 5, "five\n"));
    h.input(Input::Closed("network".into()));
    assert!(matches!(h.core.status(), Status::Reconnecting { .. }));
    h.drain();
    // Not before the backoff, then a fresh connection.
    h.advance(Duration::from_millis(500));
    assert!(!h.drain().iter().any(|e| matches!(e, Effect::Connect)));
    h.advance(Duration::from_secs(2));
    assert!(h.drain().iter().any(|e| matches!(e, Effect::Connect)));
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: "c2".into() });
    let sent = h.sent();
    let WsMessage::AuthRequest { resume, resume_epochs, .. } = auth_request(&sent) else { unreachable!() };
    assert_eq!(resume, &vec![(0, 5)]);
    assert_eq!(resume_epochs, &vec![(0, 9)]);
    h.msg(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false });
    // The server skips the history the client still holds; whatever it sends is ignored.
    h.initial(vec![world_msg(0, "A", 3..6, 9)]);
    assert_eq!(h.texts(0), vec!["line 0", "line 1", "line 2", "line 3", "line 4", "five"]);
    // Resumed: it waits for the server's unprompted replay rather than asking.
    assert!(!h.sent().iter().any(|m| matches!(m, WsMessage::RequestScrollback { .. })));
    h.msg(WsMessage::ScrollbackLines { world_index: 0, lines: vec![tl(6, "six")], backfill_complete: true, clamped_by_pending: false, request_id: Some(0) });
    assert_eq!(h.seqs(0), vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn a_resumed_world_whose_replay_never_comes_asks_for_it() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.input(Input::Closed("network".into()));
    h.advance(Duration::from_secs(3));
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: "c2".into() });
    h.msg(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false });
    h.initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.sent();
    h.advance(Duration::from_secs(16));
    assert!(h.sent().iter().any(|m| matches!(m, WsMessage::RequestScrollback { after_seq: Some(4), .. })));
}

#[test]
fn a_new_epoch_throws_the_old_buffer_away() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.input(Input::Closed("server restarted".into()));
    h.advance(Duration::from_secs(3));
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: "c2".into() });
    h.msg(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false });
    h.initial(vec![world_msg(0, "A", 0..2, 10)]);
    assert_eq!(h.texts(0), vec!["line 0", "line 1"]);
    assert_eq!(h.core.worlds[0].seen.ranges(), &[(0, 1)]);
}

#[test]
fn a_state_refresh_is_not_a_reconnect() {
    // RequestState's answer carries no resume replay: a kept buffer gap-fills itself.
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    h.sent();
    h.initial(vec![world_msg(0, "A", 0..8, 9)]);
    h.advance(Duration::from_millis(10));
    assert!(h.sent().iter().any(|m| matches!(m, WsMessage::RequestScrollback { after_seq: Some(4), .. })));
}

#[test]
fn a_silent_connection_is_closed_and_reopened() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..1, 9)]);
    h.drain();
    h.advance(Duration::from_secs(5));
    for _ in 0..16 {
        h.advance(Duration::from_secs(5));
    }
    assert!(h.drain().iter().any(|e| matches!(e, Effect::Disconnect)));
}

#[test]
fn after_a_server_reload_it_reconnects_quickly() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..1, 9)]);
    h.msg(WsMessage::ServerReloading);
    h.input(Input::Closed("reload".into()));
    h.drain();
    h.advance(Duration::from_secs(2));
    assert!(h.drain().iter().any(|e| matches!(e, Effect::Connect)));
    h.input(Input::Closed("still reloading".into()));
    h.drain();
    h.advance(Duration::from_secs(1));
    assert!(h.drain().iter().any(|e| matches!(e, Effect::Connect)), "1 s after the first retry");
}

#[test]
fn backoff_grows_and_never_stops() {
    let mut h = H::password();
    h.core.start(h.now);
    let mut waits = Vec::new();
    for _ in 0..8 {
        h.input(Input::Closed("refused".into()));
        let Status::Reconnecting { at, .. } = h.core.status().clone() else { panic!() };
        waits.push(at.duration_since(h.now).as_secs());
        h.now = at;
        h.advance(Duration::ZERO);
        h.drain();
    }
    assert_eq!(waits, vec![2, 2, 5, 10, 20, 30, 30, 30]);
}

#[test]
fn a_changed_certificate_stops_everything_until_the_user_trusts_it() {
    let mut h = H::password();
    h.core.start(h.now);
    let m = CertMismatch { host: "h:9000".into(), old_fingerprint: "aa".into(), new_fingerprint: "bb".into() };
    h.input(Input::CertMismatch(m.clone()));
    assert_eq!(h.core.status(), &Status::CertMismatch(m));
    h.drain();
    h.advance(Duration::from_secs(120));
    assert!(!h.drain().iter().any(|e| matches!(e, Effect::Connect)));
    let e = h.core.trust_cert();
    assert!(matches!(e.as_slice(), [Effect::TrustCert { fingerprint, .. }, Effect::Changed, Effect::Connect] if fingerprint == "bb"), "{e:?}");
}

// ---- ▶ ownership ----

#[test]
fn a_local_claim_is_reconciled_with_the_servers() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..1, 9), world_msg(1, "B", 0..3, 9)]);
    for l in &mut h.core.worlds[1].lines {
        l.viewed = false;
    }
    h.sent();
    let e = h.core.switch_world(1, h.now);
    h.keep(e);
    let sent = h.sent();
    assert!(sent.iter().any(|m| matches!(m, WsMessage::MarkWorldSeen { world_index: 1, previous_world_index: Some(0) })));
    assert!(sent.iter().any(|m| matches!(m, WsMessage::RequestWorldState { world_index: 1 })));
    let mine = |h: &H| h.core.worlds[1].lines.iter().filter(|l| l.display_id == Some(7)).filter_map(|l| l.seq).collect::<Vec<_>>();
    assert_eq!(mine(&h), vec![0, 1, 2], "guessed before the server answers");
    h.msg(WsMessage::ClaimedNew { world_index: 1, seqs: vec![1, 2] });
    assert_eq!(mine(&h), vec![1, 2], "the server's word wins");
    h.msg(WsMessage::ReleasedNew { world_index: 1 });
    assert!(mine(&h).is_empty());
}

#[test]
fn a_hidden_window_says_so_after_connecting() {
    let mut h = H::password();
    h.core.visible = false;
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..1, 9)]);
    let sent = h.sent();
    assert!(sent.iter().any(|m| matches!(m, WsMessage::ClientVisibility { visible: false })));
    assert!(!sent.iter().any(|m| matches!(m, WsMessage::MarkWorldSeen { .. })));
}

// ---- history ----

#[test]
fn a_world_short_of_its_history_fetches_older_lines() {
    let mut h = H::password();
    let mut w = world_msg(0, "A", 100..110, 9);
    w.total_visible_lines = Some(500);
    h.connect_and_login().initial(vec![w]);
    h.sent();
    h.advance(Duration::from_millis(600));
    let sent = h.sent();
    let Some(WsMessage::RequestScrollback { before_seq, count, request_id: Some(id), .. }) =
        sent.iter().find(|m| matches!(m, WsMessage::RequestScrollback { .. }))
    else {
        panic!("a backfill request: {sent:?}")
    };
    assert_eq!(*before_seq, Some(100));
    assert_eq!(*count, 65, "the first screenful-plus: 75 minus the 10 held");
    let older: Vec<TimestampedLine> = (90..100).map(|s| tl(s, "old")).collect();
    h.msg(WsMessage::ScrollbackLines { world_index: 0, lines: older, backfill_complete: false, clamped_by_pending: false, request_id: Some(*id) });
    assert_eq!(h.seqs(0)[..2], [90, 91]);
    assert_eq!(h.core.worlds[0].seen.frontier(), 109, "older lines don't move the frontier");
}

#[test]
fn split_lines_matches_the_web_clients_split() {
    assert_eq!(split_lines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
    assert_eq!(split_lines("a\n"), vec!["a", ""]);
    assert_eq!(split_lines(""), vec![""]);
    assert!(is_ansi_only("\x1b[0m\x1b[1;31m"));
    assert!(!is_ansi_only(""));
    assert!(!is_ansi_only("\x1b[0mx"));
}

#[test]
fn saving_the_settings_back_unchanged_changes_nothing_but_what_was_edited() {
    let mut app = crate::App::new();
    app.settings.web_font_size_desktop = 15.0;
    app.settings.websocket_allow_list = "10.0.0.*".into();
    let before = app.build_global_settings_msg();
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let _ = app.handle_ws_client_msg(1, settings_update(&before), &tx);
    let after = app.build_global_settings_msg();
    // `web_secure` goes back true, as the web client sends it: it's kept only for wire
    // compatibility and never read (the server is always TLS-capable).
    let json = |s: &crate::websocket::GlobalSettingsMsg| {
        let mut v = serde_json::to_value(s).unwrap();
        v.as_object_mut().unwrap().remove("web_secure");
        v
    };
    assert_eq!(json(&before), json(&after));
    // And the one field the slider moves does move.
    let mut edited = after.clone();
    edited.web_font_size_desktop = 17.0;
    let _ = app.handle_ws_client_msg(1, settings_update(&edited), &tx);
    assert_eq!(app.settings.web_font_size_desktop, 17.0);
    assert_eq!(app.settings.websocket_allow_list, "10.0.0.*");
}

#[test]
fn the_scrollback_badge_counts_down_to_done() {
    let mut t = H::password();
    t.connect_and_login();
    let mut w = world_msg(0, "w", 900..1000, 7);
    w.output_lines_ts.drain(..80);
    w.total_visible_lines = Some(1000);
    t.initial(vec![w]);
    // 20 of the 100 (remote_initial_lines' default) held: 20%.
    assert_eq!(t.core.backfill_progress(), Some(20));
}

#[test]
fn a_multiuser_logout_returns_to_the_login_screen_on_the_same_connection() {
    let mut t = H::new(Credentials { username: Some("ann".into()), password: Some("pw".into()), auth_key: None });
    let e = t.core.start(t.now);
    t.keep(e);
    t.input(Input::Opened);
    t.msg(WsMessage::ServerHello { multiuser_mode: true, challenge: CHALLENGE.into() });
    t.msg(WsMessage::AuthResponse { success: true, error: None, username: Some("ann".into()), multiuser_mode: true });
    t.initial(vec![world_msg(0, "w", 0..3, 7)]);
    t.sent();
    let e = t.core.logout();
    t.keep(e);
    assert!(t.sent().iter().any(|m| matches!(m, WsMessage::Logout)));
    t.core.splash_lines = vec!["Welcome".into()];
    t.msg(WsMessage::LoggedOut);
    assert!(t.core.worlds.is_empty() && t.core.splash_lines.is_empty(), "as app.js clears them");
    assert!(matches!(t.core.status(), Status::NeedLogin { multiuser: true, .. }));
    let e = t.core.login(Credentials { username: Some("bob".into()), password: Some("pw2".into()), auth_key: None }, t.now);
    t.keep(e);
    let sent = t.sent();
    assert!(matches!(auth_request(&sent), WsMessage::AuthRequest { username: Some(u), .. } if u == "bob"));
}

#[test]
fn restarting_reconnects_at_once_and_fetches_everything_again() {
    let mut t = H::password();
    t.connect_and_login();
    t.initial(vec![world_msg(0, "w", 0..50, 7)]);
    t.drain();
    let e = t.core.restart();
    t.keep(e);
    assert!(t.drain().iter().any(|e| matches!(e, Effect::Disconnect)));
    assert!(t.core.worlds.is_empty());
    t.input(Input::Closed("closed by the client".into()));
    assert!(t.drain().iter().any(|e| matches!(e, Effect::Connect)), "no backoff");
    t.input(Input::Opened);
    t.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: CHALLENGE.into() });
    let sent = t.sent();
    assert!(matches!(auth_request(&sent), WsMessage::AuthRequest { resume, .. } if resume.is_empty()), "nothing to resume");
}

fn cached(name: &str, epoch: u64, seqs: std::ops::Range<u64>) -> crate::native_gui::core::cache::CachedWorld {
    use crate::native_gui::core::cache::{CachedLine, CachedWorld};
    let lines: Vec<CachedLine> = seqs
        .clone()
        .map(|s| CachedLine { text: format!("cached {s}"), ts: 1_000, seq: Some(s), from_server: true, gagged: false, highlight: None })
        .collect();
    CachedWorld { name: name.into(), epoch, lines, seen: vec![(seqs.start, seqs.end - 1)] }
}

#[test]
fn a_cold_start_shows_the_cached_lines_and_gap_fills_after_them() {
    let mut h = H::password();
    h.core.set_cache(vec![cached("A", 9, 0..5), cached("B", 3, 0..2)]);
    h.connect_and_login();
    let sent = h.sent();
    let WsMessage::AuthRequest { resume, .. } = auth_request(&sent) else { unreachable!() };
    assert!(resume.is_empty(), "a cache is never a resume list: its indices may be stale");
    // A's epoch still stands; B's sequence space is gone (a new epoch).
    h.initial(vec![world_msg(0, "A", 0..8, 9), world_msg(1, "B", 0..4, 4)]);
    assert_eq!(h.texts(0), vec!["cached 0", "cached 1", "cached 2", "cached 3", "cached 4"]);
    assert_eq!(h.texts(1), vec!["line 0", "line 1", "line 2", "line 3"]);
    h.advance(Duration::from_secs(1));
    let asks: Vec<(usize, Option<u64>)> = h
        .sent()
        .into_iter()
        .filter_map(|m| if let WsMessage::RequestScrollback { world_index, after_seq, .. } = m { Some((world_index, after_seq)) } else { None })
        .collect();
    assert!(asks.contains(&(0, Some(4))), "A asks for what came after its cached frontier: {asks:?}");
    // And what gets cached back is capped, epoch-stamped, with the seq ranges.
    let snap = h.core.cache_snapshot();
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[1].epoch, 4);
    assert_eq!(snap[1].seen, vec![(0, 3)]);
}

#[test]
fn the_world_last_shown_wins_over_the_servers_current_one() {
    let mut h = H::password();
    h.core.prefer_world(Some("B".into()));
    h.connect_and_login();
    h.initial(vec![world_msg(0, "A", 0..2, 1), world_msg(1, "B", 0..2, 2)]);
    assert_eq!(h.core.current, 1);
    // A world that's gone is no preference.
    let mut h = H::password();
    h.core.prefer_world(Some("Gone".into()));
    h.connect_and_login();
    h.initial(vec![world_msg(0, "A", 0..2, 1), world_msg(1, "B", 0..2, 2)]);
    assert_eq!(h.core.current, 0);
}

#[test]
fn a_live_theme_edit_reaches_the_settings() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..2, 1)]);
    h.msg(WsMessage::ThemeCssVarsUpdated { css_vars: String::new(), colors_json: r##"{"bg":"#000000"}"##.into() });
    assert_eq!(h.core.settings.as_ref().unwrap().theme_colors_json, r##"{"bg":"#000000"}"##);
}

#[test]
fn the_server_can_fill_the_input_and_change_the_keys() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..2, 1)]);
    h.drain();
    h.msg(WsMessage::SetInputBuffer { text: "say hi".into(), cursor_start: false });
    assert!(h.drain().iter().any(|e| matches!(e, Effect::Event(m) if matches!(**m, WsMessage::SetInputBuffer { .. }))));
    h.msg(WsMessage::KeybindingsUpdated { bindings_json: r#"{"F3":"redraw"}"#.into() });
    assert_eq!(h.core.settings.as_ref().unwrap().keybindings_json, r#"{"F3":"redraw"}"#);
}

#[test]
fn reconnect_now_skips_the_countdown_and_only_when_closed() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..2, 1)]);
    assert_eq!(h.core.initial_states, 1);
    // Online: nothing to do.
    let e = h.core.reconnect_now();
    assert!(!e.iter().any(|e| matches!(e, Effect::Connect)));
    h.input(Input::Closed("network".into()));
    h.drain();
    // Counting down: connects at once, not after the backoff.
    let e = h.core.reconnect_now();
    assert!(e.iter().any(|e| matches!(e, Effect::Connect)));
    assert!(matches!(h.core.status(), Status::Connecting { .. }));
    // Already connecting: not again.
    let e = h.core.reconnect_now();
    assert!(!e.iter().any(|e| matches!(e, Effect::Connect)));
    h.input(Input::Opened);
    h.msg(WsMessage::ServerHello { multiuser_mode: false, challenge: "c2".into() });
    h.msg(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false });
    h.initial(vec![world_msg(0, "A", 0..2, 1)]);
    assert_eq!(h.core.initial_states, 2, "a held command waits for this");
}

#[test]
fn a_follower_keeps_up_with_its_leader_and_shows_a_world_of_its_own() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..3, 9), world_msg(1, "B", 0..2, 9)]);
    h.drain();
    let mut f = h.core.follower();
    let _ = f.switch_world(1, h.now);
    assert_eq!((h.core.current, f.current), (0, 1), "each has its own world on screen");
    let m = data(0, 3, "three\n");
    h.msg(m.clone());
    let _ = f.handle(Input::Message(Box::new(m)), h.now);
    let texts = |c: &ClientCore| c.worlds[0].lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>();
    assert_eq!(texts(&h.core), texts(&f));
}

#[test]
fn a_gap_fill_lands_in_a_follower_as_it_does_in_its_leader() {
    let mut h = H::password();
    h.connect_and_login().initial(vec![world_msg(0, "A", 0..5, 9)]);
    let mut f = h.core.follower();
    let ten = data(0, 10, "ten\n"); // a hole, 5..9
    h.msg(ten.clone());
    let _ = f.handle(Input::Message(Box::new(ten)), h.now);
    h.sent();
    h.msg(WsMessage::ResyncRequired { world_index: 0, from_seq: 4 });
    let id = h.sent().iter().find_map(|m| if let WsMessage::RequestScrollback { request_id, .. } = m { *request_id } else { None }).expect("leader asks");
    // The follower's own requests (never sent) are numbered apart from the leader's.
    let mine = f.handle(Input::Message(Box::new(WsMessage::ResyncRequired { world_index: 0, from_seq: 4 })), h.now);
    let my_id = mine
        .iter()
        .find_map(|e| match e {
            Effect::Send(m) => match &**m {
                WsMessage::RequestScrollback { request_id, .. } => *request_id,
                _ => None,
            },
            _ => None,
        })
        .expect("follower asks too");
    assert_ne!(id, my_id);
    // The answer to the leader's request, taken on first, lands the same in both.
    let fill = WsMessage::ScrollbackLines { world_index: 0, lines: (5..10).map(|s| tl(s, "fill")).collect(), backfill_complete: false, clamped_by_pending: false, request_id: Some(id) };
    f.adopt_request(id, h.core.pending_request(id).expect("still waiting"));
    h.msg(fill.clone());
    let _ = f.handle(Input::Message(Box::new(fill)), h.now);
    assert_eq!(h.seqs(0), f.worlds[0].lines.iter().filter_map(|l| l.seq).collect::<Vec<_>>());
    assert_eq!(h.core.worlds[0].seen.frontier(), 10);
    assert_eq!(f.worlds[0].seen.frontier(), 10);
}
