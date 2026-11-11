# 19. `PanelSocket` — the root daemon's side of the Tauri panel

Part of the [macOS Daemon plan](../plan.md). Step: `mac-daemon.panel-socket`. Decisions:
[tauri-desktop-panel.md](../../../decisions/tauri-desktop-panel.md) (3, 4, 5), amended by
[daemon-ipc-and-process-model.md](../../../decisions/daemon-ipc-and-process-model.md) (2, 3, 4, 5, 6).

Served by the root LaunchDaemon, not the agent. Framing, the `hello` handshake and the limits come from the shared
`daemon-ipc` crate through `mac-daemon.adopt-daemon-ipc-ffi`; `DaemonIPCCodec` is retired. A Unix domain socket in a
root-owned directory set by `mac-daemon.root-policy-store`; clients are authenticated by `mac-daemon.peer-auth`
(audit token and code signature, one slot per role), not by peer uid. The agent is a client too, for its heartbeat and
scan events.

- Messages in: `request_disarm`, `confirm_disarm`, `cancel_disarm`, `raise_protection`, `list_apps`, `request_add_app`,
  `request_remove_app`, `confirm_remove_app`, `cancel_remove_app`. Messages out: status, counts, scan verdicts (verdict and time
  only) and the state of any pending request.
- `config_update` carrying `protection_mode` is removed from the codec. No message sets a mode directly.
- Lowering requests are answered by the root daemon's own `ProtectionSchedule` and custom-app cooldown. The socket never shortcuts
  them, and the panel's absence or termination changes nothing.
- A request, confirm or cancel is a tamper-log event, content-free by the existing test.
- No client connected is not an error. The root daemon persists events itself.
- Test first. Framing and validation are the crate's; the role and slot logic is tested through a fake peer.
- Falsify before coding: what a standard user and a second local user see at the socket path, and that an unsigned
  same-uid process is refused by `mac-daemon.peer-auth`. The cooldown remains the barrier for a request the signed panel
  sends.
