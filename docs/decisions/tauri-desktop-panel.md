# ADR: Tauri desktop panel, in the MVP on macOS

| Field | Value |
|---|---|
| **Status** | Accepted 2026-11-11 on the owner's answers |
| **Date** | 2026-11-11 |
| **Owner** | Ivan Dutov |
| **Supersedes** | The Electron choice in [desktop/plan.md](../components/desktop/plan.md); the "desktop control panel is out of the MVP" and "macOS panel is native in the agent" parts of [mvp-scope.md](mvp-scope.md) |
| **Superseded by** | In part, 2026-11-11: decisions 3 and 5 as to where policy state and the socket live, by [daemon-ipc-and-process-model.md](daemon-ipc-and-process-model.md) decisions 2 and 4 |

Tier: architecture decision that moves a trust boundary (the process holding the UI, its IPC
surface and its path to the daemon), extending a product decision (what the MVP contains). Red-team
tier 2, three lenses, under [decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md);
findings and their dispositions are at the end.

---

## Context

`apps/desktop` is an Electron and React skeleton. Measured 2026-11-11: three of ten steps are done
(`desktop.ipc-handlers`, `desktop.daemon-ipc`, `desktop.wire-daemon-ipc`), all Electron-specific,
and the transport they built is the Windows named pipe `\\.\pipe\holy-blocker-daemon`, which the
macOS MVP cannot use. The macOS daemon's IPC has a framing codec and no socket
([14-daemon-ipc.md](../components/mac-daemon/modules/14-daemon-ipc.md)). Nothing in the repository
mentions Tauri outside one line of `PLAN.md`.

[mvp-scope.md](mvp-scope.md) kept the desktop control panel out of the MVP and put the macOS
custom-app panel natively in the agent. The owner now wants a Tauri panel, for **footprint**, and
wants it in the MVP.

## Decision

1. **`apps/desktop` becomes a Tauri 2 app and Electron is removed.** The React renderer and
   `lucide-react` stay. The main process becomes Rust. Footprint is the stated reason and is
   **not yet measured**: `desktop.tauri-shell` records bundle size and resident memory of the shell
   against the Electron skeleton, so the reason is checked rather than assumed.

2. **The MVP gains a macOS control panel with five contents, and no others:**
   daemon status; content-free block counts; lowering protection through request, cooldown and
   confirm; raising protection immediately; the custom-app list panel. This amends
   [mvp-scope.md](mvp-scope.md) decisions 2 and 3 and
   [custom-app-blocking.md](custom-app-blocking.md): the list panel moves from the agent to this
   app. The list, its store and every cooldown stay in the agent.

3. **The daemon owns every piece of state; the panel is a pure client.** The socket accepts an
   enumerated set of messages and no raw mode set:
   `request_disarm`, `confirm_disarm`, `cancel_disarm`, `raise_protection`, and list requests
   (`list_apps`, `request_add_app`, `request_remove_app`, `confirm_remove_app`, `cancel_remove_app`).
   The cooldown clock is monotonic and lives in the agent, the same machine as
   `mac-daemon.protection-schedule` and `mac-daemon.custom-app-core`. Anything the panel writes
   to disk (window geometry, last tab) is a display cache and is never read back as policy.

   *Amended 2026-11-11 by [daemon-ipc-and-process-model.md](daemon-ipc-and-process-model.md)
   decision 2: the list, the cooldown clock, the tamper log and the block counts live in a root
   LaunchDaemon, not the agent; the agent and the panel are both its clients. Where decisions 2, 4
   and 6 say "the agent" holds state, read "the root daemon". The enumerated message set is
   unchanged.*

4. **Enforcement does not depend on the panel.** Quitting, crashing or removing the panel changes
   no enforcement state. The agent persists its own counts and tamper events; the panel reads
   them.

