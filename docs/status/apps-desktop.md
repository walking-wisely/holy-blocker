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
client and their tests are deleted. CI runs clippy and `cargo test` on `src-tauri` in the macOS desktop job (first run is this PR's). `cargo test` needs the renderer built first, because `generate_context!` panics when `frontendDist` is absent. The dev flow was observed with a plain `PORT` and URL, Vite started by `beforeDevCommand`; it is unobserved under the Portless HTTPS proxy, which was not running here. Not yet done: the release guard (`desktop.release-guard`).
