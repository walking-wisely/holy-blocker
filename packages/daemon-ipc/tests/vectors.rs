use daemon_ipc::{ClientRole, CloseReason, End, Event, Inbound, Limits, Session, frame, msg::Wire};
use serde_json::Value;

fn load(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn end_of(name: &str) -> End {
    match name {
        "daemon_panel" => End::Daemon(ClientRole::Panel),
        "daemon_agent" => End::Daemon(ClientRole::Agent),
        "panel" => End::Panel,
        "agent" => End::Agent,
        other => panic!("unknown end {other}"),
    }
}

fn describe(event: &Event) -> String {
    match event {
        Event::Hello { proto } => format!("hello:{proto}"),
        Event::Message(Inbound::ToDaemon(m)) => format!("msg:{}", m.tag()),
        Event::Message(Inbound::ToPanel(m)) => format!("msg:{}", m.tag()),
        Event::Unsupported => "unsupported".to_owned(),
        Event::Throttled => "throttled".to_owned(),
        Event::Duplicate => "duplicate".to_owned(),
        Event::Close(reason) => match reason {
            CloseReason::PreHello => "close:pre_hello".to_owned(),
            CloseReason::ProtoMismatch(n) => format!("close:proto_mismatch:{n}"),
            CloseReason::DuplicateHello => "close:duplicate_hello".to_owned(),
            CloseReason::Malformed => "close:malformed".to_owned(),
            CloseReason::Oversized => "close:oversized".to_owned(),
            CloseReason::WrongRole => "close:wrong_role".to_owned(),
            CloseReason::PreHelloTimeout => "close:pre_hello_timeout".to_owned(),
            CloseReason::FrameTimeout => "close:frame_timeout".to_owned(),
            CloseReason::Idle => "close:idle".to_owned(),
            CloseReason::HeartbeatLost => "close:heartbeat_lost".to_owned(),
            CloseReason::UnsupportedFlood => "close:unsupported_flood".to_owned(),
            CloseReason::Flood => "close:flood".to_owned(),
        },
    }
}

fn stream_bytes(segments: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for seg in segments {
        if let Some(body) = seg.get("frame") {
            let body = body.as_str().unwrap().as_bytes();
            out.extend(frame::encode(body, Limits::new(u32::MAX)).unwrap());
        } else {
            out.extend(unhex(seg["hex"].as_str().unwrap()));
        }
    }
    out
}

fn replay(end: End, max_frame: u32, bytes: &[u8], chunk: usize) -> Vec<String> {
    let limits = Limits {
        messages_per_sec: 1000,
        state_changes_per_sec: 1000,
        ..Limits::new(max_frame)
    };
    let mut session = Session::new(end, limits, 0);
    let mut events = Vec::new();
    for piece in bytes.chunks(chunk) {
        events.extend(session.receive(piece, 0).iter().map(describe));
    }
    events
}

#[test]
fn handshake_vectors_hold_whole_and_chunked() {
    let doc = load(include_str!("vectors/daemon-ipc-handshake.json"));
    assert_eq!(doc["proto"], daemon_ipc::PROTO);
    assert_eq!(doc["min_proto"], daemon_ipc::MIN_PROTO);
    for case in doc["cases"].as_array().unwrap() {
        let end = end_of(case["end"].as_str().unwrap());
        let max = case["max_frame"].as_u64().unwrap() as u32;
        let bytes = stream_bytes(case["stream"].as_array().unwrap());
        let expect: Vec<String> = case["expect"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        for chunk in [bytes.len().max(1), 1, 3] {
            assert_eq!(
                replay(end, max, &bytes, chunk),
                expect,
                "{} at chunk {chunk}",
                case["name"]
            );
        }
    }
}

#[test]
fn framing_vectors_hold() {
    let doc = load(include_str!("vectors/daemon-ipc-framing.json"));
    let limits = Limits::new(doc["max_body"].as_u64().unwrap() as u32);
    for case in doc["valid"].as_array().unwrap() {
        let bytes = unhex(case["frame_hex"].as_str().unwrap());
        let body = case["body_utf8"].as_str().unwrap().as_bytes();
        let mut input = &bytes[..];
        let got = frame::FrameDecoder::new(limits).next(&mut input);
        assert_eq!(got, frame::Next::Frame(body.to_vec()), "{}", case["name"]);
        assert!(input.is_empty());
        assert_eq!(
            frame::encode(body, limits).unwrap(),
            bytes,
            "{}",
            case["name"]
        );
    }
    for case in doc["needs_more"].as_array().unwrap() {
        let bytes = unhex(case["bytes_hex"].as_str().unwrap());
        let mut input = &bytes[..];
        assert_eq!(
            frame::FrameDecoder::new(limits).next(&mut input),
            frame::Next::NeedMore,
            "{}",
            case["name"]
        );
    }
    for case in doc["oversized"].as_array().unwrap() {
        let bytes = unhex(case["bytes_hex"].as_str().unwrap());
        let mut input = &bytes[..];
        assert_eq!(
            frame::FrameDecoder::new(limits).next(&mut input),
            frame::Next::Oversized(case["declared"].as_u64().unwrap() as u32)
        );
    }
    let at_cap = unhex(
        doc["at_cap_is_not_oversized"]["bytes_hex"]
            .as_str()
            .unwrap(),
    );
    let mut input = &at_cap[..];
    assert_eq!(
        frame::FrameDecoder::new(limits).next(&mut input),
        frame::Next::NeedMore
    );
}

#[test]
fn encoding_over_the_cap_is_refused() {
    assert_eq!(
        frame::encode(&[b' '; 5], Limits::new(4)),
        Err(frame::TooLarge)
    );
    assert!(frame::encode(&[b' '; 4], Limits::new(4)).is_ok());
}
