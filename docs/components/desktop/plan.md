# Desktop Control Panel — Plan

The system architecture is in [../../architecture/overview.md](../../architecture/overview.md). This is the build plan for
`apps/desktop`. The decision behind it is
[tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md); read it first, since it fixes what the
panel may hold and what it may never do.

## Related flows

- [block](../../product/flows/block.md) — how block events reach the panel
- [warn-interstitial](../../product/flows/warn-interstitial.md)
- [protection-mode-change](../../product/flows/protection-mode-change.md) — rewritten by `desktop.protection-view`: lowering is request, cooldown, confirm, owned by the daemon
- [override-gate](../../product/flows/override-gate.md) — voice gate, outside the MVP

## Current state

`apps/desktop` is an Electron and React skeleton. Three steps landed on it before the framework decision
(`ipc-handlers`, `daemon-ipc`, `wire-daemon-ipc`); they built a Windows named-pipe client in TypeScript and
are superseded: `desktop.tauri-shell` deletes them. The renderer (`src/renderer`) is kept.

## Shape

```
React renderer  ──(named Tauri commands only)──▶  Rust core  ──(Unix socket, peer-checked)──▶  agent / daemon
   no state                                          no policy                                   all state
```

- **The agent owns every state machine.** Protection lowering, the custom-app list and its cooldowns, the tamper
  log and the block counts live in the macOS agent. The panel sends `request_*`, `confirm_*`, `cancel_*` and
  `raise_protection`; it never sets a mode, a threshold or a list entry directly.
- **The panel holds no policy.** Anything it writes (window geometry, last tab) is a display cache and is never read back as
  input.
- **Enforcement does not depend on the panel.** Quitting it changes nothing.
- **No network, narrow surface.** One Rust command per action, no shell, fs, http, updater or opener plugin, CSP with bundled assets only,
  devtools off and no `devUrl` in release, state in Rust-side files and never in webview storage.

## What the macOS MVP panel contains

Status; content-free daily block counts; lowering protection through request, cooldown and confirm; raising it
immediately; the custom-app list panel. Nothing else.

## Steps

The ledger computes order from `depends_on`; each step's specification is in "Step specifications" below. Run
`python3 -m tools.plan.ledger next docs/components/desktop`.

| Group | Steps | MVP |
|---|---|---|
| shell | `desktop.tauri-shell`, `desktop.release-guard` | yes |
| panel | `desktop.daemon-socket`, `desktop.status-view`, `desktop.protection-view`, `desktop.custom-app-panel`, `desktop.e2e-panel` | yes |
| deferred | `desktop.tray`, `desktop.voice-adapter`, `desktop.override-gate-view` | no |

The daemon side lives in `mac-daemon`: `mac-daemon.panel-socket` and `mac-daemon.block-counts`. Cross-component edges are
in the build-order table of [mvp-scope.md](../../decisions/mvp-scope.md), since `depends_on` stays inside one component.

Finished before the decision, superseded by `desktop.tauri-shell`:

1. ~~`ipc-handlers.ts` — extract the `daemon:get-status` handler and add `daemon:get-events`.~~ **Done.**
<!-- step: desktop.ipc-handlers -->
2. ~~`daemon-ipc.ts` — named pipe client with the reconnect loop.~~ **Done.**
<!-- step: desktop.daemon-ipc -->
3. ~~Wire `DaemonIpc` into `ipc-handlers.ts`.~~ **Done.**
<!-- step: desktop.wire-daemon-ipc -->

## Step specifications

### desktop.custom-app-panel

Decision: [tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) (2, 3), replacing `mac-daemon.custom-app-panel`. Behaviour as specified in
[custom-app-blocking.md](../../decisions/custom-app-blocking.md): honest labels and the coverage note, manual identity
entry, the removal cost stated at add time, refusal of `never` identities, derived tokens shown and confirmed.
Enumeration is in memory and discarded when the panel closes. The panel never opens the store file.

### desktop.daemon-socket

**Done.**

Decision: [tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) (3, 5).

Rust client with the same framing as `DaemonIPCCodec`. The framing vectors are written and read on the Rust side; the Swift side adopting them is part of `mac-daemon.panel-socket`. Reconnect with
backoff (500 ms start, 30 s cap). Connection state is a status value the UI reads. No command accepts a path or a raw message.
The five payload-free messages have commands; `request_add_app`, `request_remove_app`, `confirm_remove_app` and `cancel_remove_app` carry
payloads and come with `desktop.custom-app-panel`. The shared vectors live in `apps/desktop/src-tauri/tests/vectors/`.

