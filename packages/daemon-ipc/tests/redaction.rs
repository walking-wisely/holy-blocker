use daemon_ipc::msg::{AppEntry, Cooldown, ToDaemon, ToPanel};
use daemon_ipc::{AppIdentity, ClientRole, End, Limits, Session, frame};

fn bytes_form(text: &str) -> String {
    format!("{:?}", text.as_bytes())
}

const SECRET: &str = "com.secret.hidden-app";

fn id() -> AppIdentity {
    AppIdentity::new(SECRET).unwrap()
}

#[test]
fn debug_output_of_identity_carrying_messages_hides_the_identity() {
    let dumps = [
        format!("{:?}", id()),
        format!("{:?}", ToDaemon::RequestAddApp { identity: id() }),
        format!("{:?}", ToDaemon::ConfirmRemoveApp { identity: id() }),
        format!(
            "{:?}",
            ToPanel::Apps {
                apps: vec![AppEntry {
                    identity: id(),
                    removal: Cooldown::Idle
                }]
            }
        ),
    ];
    for dump in dumps {
        assert!(!dump.contains("secret"), "{dump}");
    }
}

#[test]
fn events_and_close_reasons_hold_no_text_from_the_body() {
    let mut session = Session::new(End::Daemon(ClientRole::Panel), Limits::new(4096), 0);
    let limits = Limits::new(4096);
    let mut input = frame::encode(br#"{"type":"hello","proto":1}"#, limits).unwrap();
    let body = format!(r#"{{"type":"request_add_app","identity":"{SECRET}!"}}"#);
    input.extend(frame::encode(body.as_bytes(), limits).unwrap());
    let events = session.receive(&input, 0);
    let dump = format!("{events:?}");
    assert!(!dump.contains("secret"), "{dump}");
}

#[test]
fn identity_validation_rejects_what_a_bundle_id_cannot_be() {
    for bad in [
        "",
        ".",
        "..",
        "-rf",
        "_x",
        "a..b",
        "has space",
        "slash/inside",
        "uni\u{e9}code",
        &"a".repeat(256),
    ] {
        assert!(AppIdentity::new(bad).is_err());
    }
    for good in [
        "com.apple.Safari",
        "org.mozilla.firefox",
        "a",
        "my_app-1.beta",
    ] {
        assert!(AppIdentity::new(good).is_ok());
    }
}

#[test]
fn a_decoded_frame_never_prints_its_body() {
    let limits = Limits::new(4096);
    let body = format!(r#"{{"type":"request_add_app","identity":"{SECRET}"}}"#);
    let bytes = frame::encode(body.as_bytes(), limits).unwrap();
    let mut input = &bytes[..];
    let next = frame::FrameDecoder::new(limits).next(&mut input);
    let dump = format!("{next:?}");
    assert_eq!(dump, format!("Frame(<{} bytes>)", body.len()));
    assert!(!dump.contains("secret") && !dump.contains(&bytes_form(SECRET)[1..8]));
}

#[test]
fn serde_errors_for_a_bad_identity_carry_none_of_it() {
    let err = serde_json::from_str::<AppIdentity>("\"bad id with secret\"").unwrap_err();
    let text = err.to_string();
    assert!(
        !text.contains("secret") && !text.contains("bad id"),
        "{text}"
    );
}
