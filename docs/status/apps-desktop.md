# `apps/desktop` — status

Language: TypeScript + Rust / Tauri 2 + React

Skeleton — Tauri window, one `daemon_status` command that reports `not-connected`, status UI

## Footprint, measured 2026-11-11

Apple silicon, macOS 27.2.1, Xcode 27.2, Electron 44.4.5, Tauri 2.12 with the system WKWebView. Same window, same
status screen, idle 6–10 s after launch. Resident memory is the sum of `ps` RSS over the app's own processes, so it
counts shared framework pages more than once for both; read it as a ratio, not an absolute.

| | Electron skeleton | Tauri shell |
|---|---|---|
| App bundle on disk | 288 MiB (`Electron.app`; the 0.2 MiB renderer sits outside it) | 8.4 MiB (`Holy Blocker.app`, release, renderer included) |
| Resident memory | about 353 MiB, 4 processes | about 205 MiB, 4 processes (app, WebContent, GPU, Networking) |

Not measured: cold start time, battery, behaviour over a long session.

## Shell

`src-tauri` holds the Rust core: one command, `daemon_status`, listed by name in `capabilities/default.json`.
Observed on the release build: the window renders and reports `not-connected` rather than the renderer's
`panel-error`, so the command passes the capability check at runtime. The Electron code, the Windows named-pipe
client and their tests are deleted. CI runs clippy and `cargo test` on `src-tauri` in the macOS desktop job (first run is this PR's). `cargo test` needs the renderer built first, because `generate_context!` panics when `frontendDist` is absent. The dev flow was observed with a plain `PORT` and URL, Vite started by `beforeDevCommand`; `pnpm dev` now runs under `tools.plan.devrun` (reserved port, Vite 200, window launched, port freed on stop). Not yet done: the release guard (`desktop.release-guard`).

## Daemon socket

`src-tauri` has a Unix-socket client for the agent: 4-byte big-endian length, then JSON, capped at 16 MiB
(`frame.rs`); reconnect with backoff from 500 ms to a 30 s cap (`backoff.rs`); `Connected` or `NotConnected`
as the status value `daemon_status` returns (`link.rs`). Five commands send one fixed message each:
`request_disarm`, `confirm_disarm`, `cancel_disarm`, `raise_protection`, `list_apps`. None takes a path or a raw
message; the socket path is built in Rust from `$HOME`. Inbound frames are parsed only as far as a string `type`
tag, and a frame that fails that, or an oversized length prefix, drops the connection and reconnects.

Observed: 24 Rust tests including a fake daemon (connect, reconnect, split frames, garbage and oversized
frames, drop on close); the release app launched under a temporary `HOME` connected to a fake daemon socket
and stayed connected after a valid frame. `tests/vectors/daemon-ipc-framing.json` holds the framing vectors;
the Swift side does not read them yet (`mac-daemon.panel-socket` adopts them).

Not observed: any real agent (no panel socket exists in `mac-daemon` yet, and the socket path here is this
step's choice, to be agreed there); a command call from the webview through the ACL for the five new commands;
the owner and peer-uid check on the socket (the agent's job, decision 5). A `$HOME` long enough to push the
path over macOS's 104-byte `sun_path` limit leaves the panel at `not-connected` for good. Add and remove app
messages carry payloads and arrive with `desktop.custom-app-panel`. `lastHeartbeatAt` is still `None`.
Known limits, filed as `desktop.socket-hardening`: `Connected` is set when the connect succeeds, before any frame, so a silent listener on the path reads as connected; there is no read deadline; the client does not check the socket's owner or mode.
