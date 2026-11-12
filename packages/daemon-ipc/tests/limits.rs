use daemon_ipc::msg::Wire;
use daemon_ipc::{
    AppIdentity, ClientRole, CloseReason, End, Event, Inbound, Limits, Session, ToDaemon, frame,
};

fn framed(json: &str) -> Vec<u8> {
    frame::encode(json.as_bytes(), Limits::new(u32::MAX)).unwrap()
}

fn hello() -> Vec<u8> {
    framed(r#"{"type":"hello","proto":1}"#)
}

fn open(end: End, now: u64) -> Session {
    let mut session = Session::new(end, Limits::new(4096), now);
    let events = session.receive(&hello(), now);
    assert_eq!(events, vec![Event::Hello { proto: 1 }]);
    session
}

fn panel(now: u64) -> Session {
    open(End::Daemon(ClientRole::Panel), now)
}

fn agent(now: u64) -> Session {
    open(End::Daemon(ClientRole::Agent), now)
}

fn app(name: &str) -> AppIdentity {
    AppIdentity::new(name).unwrap()
}

fn delivered(events: &[Event]) -> bool {
    matches!(events, [Event::Message(Inbound::ToDaemon(_))])
}

#[test]
fn default_caps_are_four_kib_toward_the_daemon_and_sixty_four_toward_the_panel() {
    assert_eq!(Limits::to_daemon().max_frame, 4096);
    assert_eq!(Limits::to_panel().max_frame, 65536);
}

#[test]
fn a_frame_over_the_cap_closes_and_one_at_the_cap_does_not() {
    let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::to_daemon(), 0);
    session.receive(&hello(), 0);
    let mut at_cap = br#"{"type":"list_apps","pad":""#.to_vec();
    at_cap.resize(4094, b'a');
    at_cap.extend_from_slice(b"\"}");
    assert_eq!(at_cap.len(), 4096);
    let events = session.receive(&framed(std::str::from_utf8(&at_cap).unwrap()), 1);
    assert!(delivered(&events));
    let over = vec![b'a'; 4097];
    let events = session.receive(&frame::encode(&over, Limits::new(u32::MAX)).unwrap(), 2);
    assert_eq!(events, vec![Event::Close(CloseReason::Oversized)]);
}

#[test]
fn silence_before_hello_closes_at_the_deadline() {
    let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(4096), 1000);
    assert_eq!(session.tick(5999), None);
    assert_eq!(session.tick(6000), Some(CloseReason::PreHelloTimeout));
    assert!(session.is_closed());
    assert_eq!(session.tick(7000), None);
}

#[test]
fn a_hello_in_time_clears_the_pre_hello_deadline() {
    let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(4096), 0);
    session.receive(&hello(), 4000);
    assert_eq!(session.tick(9000), None);
}

#[test]
fn a_half_sent_hello_still_hits_the_pre_hello_deadline() {
    let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(4096), 0);
    session.receive(&hello()[..3], 1000);
    assert_eq!(session.tick(5000), Some(CloseReason::PreHelloTimeout));
}