5. **The transport is a Unix domain socket in a mode 0700 directory, mode 0600, with a
   peer-uid check.** This keeps other users and a browser page out. It does not separate the
   panel from a terminal run by the same person: any process of the protected user can connect.
   That is accepted and is why decision 3 matters. A rogue same-user client can do what the user can
   do, which is start a cooldown, and starting one is recorded in the tamper log.

   *Amended 2026-11-11 by [daemon-ipc-and-process-model.md](daemon-ipc-and-process-model.md)
   decisions 2 and 4: the socket is served by the root daemon from a root-owned directory, no
   longer `~/Library/Application Support/holy-blocker/panel.sock` in the user's home. The peer-uid
   check is replaced by the peer's audit token (`LOCAL_PEERTOKEN`) and a code-signature requirement
   pinned to the Developer ID team and the role's bundle id, with one slot per role; the panel
   checks the daemon's signature the same way. A same-user terminal is no longer accepted as the
   panel.*

6. **Block counts are content-free.** The agent keeps one integer per day for text and domain
   blocks, retained for 30 days, mode 0600, in the agent's own store. There are no window titles,
   application names, scores or histograms. Custom-app enforcement never feeds the counts, which
   keeps [custom-app-blocking.md](custom-app-blocking.md) decision 13 (no per-block record, no
   per-app counter) intact; a test asserts it. The panel shows an in-memory list of the last events
   only as a verdict and a time, never persisted by the panel.

7. **Tauri is configured to have no network and a narrow command surface:**
   - one Rust command per action (status, counts, and the messages of decision 3); no generic
     send-to-socket command, and no command that takes a path;
   - no shell, fs, http, updater or opener plugin; capabilities list the commands by name;
   - CSP `default-src 'self'; connect-src ipc: 'self'`, all assets bundled, no remote content;
   - devtools feature off in release, and no `devUrl` in a release build;
   - panel state lives in Rust-side files with explicit modes, never in webview storage.

   `desktop.release-guard` makes the last three a CI check, because a string in a document proves
   nothing about the built artifact.

8. **Out of the MVP, by name:** a protection-mode switch without cooldown, a threshold editor
   ([classifier-operating-point.md](classifier-operating-point.md): no default threshold ships),
   window titles, a score histogram, the sobriety counter, accountability partner configuration,
   the voice gate, the tray menu, and the Windows transport. The sobriety counter and partner
   email each need their own decision before any step; the counter measures install age today, not
   time since the last lowering ([accountability.md](accountability.md)).

9. **The voice gate stays out of the MVP, and Web Speech is rejected for it.** The re-plan is a
   pending step in the voice-gate component: on-device recognition only, microphone buffer held in
   memory, never written or logged. Whether the Web Speech API runs in `WKWebView`, and whether it
   sends audio off the device in a system WebView, is **unverified** and is not relied on either
   way.

10. **The three finished Electron steps are superseded**, not carried over. Their code (`daemon-ipc.ts`,
    `ipc-handlers.ts`, the Windows pipe client) is deleted by `desktop.tauri-shell`; the Windows
    transport returns as a later, separate step on the Tauri shell.

## Claims, tagged

| Claim | Domain | Status |
|---|---|---|
| The panel is a trust boundary and must hold no policy | engineering | PASS: bypass finding 1 and 2, `config_update` at `DaemonIPC.swift:107-112` carries a raw mode |
| Footprint justifies the rework | product | ESCALATED, decided by the owner; measurement step added |
| Full panel in the MVP | product | ESCALATED, decided by the owner and narrowed to five contents |
| Persisting window titles in a block log | product, legal | ESCALATED, owner chose content-free counts |
| Tauri 2 builds and runs on macOS 26 with Xcode 27 | engineering | UNVERIFIED here: `cargo tauri --version` fails, the CLI is not installed. `desktop.tauri-shell` runs the first build as its first act |
| Tauri capabilities can deny every plugin and command not listed | engineering | UNVERIFIED: `desktop.release-guard` inspects the built `capabilities` and fails on an unlisted command |
| A Unix socket peer uid is readable from Rust on macOS | engineering | UNVERIFIED: `desktop.daemon-socket` states it with `LOCAL_PEERCRED` and a test against a second uid, or records it as untestable |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| Keep Electron | The owner's footprint reason; also leaves a Node main process with the same trust questions as a Rust one and no shared language with the daemon crates |
| Keep the custom-app panel native in the agent, Tauri shows status only | Narrower and cheaper; the owner chose the Tauri list panel. The list store stays in the agent either way |
| A free full/warn/off switch and a threshold editor | One click lowers protection with no cooldown, and a threshold belongs to a (checkpoint, geometry) pair |
| Persist window titles and top offenders | A timestamped history of sexual-content hits, and a per-app usage history for chosen apps; both against [custom-app-blocking.md](custom-app-blocking.md) decision 13 and the counts-only rule in [accountability.md](accountability.md) |
| A raw `config_update` with `protection_mode` over the socket | Any same-user process sets `off` with no cooldown; the red-team's most severe finding |
| The panel writes the custom-app store file directly | A second writer around the cooldown, and it reopens the file-permission analysis |
| A localhost TCP port for the panel to reach the daemon | Reachable by every process including a browser page ([14-daemon-ipc.md](../components/mac-daemon/modules/14-daemon-ipc.md)) |
| `policy.json` in the panel's data directory as the source of mode and thresholds | A user-writable second source of truth |

