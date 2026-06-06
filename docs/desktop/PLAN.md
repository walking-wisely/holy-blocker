# Desktop Control Panel — Implementation Plan

The overall system architecture and component responsibilities are described in [../architecture.md](../architecture.md).
This document is the build plan: what modules to add to `apps/desktop/`, in what order, and what each one is responsible for.

## Related flows

- [../flows/block.md](../flows/block.md) — how block events reach the desktop ring buffer and tray
- [../flows/warn-interstitial.md](../flows/warn-interstitial.md) — warn events and future native overlay
- [../flows/protection-mode-change.md](../flows/protection-mode-change.md) — mode selector → policy.json → daemon/proxy propagation
- [../flows/override-gate.md](../flows/override-gate.md) — voice gate triggered when switching to Off

## Current state

The package at `apps/desktop/` already has:

- `src/main/main.ts` — creates a `BrowserWindow` (1080×720, `contextIsolation: true`, `nodeIntegration: false`) and registers one inline `ipcMain.handle` call: `daemon:get-status`, which returns a hardcoded stub `{ state: "not-connected", lastHeartbeatAt: null, watchedWindows: 0 }`.
- `src/preload/preload.ts` — exposes a single `getDaemonStatus()` call through `contextBridge`, bridging `daemon:get-status` to the renderer.
- `src/renderer/src/App.tsx` — renders a two-item sidebar (Monitor / Local Data), a status pill, a metrics grid (Daemon state, Watched windows, Model name), and an empty events panel. Calls `getDaemonStatus` on mount via `window.holyBlocker`.

What is missing: real named pipe IPC to the daemon, a policy threshold settings UI, local event persistence, the renderer views that consume live data, the accountability/sobriety features, and the override gate.

## What to add

### `src/main/daemon-ipc.ts`

Manages the persistent connection from the main process to the Windows daemon over the named pipe `\\.\pipe\holy-blocker-daemon`.

Responsibilities:

- Connect to the pipe using Node.js `net.Socket` and keep a single live connection.
- Reconnect with exponential backoff (starting at 500 ms, capped at 30 s) whenever the pipe is unavailable or the socket closes unexpectedly.
- Read the byte stream and split on newlines to extract individual JSON messages (newline-delimited JSON protocol).
- Parse each message into a typed `DaemonMessage` union and emit it via `EventEmitter` so `ipc-handlers.ts` can subscribe without coupling to the socket directly.
- Expose `send(msg: object): void` to push config messages (e.g. threshold updates) back to the daemon.
- Track connection state (`connecting | connected | disconnected`) so `ipc-handlers.ts` can report it through `daemon:get-status`.

Key types:

```ts
type DaemonMessage =
  | { type: "heartbeat"; at: string }
  | { type: "scan_event"; verdict: "block" | "warn" | "allow"; score: number; windowTitle: string; at: string }
  | { type: "status_update"; watchedWindows: number }

type ConnectionState = "connecting" | "connected" | "disconnected"
```

Testing: connect `DaemonIpc` against a mock `net.Server` that writes newline-delimited JSON to verify message parsing, reconnect behaviour, and `send` round-trips. A Vitest test suite in `src/main/__tests__/daemon-ipc.test.ts` is the target location.

### `src/main/ipc-handlers.ts`

Registers all `ipcMain.handle` calls in one place, replacing the inline handler in `main.ts`.

Responsibilities:

- `daemon:get-status` — return the live connection state and last-known `watchedWindows` count from `DaemonIpc`, plus the timestamp of the most recent heartbeat message.
- `daemon:get-events` — return the last N `scan_event` messages from a ring buffer (capped at 500 entries) held in the main process. N is caller-supplied, defaulting to 100.
- `policy:get-thresholds` — read `{ blockThreshold, warnThreshold }` from a JSON config file at `path.join(app.getPath("userData"), "policy.json")`. Return defaults `{ blockThreshold: 80, warnThreshold: 50 }` if the file does not exist.
- `policy:set-thresholds` — validate (`warnThreshold < blockThreshold`, both in `0..100`) and write the config file atomically (write to a `.tmp` sibling, then rename).
- `protection:get-mode` — read `protectionMode` from `policy.json`. Return default `"full"` if unset.
- `protection:set-mode` — validate value ∈ `{ "full", "warn", "off" }`. If `"off"`, run the override gate first (see `override:attempt`). Write atomically to `policy.json`, then push `config_update` to daemon over named pipe. See [../flows/protection-mode-change.md](../flows/protection-mode-change.md).
- `stats:get-summary` — return lifetime and weekly event counts, total blocked-content tally, and the install timestamp (used by the sobriety counter).
- `accountability:get-config` — read `{ partnerEmail: string | null }` from `userData/accountability.json`.
- `accountability:set-config` — write partner email; validate it is a well-formed address.
- `accountability:notify-override-attempt` — send an email notification to the partner (if configured) recording the timestamp of an override attempt. Called regardless of whether the gate ultimately passes or fails.
- `override:attempt` — invoked by the renderer when the user tries to disable protection. Triggers the voice gate flow and returns `{ granted: boolean }`. If granted, emits a `protection:disabled` event on the main process bus and records an override event in the stats store.

Export a single `registerIpcHandlers(daemonIpc: DaemonIpc): void` function called from `main.ts` during app startup.

Key types:

```ts
type DaemonStatus = {
  state: ConnectionState
  lastHeartbeatAt: string | null
  watchedWindows: number
}

type ScanEvent = {
  verdict: "block" | "warn" | "allow"
  score: number
  windowTitle: string
  at: string
  flaggedAsFalsePositive?: boolean
}

type ProtectionMode = "full" | "warn" | "off"
// full   — Block verdicts block; Warn verdicts are recorded but traffic passes.
// warn   — Block and Warn verdicts are downgraded; traffic always passes, events recorded.
// off    — Scanning disabled; all traffic passes. Requires voice gate to activate.

type PolicyThresholds = {
  blockThreshold: number   // 0..100, default 80
  warnThreshold: number    // 0..100, default 50
}

type StatsSummary = {
  installTimestamp: string   // ISO-8601; set once on first launch, never overwritten
  lifetimeBlocked: number
  lifetimeWarned: number
  weeklyBlocked: number
  weeklyWarned: number
  scoreHistogram: number[]   // 20 buckets covering 0..100
  topWindows: { title: string; count: number }[]  // top 5 by event count
}
```

### `src/main/stats-store.ts`

Persists event counts and the install timestamp across sessions.

Responsibilities:

- On first launch, record `installTimestamp` in `userData/stats.json` and never overwrite it.
- Increment `lifetimeBlocked` / `lifetimeWarned` each time a `scan_event` arrives from the daemon.
- Maintain a rolling 7-day event log for weekly counts (entries older than 7 days are pruned on startup).
- Maintain a score histogram (20 equal-width buckets across 0–100) for the score distribution chart.
- Maintain a window-title frequency map for the "top offending windows" list; cap at 100 unique titles.
- Expose `getSummary(): StatsSummary` consumed by `stats:get-summary`.
- Write to disk atomically (`.tmp` + rename) after each update.

### `src/preload/preload.ts` (extend)

Add new calls to the existing `contextBridge.exposeInMainWorld` object. No Node APIs should be accessible in the renderer beyond these explicit bridge methods.

