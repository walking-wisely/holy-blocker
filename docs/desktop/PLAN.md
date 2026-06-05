# Desktop Control Panel — Implementation Plan

The overall system architecture and component responsibilities are described in [../architecture.md](../architecture.md).
This document is the build plan: what modules to add to `apps/desktop/`, in what order, and what each one is responsible for.

## Current state

The package at `apps/desktop/` already has:

- `src/main/main.ts` — creates a `BrowserWindow` (1080×720, `contextIsolation: true`, `nodeIntegration: false`) and registers one inline `ipcMain.handle` call: `daemon:get-status`, which returns a hardcoded stub `{ state: "not-connected", lastHeartbeatAt: null, watchedWindows: 0 }`.
- `src/preload/preload.ts` — exposes a single `getDaemonStatus()` call through `contextBridge`, bridging `daemon:get-status` to the renderer.
- `src/renderer/src/App.tsx` — renders a two-item sidebar (Monitor / Local Data), a status pill, a metrics grid (Daemon state, Watched windows, Model name), and an empty events panel. Calls `getDaemonStatus` on mount via `window.holyBlocker`.

What is missing: real named pipe IPC to the daemon, a policy threshold settings UI, local event persistence, and the renderer views that consume live data.

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
}

type PolicyThresholds = {
  blockThreshold: number   // 0..100, default 80
  warnThreshold: number    // 0..100, default 50
}
```

### `src/preload/preload.ts` (extend)

Add three new calls to the existing `contextBridge.exposeInMainWorld` object alongside `getDaemonStatus`. No Node APIs should be accessible in the renderer beyond these explicit bridge methods.

- `getEvents: () => Promise<ScanEvent[]>` — invokes `daemon:get-events`.
- `getThresholds: () => Promise<PolicyThresholds>` — invokes `policy:get-thresholds`.
- `setThresholds: (t: PolicyThresholds) => Promise<void>` — invokes `policy:set-thresholds`.

Also extend the `Window.holyBlocker` declaration in `App.tsx` (or a shared `src/renderer/src/types.ts`) to include these three methods so the renderer has full type coverage without importing from the main-process module.

### `src/renderer/src/views/EventsView.tsx`

Replaces the `emptyState` placeholder in `App.tsx` when the Monitor nav item is active.

Responsibilities:

- On mount, call `getEvents()` and populate a list.
- Poll `getEvents()` on a configurable interval (default 2 s) to surface new events until a push mechanism (future IPC event bridge) is available.
- Render each event as a row: window title, a verdict badge coloured by verdict (`block` → red, `warn` → amber, `allow` → green), numeric score, and a relative or absolute timestamp.
- Show the existing empty-state message when the event list is empty.
- Keep the "Flag missed item" button from `App.tsx` visible in the panel header (behaviour to be wired later).

### `src/renderer/src/views/PolicyView.tsx`

Shown when the "Local Data" nav item is active.

Responsibilities:

- On mount, call `getThresholds()` and populate two number inputs: `blockThreshold` and `warnThreshold`.
- On submit, validate client-side (`warnThreshold < blockThreshold`, both `0..100`) and call `setThresholds()`. Display a success confirmation or an inline error.
- Show the current effective values read back from `getThresholds()` after a successful save so the user can confirm the write was persisted.

## Implementation order

1. `ipc-handlers.ts` — extract the existing `daemon:get-status` handler out of `main.ts` and add `daemon:get-events` with an empty ring buffer. Wire `registerIpcHandlers` into `main.ts`. No daemon connection yet; stubs are fine at this step.
2. `daemon-ipc.ts` — named pipe client with the reconnect loop. Write a Vitest test against a mock `net.Server` before wiring it into the app.
3. Wire `DaemonIpc` into `ipc-handlers.ts` so `daemon:get-status` returns live connection state and `daemon:get-events` drains the ring buffer populated by `scan_event` messages.
4. Extend `preload.ts` with `getEvents`, `getThresholds`, and `setThresholds`, and update the renderer-side type declaration.
5. `EventsView.tsx` — render live events from the main process; replace the empty-state div in `App.tsx`.
6. `PolicyView.tsx` — threshold editor wired to `getThresholds` / `setThresholds`; wire the "Local Data" nav item to render this view.

## What this does not cover

- Proxy configuration UI (deferred until the network pipeline packages are further along).
- Rule bundle management UI (deferred until `packages/text-policy` defines a bundle serialisation format).
- Android companion app (separate `apps/mobile/`).
- Packaging, code signing, and auto-update (deferred to a later iteration).
