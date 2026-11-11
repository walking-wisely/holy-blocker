# 19. `PanelSocket` — the agent's side of the Tauri panel

Part of the [macOS Daemon plan](../plan.md). Step: `mac-daemon.panel-socket`. Decision:
[tauri-desktop-panel.md](../../../decisions/tauri-desktop-panel.md) (3, 4, 5).

Opens the real transport behind `DaemonIPCCodec` ([module 14](14-daemon-ipc.md), whose framing is reused). A Unix domain
socket in a mode 0700 directory, mode 0600, with a peer-uid check.

- Messages in: `request_disarm`, `confirm_disarm`, `cancel_disarm`, `raise_protection`, `list_apps`, `request_add_app`,
  `request_remove_app`, `confirm_remove_app`, `cancel_remove_app`. Messages out: status, counts, scan verdicts (verdict and time
  only) and the state of any pending request.
- `config_update` carrying `protection_mode` is removed from the codec. No message sets a mode directly.
- Lowering requests are answered by the agent's own `ProtectionSchedule` and custom-app cooldown. The socket never shortcuts
  them, and the panel's absence or termination changes nothing.
- A request, confirm or cancel is a tamper-log event, content-free by the existing test.
- No client connected is not an error. The agent persists events itself.
- Test first. Framing and validation are pure; the peer-uid check is tested through a fake peer.
- Falsify before coding: peer uid is readable on a macOS Unix socket from Swift (`getpeereid`), and what a second local user
  sees at the socket path. A same-user process can always connect; the cooldown, not the socket, is the barrier.