- `getEvents: () => Promise<ScanEvent[]>` — invokes `daemon:get-events`.
- `flagFalsePositive: (at: string) => Promise<void>` — marks the event at the given timestamp as a false positive in the ring buffer and stats store.
- `getThresholds: () => Promise<PolicyThresholds>` — invokes `policy:get-thresholds`.
- `setThresholds: (t: PolicyThresholds) => Promise<void>` — invokes `policy:set-thresholds`.
- `getProtectionMode: () => Promise<ProtectionMode>` — invokes `protection:get-mode`.
- `setProtectionMode: (mode: ProtectionMode) => Promise<void>` — invokes `protection:set-mode`. The main process gates the `"off"` transition behind the voice gate before writing and propagating the change.
- `getStatsSummary: () => Promise<StatsSummary>` — invokes `stats:get-summary`.
- `getAccountabilityConfig: () => Promise<{ partnerEmail: string | null }>` — invokes `accountability:get-config`.
- `setAccountabilityConfig: (cfg: { partnerEmail: string | null }) => Promise<void>` — invokes `accountability:set-config`.
- `attemptOverride: () => Promise<{ granted: boolean }>` — invokes `override:attempt`.

Also extend the `Window.holyBlocker` declaration in a shared `src/renderer/src/types.ts` to include all methods so the renderer has full type coverage.

### `src/renderer/src/views/MonitorView.tsx`

Replaces the `emptyState` placeholder in `App.tsx` when the Monitor nav item is active.

Responsibilities:

- Render the sobriety header: a large "Clean since" counter showing days (and hours for the
  first 48 h) derived from `StatsSummary.installTimestamp`. Display milestone text at 7, 30,
  90, and 365 days as a quiet banner above the event list — not a modal, just an inline note.
- Show a weekly summary card: blocked this week, warned this week, vs. last week's counts.
- Show a score distribution histogram using the 20-bucket data from `StatsSummary.scoreHistogram`.
  A small bar chart (CSS only, no charting library) is sufficient. This helps the user see
  whether their thresholds are calibrated correctly.
- Show a "top offending windows" list: up to 5 window titles with their event counts.
- Below the summary, render the live event list (previously the `EventsView` scope):
  - On mount, call `getEvents()` and populate the list.
  - Poll every 2 s until a push mechanism is available.
  - Each row: window title, a verdict badge (`block` → red, `warn` → amber, `allow` → green),
    numeric score, relative timestamp, and a "false positive" flag button.
  - The flag button calls `flagFalsePositive(event.at)` and immediately greys the row to
    confirm the action. False positives are excluded from the score histogram and top-windows
    list going forward.
- Show the existing empty-state message when the event list is empty.

### `src/renderer/src/views/PolicyView.tsx`

Shown when the "Settings" nav item is active (rename "Local Data" → "Settings" to better
reflect the combined content of this view).

Responsibilities:

- **Protection mode selector**: on mount, call `getProtectionMode()` and render a
  three-option segmented control or radio group:
  - **Full protection** — blocks content that exceeds the block threshold; the normal operating mode.
  - **Warn only** — scans still run and events are recorded, but nothing is ever blocked; traffic always passes.
  - **Off** — scanning is disabled entirely. Selecting this option immediately triggers the voice gate overlay; if the gate fails, the mode is not changed.
  A short description under each option explains what the user will and won't see. The current mode is highlighted; changing it calls `setProtectionMode()`.
- **Threshold editor**: on mount, call `getThresholds()` and populate two number inputs.
  On submit, validate client-side and call `setThresholds()`. Confirm the written values
  by re-reading after a successful save. Grey out the threshold inputs when mode is `"off"` since thresholds have no effect in that state.
- **Accountability partner**: on mount, call `getAccountabilityConfig()` and display the
  current partner email (or empty). A text input + save button writes via
  `setAccountabilityConfig()`. A short explanation tells the user what the partner receives
  (a notification on every override attempt, regardless of whether the gate passes).

### `src/renderer/src/views/OverrideGateView.tsx`

A full-screen modal overlay shown when the user attempts to disable protection (any path that
calls `attemptOverride()`). It should not be dismissable by clicking outside — the only exit
is completing the gate or killing the app.

Responsibilities:

- Display the verse the user must read, sourced from `packages/voice-gate`'s `VoiceGate.currentVerse()`.
- Show a "Start reading" button that activates the microphone and begins recording via the
  voice adapter.
