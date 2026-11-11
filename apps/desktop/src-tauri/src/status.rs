use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub state: ConnectionState,
    pub last_heartbeat_at: Option<String>,
    pub watched_windows: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    NotConnected,
    Connected,
}

pub fn snapshot(state: ConnectionState) -> DaemonStatus {
    DaemonStatus {
        state,
        last_heartbeat_at: None,
        watched_windows: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_carries_the_state_with_no_other_data() {
        let status = snapshot(ConnectionState::NotConnected);
        assert_eq!(status.state, ConnectionState::NotConnected);
        assert_eq!(status.last_heartbeat_at, None);
        assert_eq!(status.watched_windows, 0);
    }

    #[test]
    fn serialises_with_the_field_names_the_renderer_reads() {
        let json = serde_json::to_value(snapshot(ConnectionState::NotConnected)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "not-connected",
                "lastHeartbeatAt": null,
                "watchedWindows": 0,
            })
        );
    }

    #[test]
    fn connected_serialises_in_kebab_case() {
        let json = serde_json::to_value(ConnectionState::Connected).unwrap();
        assert_eq!(json, serde_json::json!("connected"));
    }
}