#[test]
fn a_partial_frame_closes_five_seconds_after_its_first_byte() {
    let mut session = panel(0);
    let message = framed(r#"{"type":"list_apps"}"#);
    session.receive(&message[..2], 10_000);
    assert_eq!(session.tick(14_999), None);
    assert_eq!(session.tick(15_000), Some(CloseReason::FrameTimeout));
}

#[test]
fn a_trickle_does_not_extend_the_frame_deadline() {
    let mut session = panel(0);
    let message = framed(r#"{"type":"list_apps"}"#);
    for (i, byte) in message[..message.len() - 1].iter().enumerate() {
        let events = session.receive(std::slice::from_ref(byte), 10_000 + i as u64 * 1000);
        if session.is_closed() {
            assert_eq!(events, vec![Event::Close(CloseReason::FrameTimeout)]);
            return;
        }
    }
    panic!("the trickle was never closed");
}

#[test]
fn bytes_arriving_after_the_deadline_do_not_revive_the_frame() {
    let mut session = panel(0);
    let message = framed(r#"{"type":"list_apps"}"#);
    session.receive(&message[..2], 1000);
    let events = session.receive(&message[2..], 7000);
    assert_eq!(events, vec![Event::Close(CloseReason::FrameTimeout)]);
}

#[test]
fn a_completed_frame_restarts_the_frame_clock_for_the_next_one() {
    let mut session = panel(0);
    let message = framed(r#"{"type":"list_apps"}"#);
    let mut two = message.clone();
    two.extend_from_slice(&message[..3]);
    session.receive(&two, 1000);
    assert_eq!(session.tick(5999), None);
    assert_eq!(session.tick(6000), Some(CloseReason::FrameTimeout));
}

#[test]
fn sixty_idle_seconds_close_and_activity_resets_the_clock() {
    let mut session = panel(0);
    assert_eq!(session.tick(59_999), None);
    session.receive(&framed(r#"{"type":"list_apps"}"#), 30_000);
    assert_eq!(session.tick(89_999), None);
    assert_eq!(session.tick(90_000), Some(CloseReason::Idle));
}

#[test]
fn the_eleventh_message_in_a_second_is_dropped_and_counted() {
    let mut session = panel(0);
    let message = framed(r#"{"type":"list_apps"}"#);
    for _ in 0..10 {
        assert!(delivered(&session.receive(&message, 0)));
    }
    assert_eq!(session.receive(&message, 0), vec![Event::Throttled]);
    assert!(delivered(&session.receive(&message, 100)));
    assert_eq!(session.drain_counters(100).throttled, 1);
}

#[test]
fn only_one_state_changing_message_passes_per_second() {
    let mut session = panel(0);
    let first = framed(r#"{"type":"request_add_app","identity":"com.example.a"}"#);
    let second = framed(r#"{"type":"request_add_app","identity":"com.example.b"}"#);
    assert!(delivered(&session.receive(&first, 0)));
    assert_eq!(session.receive(&second, 10), vec![Event::Throttled]);
    assert!(delivered(&session.receive(&second, 1000)));
}

#[test]
fn a_dropped_state_change_does_not_spend_a_general_token() {
    let mut session = panel(0);
    let add = framed(r#"{"type":"request_add_app","identity":"com.example.a"}"#);
    let list = framed(r#"{"type":"list_apps"}"#);
    session.receive(&add, 0);
    for _ in 0..5 {
        assert_eq!(session.receive(&add, 0), vec![Event::Throttled]);
    }
    for _ in 0..9 {
        assert!(delivered(&session.receive(&list, 0)));
    }
}

#[test]
fn protective_messages_are_never_dropped() {
    let mut session = panel(0);
    let list = framed(r#"{"type":"list_apps"}"#);
    for _ in 0..10 {
        session.receive(&list, 0);
    }
    assert_eq!(session.receive(&list, 0), vec![Event::Throttled]);
    for _ in 0..50 {
        for body in [
            r#"{"type":"cancel_disarm"}"#,
            r#"{"type":"raise_protection"}"#,
        ] {
            assert!(delivered(&session.receive(&framed(body), 0)));
        }
    }
    assert_eq!(session.drain_counters(0).throttled, 1);
}

#[test]
fn a_repeated_disarm_request_is_a_duplicate_until_it_is_settled() {
    let mut session = panel(0);
    let request = framed(r#"{"type":"request_disarm"}"#);
    assert!(delivered(&session.receive(&request, 0)));
    assert_eq!(session.receive(&request, 1000), vec![Event::Duplicate]);
    session.settle_disarm();
    assert!(delivered(&session.receive(&request, 2000)));
    assert_eq!(session.drain_counters(2000).duplicates, 1);
}

#[test]
fn confirming_or_cancelling_clears_the_pending_disarm() {
    for ending in [
        r#"{"type":"confirm_disarm"}"#,
        r#"{"type":"cancel_disarm"}"#,
    ] {
        let mut session = panel(0);
        let request = framed(r#"{"type":"request_disarm"}"#);
        session.receive(&request, 0);
        assert!(delivered(&session.receive(&framed(ending), 1000)));
        assert!(delivered(&session.receive(&request, 2000)));
    }
}

#[test]
fn removal_requests_are_pending_per_identity() {
    let mut session = panel(0);
    let a = framed(r#"{"type":"request_remove_app","identity":"com.example.a"}"#);
    let b = framed(r#"{"type":"request_remove_app","identity":"com.example.b"}"#);
    assert!(delivered(&session.receive(&a, 0)));
    assert!(delivered(&session.receive(&b, 1000)));
    assert_eq!(session.receive(&a, 2000), vec![Event::Duplicate]);
    session.settle_removal(&app("com.example.a"));
    assert!(delivered(&session.receive(&a, 3000)));
    let cancel = framed(r#"{"type":"cancel_remove_app","identity":"com.example.b"}"#);
    assert!(delivered(&session.receive(&cancel, 4000)));
    assert!(delivered(&session.receive(&b, 5000)));
}

#[test]
fn pending_removals_are_bounded() {
    let mut session = panel(0);
    let mut now = 0;
    let mut dropped = 0;
    for i in 0..40 {
        let body = format!(r#"{{"type":"request_remove_app","identity":"com.example.a{i}"}}"#);
        let events = session.receive(&framed(&body), now);
        if events == vec![Event::Throttled] {
            dropped += 1;
        }
        now += 1000;
    }
    assert!(dropped > 0);
}

#[test]
fn unknown_types_close_once_the_budget_is_spent() {
    let mut session = panel(0);
    let unknown = framed(r#"{"type":"from_the_future"}"#);
    for i in 0..8 {
        assert_eq!(session.receive(&unknown, i * 200), vec![Event::Unsupported]);
    }
    assert_eq!(
        session.receive(&unknown, 2000),
        vec![Event::Close(CloseReason::UnsupportedFlood)]
    );
}

#[test]
fn a_supported_message_resets_the_unknown_budget() {
    let mut session = panel(0);
    let unknown = framed(r#"{"type":"from_the_future"}"#);
    let list = framed(r#"{"type":"list_apps"}"#);
    for round in 0..5u64 {
        for i in 0..8 {
            assert_eq!(
                session.receive(&unknown, round * 1000 + i),
                vec![Event::Unsupported]
            );
        }
        assert!(delivered(&session.receive(&list, round * 1000 + 100)));
    }
}

#[test]
fn an_agent_that_stops_sending_heartbeats_is_lost_after_the_window() {
    let mut session = agent(0);
    session.receive(&framed(r#"{"type":"heartbeat"}"#), 10_000);
    assert_eq!(session.tick(24_999), None);
    assert_eq!(session.tick(25_000), Some(CloseReason::HeartbeatLost));
    assert!(session.is_closed());
}

#[test]
fn the_heartbeat_window_starts_at_the_hello() {
    let mut session = agent(2000);
    assert_eq!(session.tick(16_999), None);
    assert_eq!(session.tick(17_000), Some(CloseReason::HeartbeatLost));
}

#[test]
fn a_panel_session_needs_no_heartbeat() {
    let mut session = panel(0);
    session.receive(&framed(r#"{"type":"list_apps"}"#), 50_000);
    assert_eq!(session.tick(59_999), None);
}

#[test]
fn a_throttled_heartbeat_still_proves_the_agent_is_alive() {
    let mut session = agent(0);
    let scan = framed(r#"{"type":"scan_event","verdict":"blocked","at_ms":1}"#);
    for _ in 0..10 {
        session.receive(&scan, 10_000);
    }
    let events = session.receive(&framed(r#"{"type":"heartbeat"}"#), 10_000);
    assert_eq!(events, vec![Event::Throttled]);
    assert_eq!(session.tick(24_999), None);
    assert_eq!(session.tick(25_000), Some(CloseReason::HeartbeatLost));
}

#[test]
fn the_wrong_role_still_closes_before_the_rate_limit_is_consulted() {
    let mut session = agent(0);
    let events = session.receive(&framed(r#"{"type":"request_disarm"}"#), 0);
    assert_eq!(events, vec![Event::Close(CloseReason::WrongRole)]);
}

#[test]
fn counters_hold_a_count_and_a_window_and_reset_on_drain() {
    let mut session = panel(0);
    let list = framed(r#"{"type":"list_apps"}"#);
    for _ in 0..13 {
        session.receive(&list, 0);
    }
    let counters = session.drain_counters(4000);
    assert_eq!(counters.throttled, 3);
    assert_eq!(counters.window_ms, 4000);
    let again = session.drain_counters(5000);
    assert_eq!(again.throttled, 0);
    assert_eq!(again.window_ms, 1000);
}

#[test]
fn nothing_the_limits_emit_carries_text_from_a_frame_body() {
    let secret = "zz-secret-marker";
    let mut session = panel(0);
    let mut printed = String::new();
    let bodies = [
        format!(r#"{{"type":"request_add_app","identity":"{secret}"}}"#),
        format!(r#"{{"type":"request_add_app","identity":"{secret}"}}"#),
        format!(r#"{{"type":"request_remove_app","identity":"{secret}"}}"#),
        format!(r#"{{"type":"request_remove_app","identity":"{secret}"}}"#),
        format!(r#"{{"type":"{secret}"}}"#),
    ];
    for (i, body) in bodies.iter().enumerate() {
        let events = session.receive(&framed(body), 1000 * i as u64);
        printed.push_str(&format!("{events:?}"));
    }
    printed.push_str(&format!("{:?}", session.drain_counters(9000)));
    printed.push_str(&format!("{:?}", session.tick(100_000)));
    assert!(!printed.contains(secret), "{printed}");
}

#[test]
fn a_clock_that_runs_backwards_never_panics_or_closes() {
    let mut session = panel(10_000);
    assert_eq!(session.tick(5000), None);
    let events = session.receive(&framed(r#"{"type":"list_apps"}"#), 1000);
    assert!(delivered(&events));
}

#[test]
fn the_message_kinds_the_limiter_treats_specially_are_the_documented_ones() {
    let exempt: Vec<_> = ["cancel_disarm", "raise_protection"].to_vec();
    for tag in <ToDaemon as Wire>::TAGS {
        let protective = exempt.contains(tag);
        let state_changing = matches!(
            *tag,
            "request_disarm"
                | "confirm_disarm"
                | "request_add_app"
                | "request_remove_app"
                | "confirm_remove_app"
                | "cancel_remove_app"
        );
        let body = match *tag {
            "request_add_app" | "request_remove_app" | "confirm_remove_app"
            | "cancel_remove_app" => {
                format!(r#"{{"type":"{tag}","identity":"com.example.a"}}"#)
            }
            "scan_event" => r#"{"type":"scan_event","verdict":"blocked","at_ms":1}"#.to_owned(),
            other => format!(r#"{{"type":"{other}"}}"#),
        };
        let parsed: ToDaemon = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.is_exempt_from_limits(), protective, "{tag}");
        assert_eq!(parsed.is_state_changing(), state_changing, "{tag}");
    }
}

#[test]
fn a_protective_message_in_late_bytes_is_delivered_before_the_close() {
    for body in [
        r#"{"type":"cancel_disarm"}"#,
        r#"{"type":"raise_protection"}"#,
    ] {
        let mut session = panel(0);
        let events = session.receive(&framed(body), 61_000);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(&events[0], Event::Message(Inbound::ToDaemon(_))));
        assert_eq!(events[1], Event::Close(CloseReason::Idle));
    }
}

#[test]
fn an_ordinary_message_in_late_bytes_is_not_delivered() {
    let mut session = panel(0);
    let events = session.receive(&framed(r#"{"type":"list_apps"}"#), 61_000);
    assert_eq!(events, vec![Event::Close(CloseReason::Idle)]);
}