- While recording, show a simple animated indicator (not a countdown — the user should not
  be able to time themselves to stop at exactly the right moment).
- On completion, call `attemptOverride()` via IPC. The main process runs the full voice gate
  (transcript match + rate check + liveness check) and returns `{ granted: boolean }`.
- On failure, display which check failed and offer one retry with a fresh verse.
- On success, close the overlay and proceed with the action the user requested.
- The accountability partner notification is sent by the main process on every attempt
  (pass or fail) — the renderer does not need to handle this separately.

### `src/main/voice-adapter-electron.ts`

Implements the `VoiceAdapter` interface from `packages/voice-gate` for the Electron main
process. The renderer captures audio and runs speech recognition; results are sent to the
main process via a dedicated IPC channel rather than crossing the node/renderer boundary
at the adapter level.

Responsibilities:

- Expose IPC channels `voice:start`, `voice:stop` so the renderer can signal recording state.
- The renderer uses `MediaRecorder` for audio and the Web Speech API for transcription,
  sending the final `{ audioBuffer, transcript, durationMs }` back via `voice:result`.
- The adapter collects this result and fulfils the `VoiceAdapter.stopRecording()` promise.

## System tray integration

The main process should register a tray icon. See [../flows/protection-mode-change.md](../flows/protection-mode-change.md) for how mode changes update the icon.

Icon colour by condition (evaluated in priority order):

| Condition | Colour |
|---|---|
| Mode `"off"` | Grey |
| Daemon disconnected | Red |
| `block` event in last hour | Red |
| `warn` event in last hour | Amber |
| Mode `"warn"`, no recent events | Blue |
| Mode `"full"`, no recent events | Green |

Right-click menu items:
- "Open Holy Blocker"
- *(separator)*
- Current mode as a non-clickable label: `"Full protection ✓"` / `"Warn only ✓"` / `"Off"`
- "Switch to Warn only" or "Switch to Full" (toggles between `full` and `warn`, no gate)
- "Disable protection…" (triggers override gate) — hidden when already `"off"`
- "Re-enable protection" (instant, no gate) — shown only when mode is `"off"`
- *(separator)*
- "Quit"

The tray icon is the user's ambient signal that the app is running. Without it, there is no
visible feedback when the window is closed.

## Implementation order

1. `ipc-handlers.ts` — extract the existing `daemon:get-status` handler out of `main.ts` and add `daemon:get-events` with an empty ring buffer. Wire `registerIpcHandlers` into `main.ts`. No daemon connection yet; stubs are fine at this step.
2. `daemon-ipc.ts` — named pipe client with the reconnect loop. Write a Vitest test against a mock `net.Server` before wiring it into the app.
3. Wire `DaemonIpc` into `ipc-handlers.ts` so `daemon:get-status` returns live connection state and `daemon:get-events` drains the ring buffer populated by `scan_event` messages.
4. `stats-store.ts` — event persistence and install timestamp. Add `stats:get-summary` to `ipc-handlers.ts`.
5. Extend `preload.ts` with all new bridge methods and update `src/renderer/src/types.ts`.
6. `MonitorView.tsx` — sobriety counter, weekly summary, score histogram, top windows, live event list with false-positive flagging.
7. `PolicyView.tsx` — protection mode selector, threshold editor, and accountability partner config.
8. `voice-adapter-electron.ts` + wire `packages/voice-gate` into `ipc-handlers.ts` for `override:attempt`.
9. `OverrideGateView.tsx` — full-screen verse-reading overlay.
10. System tray icon and right-click menu.

## What this does not cover

- Proxy configuration UI (deferred until the network pipeline packages are further along).
- Rule bundle management UI (deferred until `packages/text-policy` defines a bundle serialisation format).
- Android companion app (separate `apps/mobile/`).
- Packaging, code signing, and auto-update (deferred to a later iteration).
- Streak calendar heatmap (good future addition once the rolling event log in `stats-store.ts` exists — the data model already supports it).
