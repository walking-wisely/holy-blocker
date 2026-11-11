use daemon_ipc::msg::{
    AgentState, AppEntry, Cooldown, DayCount, Protection, Refusal, Verdict, Wire,
};
use daemon_ipc::{AppIdentity, ToDaemon, ToPanel};

fn id() -> AppIdentity {
    AppIdentity::new("com.example.app").unwrap()
}

fn every_to_daemon() -> Vec<ToDaemon> {
    vec![
        ToDaemon::RequestDisarm,
        ToDaemon::ConfirmDisarm,
        ToDaemon::CancelDisarm,
        ToDaemon::RaiseProtection,
        ToDaemon::ListApps,
        ToDaemon::RequestAddApp { identity: id() },
        ToDaemon::RequestRemoveApp { identity: id() },
        ToDaemon::ConfirmRemoveApp { identity: id() },
        ToDaemon::CancelRemoveApp { identity: id() },
        ToDaemon::Heartbeat,
        ToDaemon::ScanEvent {
            verdict: Verdict::Blocked,
            at_ms: 1,
        },
    ]
}

fn every_to_panel() -> Vec<ToPanel> {
    vec![
        ToPanel::Status {
            protection: Protection::Full,
            agent: AgentState::Running,
            disarm: Cooldown::Cooling { remaining_ms: 5 },
        },
        ToPanel::Counts {
            days: vec![DayCount {
                day: 1,
                text: 2,
                domain: 3,
            }],
        },
        ToPanel::Apps {
            apps: vec![AppEntry {
                identity: id(),
                removal: Cooldown::Ready { expires_in_ms: 9 },
            }],
        },
        ToPanel::Scan {
            verdict: Verdict::Allowed,
            at_ms: 2,
        },
        ToPanel::Refused {
            reason: Refusal::Protected,
        },
    ]
}

fn wire_type<M: serde::Serialize>(m: &M) -> String {
    serde_json::to_value(m).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn every_to_daemon_variant_is_in_tags_and_round_trips() {
    let all = every_to_daemon();
    assert_eq!(all.len(), ToDaemon::TAGS.len());
    for m in all {
        assert_eq!(wire_type(&m), m.tag());
        assert!(ToDaemon::TAGS.contains(&m.tag()));
        let back: ToDaemon = serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}

#[test]
fn every_to_panel_variant_is_in_tags_and_round_trips() {
    let all = every_to_panel();
    assert_eq!(all.len(), ToPanel::TAGS.len());
    for m in all {
        assert_eq!(wire_type(&m), m.tag());
        assert!(ToPanel::TAGS.contains(&m.tag()));
        let back: ToPanel = serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}

#[test]
fn encode_message_reports_the_cap_not_a_serde_failure() {
    use daemon_ipc::{EncodeError, Limits, encode_message};
    assert_eq!(
        encode_message(&ToDaemon::ListApps, Limits::new(4)),
        Err(EncodeError::TooLarge)
    );
    assert!(encode_message(&ToDaemon::ListApps, Limits::new(64)).is_ok());
}