### desktop.socket-hardening

<!-- step: desktop.socket-hardening -->

Found reviewing `desktop.daemon-socket`; none blocks it, because the agent's side of decision 5 owns the peer check.

- After connect, check the peer uid (`LOCAL_PEERCRED`) and refuse a socket that is not owned by the user or not mode 0600, or a `$HOME` that is empty or relative.
- `Connected` only after the first valid frame, plus a read deadline, so a silent or trickling peer cannot hold the status at connected.
- Reset the backoff only after a connection has lasted, so a peer that sends one frame and closes does not cause a reconnect every 500 ms.
- Write through a channel to a writer thread instead of holding the writer lock across a blocking write; recover from a poisoned lock.
- `Drop` rechecks `stop` after the writer is installed.

Needs the agent's socket (`mac-daemon.panel-socket`) to settle the connect-time contract first.

### desktop.e2e-panel

The feature's demo, per [feature-demos-and-local-e2e.md](../../decisions/feature-demos-and-local-e2e.md): headless as a regression
check and `--interactive` as the owner's demo. Needs `engineering.e2e-contract` and `engineering.e2e-runner`.
Layers: pure framing and cooldown logic at unit level, the socket at integration level, the panel and agent together here.

### desktop.protection-view

Decision: [tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) (2, 3). The cooldown is the agent's; this view displays its state and sends request, confirm or cancel.
No full/warn switch and no threshold editor. Closing the view or the app changes nothing about a pending request.

### desktop.release-guard

Decision: [tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) (7).

Inspect the built artifact, not the source: the resolved capabilities, the CSP in the bundled config, the plugin
list linked into the binary, and the absence of the `devtools` feature. Fails on any plugin or command not on an
allowlist. Add the macOS case to `engineering.dev-build-release-guard-macos`'s reach if that step lands first.

### desktop.status-view

Decision: [tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) (2, 6). Shows what the agent reports and persists nothing itself. No titles, scores, histograms or per-app rows.
Verdict-and-time events are held in memory only.

### desktop.tauri-shell

**Done.** Toolchain verified on macOS 27.2.1 with Xcode 27.2; footprint is in [apps-desktop status](../../status/apps-desktop.md).

Decision: [tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) (1, 7, 10).

First act: `cargo install tauri-cli` or the pnpm CLI and run a hello-world build on this machine. If it does not
build on macOS 26 with Xcode 27, stop and report; do not continue on an unverified toolchain.

- Rust crate under `apps/desktop/src-tauri`, React renderer unchanged in behaviour, dev flow kept on `devrun`
  (`PORT`, `DEV_URL`; single-quote the deferred expansion, see `CLAUDE.md`).
- Capabilities list commands by name. No shell, fs, http, updater or opener plugin. CSP and bundled assets as in the decision.
- Delete `electron`, `src/main`, `src/preload`, the Windows pipe client and their tests; keep a minimal status screen.
- Record bundle size and resident memory of the built shell next to the Electron skeleton's, in `docs/status/apps-desktop.md`.
  Measure Electron before deleting it.
- Sweep `CLAUDE.md`, `CONTRIBUTING.md`, the three flows, `docs/engineering/security-backlog.md` and the security and
  privacy skills for Electron wording, replacing context isolation guidance with the capability model.
- Security and privacy reviews are mandatory: this removes and replaces the preload/IPC surface.

## What this does not cover

- **The Windows transport.** The Windows daemon's named pipe returns as its own step on the Tauri shell, after the macOS MVP.
- **The sobriety counter and accountability partner configuration.** Each needs a decision first; the counter measured install
  age, not time since the last lowering, and [sync-plan.md](sync-plan.md) and the old plan disagree on how partner mail is sent.
  [sync-plan.md](sync-plan.md) is deferred as a whole and still names Electron.
- **A mode switch without cooldown, a threshold editor, window titles, a score histogram and per-app counts.**
- **The voice gate.** `desktop.voice-adapter` and `desktop.override-gate-view` wait for `voice-gate.desktop-adapter`, which re-plans
  recognition as on-device only.
- **The tray menu.** Its old items included a gate-free mode switch and a quit; it needs the same rules before it is built.
- Proxy configuration UI, rule bundle management, packaging, signing and auto-update.
- The Android companion app (`apps/mobile`).