## Red-team findings and what became of them

Three fresh-context reviews (bypass, intent, legal and privacy), read-only, 2026-11-11. None ran a
command against a live system; every finding below is design-level and labelled so.

| # | Lens | Finding | Disposition |
|---|---|---|---|
| 1 | Bypass | Raw `config_update` with a mode and no sender authentication | Decisions 3 and 5 |
| 2 | Bypass | The old design enforced the gate in the app, a client-side-only gate | Decision 3 |
| 3 | Bypass | `policy.json` as a user-writable second source of truth | Decision 3, last sentence |
| 4 | Bypass | The panel's Cmd-Q is another easy silent route | Decision 4 |
| 5 | Bypass | Tauri capabilities, devtools and `devUrl` are a new surface | Decision 7, `desktop.release-guard` |
| 6 | Bypass | The panel gets write access to the app list | Decision 3 (list edits are requests) |
| 7 | Bypass | Events lost when no client is connected | Decision 4 |
| 8 | Bypass | Updater as a binary-replacement path; admin and removal | Decision 7 (no updater); admin out of scope here as in [mvp-scope.md](mvp-scope.md) |
| 9 | Intent | Full panel contradicts the four MVP requirements | Accepted by the owner as an amendment; narrowed to five contents |
| 10 | Intent | Sobriety counter measures install age | Decision 8 |
| 11 | Intent | Block log invites dwelling and collides with decision 13 | Decision 6 |
| 12 | Intent | Free mode and threshold controls weaken with no friction | Decision 8 |
| 13 | Intent | Footprint is a weak reason inside the MVP | Overruled by the owner; measurement added to decision 1 |
| 14 | Legal and privacy | Persisted log is a new special-category record with no retention bound | Decision 6: 30 days, content-free |
| 15 | Legal and privacy | Window titles are a new data flow | Decision 6 |
| 16 | Legal and privacy | Partner email: local mail and the backend model contradict | Deferred, decision 8 |
| 17 | Legal and privacy | Audio buffer and cloud speech | Decision 9 |
| 18 | Legal and privacy | Tauri updater, remote assets, telemetry | Decision 7 |
| 19 | Legal and privacy | WebView storage and `WKWebView` caches | Decision 7, last bullet |
| 20 | Legal and privacy | Unix socket permissions and peer authentication | Decision 5 |

Finding 13 was overruled, not met: the owner decided the footprint is worth it inside the MVP. The
cost is that `mac-daemon.install`, `mac-daemon.protection-schedule` and the e2e scenario now share
the MVP queue with seven new desktop steps and two new daemon steps.

## What this does not cover

- **Footprint is unmeasured** until `desktop.tauri-shell` runs.
- **Tauri has not built on this machine.** The Rust toolchain is present (rustc 1.90.0); the Tauri
  CLI is not.
- **A same-user process is not separated from the panel** (decision 5). The barrier is the
  cooldown and the tamper log, not socket authentication.
- **A protected user who can stop the agent** is unchanged: kill and `launchctl bootout` are
  detected at next start, not prevented, as in [mvp-scope.md](mvp-scope.md).
- **Remaining Electron references** (`CLAUDE.md`, `CONTRIBUTING.md`, the flows, the security
  skills) are swept by `desktop.tauri-shell`, with the code that makes them untrue.
- **Windows, Android and iOS.** The Android panel is unaffected.
- **Accountability partner and sobriety counter** have no steps until a decision exists.
