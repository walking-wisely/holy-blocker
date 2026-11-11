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
}

pub fn current() -> DaemonStatus {
    DaemonStatus {
        state: ConnectionState::NotConnected,
        last_heartbeat_at: None,
        watched_windows: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_reports_not_connected_with_no_data() {
        let status = current();
        assert_eq!(status.state, ConnectionState::NotConnected);
        assert_eq!(status.last_heartbeat_at, None);
        assert_eq!(status.watched_windows, 0);
    }

    #[test]
    fn serialises_with_the_field_names_the_renderer_reads() {
        let json = serde_json::to_value(current()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "not-connected",
                "lastHeartbeatAt": null,
                "watchedWindows": 0,
            })
        );
    }
}
