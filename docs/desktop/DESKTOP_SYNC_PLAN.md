# Desktop Plan — Sync & Gap Resolution

This document records the gaps found between `docs/desktop/PLAN.md` and the rest of the
documentation (flows, decisions, architecture). It also captures the decision to introduce
a lightweight backend service, which the partner setup flow makes unavoidable.

Read alongside [PLAN.md](PLAN.md). Items here either amend the plan or flag decisions that
must be made before a section of the plan is implemented.

---

## 1. Backend introduction

### Why it can't be deferred

The partner setup flow (`flows/partner-setup.md`) requires server-side infrastructure that
cannot be replicated locally:

- Host the partner landing page at `https://holyblocker.app/invite/{token}`
- Store invite tokens (UUID → expiry, status, partner contact once confirmed)
- Send transactional emails: gate attempt notifications, weekly summaries, milestone
  emails, encouragement delivery
- Mark tokens confirmed and push the event to the waiting desktop app

None of these can be done with a purely local, offline client.

### Scope for v1

The v1 backend is a **lightweight notification and invite service**, not a federated ML
coordinator. The two concerns are architecturally distinct and should not be conflated at
this stage:

| Concern | v1 scope |
|---|---|
| Partner invite flow | Yes — tokens, email, confirmation webhook |
| Weekly summary delivery | Yes — scheduled email per active partner |
| Gate attempt notifications | Yes — triggered by desktop IPC |
| Federated ML aggregation | No — deferred to Phase 2 (no training data yet) |
| Device HMAC enrollment | No — deferred to Phase 2 |
| Enclave attestation | No — deferred to Phase 3 |

### Technology

Go is a good fit: small binary, straightforward `net/http`, easy to deploy. Keep the
invite/notification service and the future ML coordinator as **separate Go packages** within
a `backend/` directory from the start, so the boundary is clear before Phase 2 begins.

For the desktop ↔ backend connection while an invite is pending, simple HTTP polling
(every 30 s) is sufficient. A persistent WebSocket connection is unnecessary overhead
at this stage.

### Architecture doc update required

`docs/architecture.md` should be updated to acknowledge the backend service and its scope.
The current document describes a purely local system. The backend does not weaken the
local-first model — no scanning data, no content, no URLs are sent to the backend. Only
the partnership metadata (invite token, partner email, streak counts) crosses the wire.

---

## 2. Gaps in `docs/desktop/PLAN.md`

### 2a. Remove "Clean since install" — replace with streak

The `MonitorView` spec describes a "Clean since" counter derived from `installTimestamp`.
This is both theologically misleading (implies the tool is the source of cleanness) and
technically wrong (it never resets, making it a meaningless increasing counter).

**Resolution:** remove the "Clean since" concept entirely. Display the streak instead.

The streak is already defined in `flows/partner-setup.md`:
- Increments +1 day at midnight if no successful gate override occurred that day
- Resets to 0 when protection mode transitions to `"off"` (gate cleared)
- Does NOT reset on warn events, cancelled gate attempts, or Full ↔ Warn switches

**Required changes:**

`StatsSummary` type needs a `currentStreak` field and the `installTimestamp` field
can be removed (the only remaining use was the discarded counter):

```ts
type StatsSummary = {
  lifetimeBlocked: number
  lifetimeWarned: number
  weeklyBlocked: number
  weeklyWarned: number
  currentStreak: number         // days without a successful gate override
  lastStreakResetAt: string | null  // ISO-8601; null if never reset
  scoreHistogram: number[]
  topWindows: { title: string; count: number }[]
}
```

`stats-store.ts` responsibilities expand to:
- Track `currentStreak` and `lastStreakResetAt` in `userData/stats.json`
- On app launch and at midnight, increment the streak if no override occurred since the
  last increment
- On a successful gate override, reset `currentStreak` to 0 and record `lastStreakResetAt`
- Expose streak data via `getSummary()`

`MonitorView.tsx` replaces the "Clean since" header with a streak display:
- `{currentStreak} days` as the primary number
- Milestone text at 7, 30, 90, 180, 365 days (inline banner, same as before)
- "Your streak resets if protection is disabled" as a quiet sub-label
- No header shown when `currentStreak === 0` — show a neutral starting message instead

### 2b. WarnOverlay is missing from the plan

`flows/warn-interstitial.md` specifies two warn overlay paths:

1. **Proxy path (web content):** the proxy injects the overlay directly into the HTML
   response. The desktop is not involved in rendering — it only receives a `scan_event`
   for bookkeeping.

2. **Daemon path (native apps):** when the daemon detects a warn verdict via OCR or image
   classification, there is no HTML to inject into. The desktop must spawn a frameless
   `BrowserWindow` with `alwaysOnTop: true` covering the full primary screen.

Path 2 is completely absent from `PLAN.md`. Add the following:

**`src/renderer/src/views/WarnOverlay.tsx`**

A full-screen overlay component rendered inside a dedicated frameless `BrowserWindow`
created by the main process on demand.

Responsibilities:
- Receive `{ capturedFrameB64, score, category }` via a dedicated IPC channel
  (`warn:show`) when the BrowserWindow is created
- Display `capturedFrameB64` as a blurred backdrop (`filter: blur(20px)`) — never stored,
  only used as a visual context cue for the user
- Show the verse selected by `category` from `data/verses/warn.json` (imported as a
  static Vite JSON module)
- Provide two buttons:
  - **"I understand, continue"** — closes the overlay window, records in-memory
    suppression for this window title for 5 minutes
  - **"Close"** — same effect as "I understand" (deliberate dismissal counts the same)
- Send `warn:dismissed` back to main process when either button is pressed so the daemon
  can be notified via named pipe

**Main process changes (add to `ipc-handlers.ts` or a new `warn-overlay.ts`):**
- Listen for `scan_event { action: "warn" }` messages from `DaemonIpc`
- Check in-memory suppression map (window title → expiry timestamp); skip if suppressed
- Create frameless `BrowserWindow` with `alwaysOnTop: true`, `transparent: true`,
  bounds = full primary monitor; load the overlay renderer page
- Pass event data via the new `warn:show` IPC channel after the window loads
- Destroy the window when `warn:dismissed` is received

**`scan_event` shape from daemon (update to match `flows/warn-interstitial.md`):**

The daemon's `scan_event` message should include `capturedFrameB64` for warn verdicts
so the overlay can use it as a blurred backdrop. This field is absent from the current
`DaemonMessage` type in `daemon-ipc.ts`. Update:

```ts
type DaemonMessage =
  | { type: "heartbeat"; at: string }
  | { type: "scan_event"; verdict: "block" | "warn" | "allow"; score: number;
      windowTitle: string; at: string;
      capturedFrameB64?: string }   // present for "warn" from daemon path
  | { type: "status_update"; watchedWindows: number }
```

**Implementation order:** add `WarnOverlay.tsx` between steps 6 and 7 of the current
implementation order (after MonitorView, before PolicyView).

### 2c. Partner setup uses invite-token model, not direct email entry

`ipc-handlers.ts` currently specifies:
- `accountability:get-config` → `{ partnerEmail: string | null }`
- `accountability:set-config` → validates email and writes

This does not match the actual flow (`flows/partner-setup.md`). The user never enters the
partner's email. Instead:

1. User clicks "Add accountability partner" → app generates a UUID invite token
2. App constructs `https://holyblocker.app/invite/{token}` and shows it with copy/send options
3. Partner opens link → enters their own name + email on the web landing page → confirms
4. Backend marks token confirmed and notifies the desktop (polled)
5. Desktop updates UI to show "Partner: [Name] ✓ Active"

**Replace** `accountability:get-config` / `accountability:set-config` with:

```ts
// accountability:get-status → AccountabilityStatus
type AccountabilityStatus =
  | { state: "none" }
  | { state: "pending"; token: string; expiresAt: string }
  | { state: "active"; partnerName: string }

// accountability:generate-invite → { token: string; link: string; expiresAt: string }
// accountability:cancel-invite → void (clears pending state, does not contact backend)
// accountability:poll-invite → AccountabilityStatus (single poll; UI calls on interval)
// accountability:remove-partner → void (posts remove to backend, clears local state)
```

The backend owns the token store. The desktop stores only the current token (for polling)
and the confirmed partner name (once active). Partner email is never held by the desktop —
it lives on the backend and is used only for email delivery.

**`PolicyView.tsx` (formerly "Settings") accountability section changes:**
- State `"none"`: show "Add accountability partner" button
- State `"pending"`: show invite link with copy button, quick-send options (email/messages),
  "Invite pending — waiting for [no name yet] to confirm", expiry date, and "Cancel" button;
  poll `accountability:poll-invite` every 30 s while this view is mounted
- State `"active"`: show "Partner: [Name] ✓ Active", weekly summary info, "Remove partner"
  button with confirmation prompt

### 2d. Two-stage accountability notifications on gate attempt

`flows/override-gate.md` specifies two notifications, not one:

1. **Immediately when the gate opens** — before the outcome is known:
   `"An override attempt was started on <device> at <timestamp>"`
2. **On gate completion** — pass or fail:
   `"Override attempt failed"` or `"Protection was disabled"`

The current `ipc-handlers.ts` has a single `accountability:notify-override-attempt` call.
This needs to become two separate backend calls:

- `accountability:notify-gate-started(timestamp)` — called at the start of `override:attempt`
- `accountability:notify-gate-outcome(outcome: "cancelled" | "disabled", timestamp)` — called
  after the gate resolves

Both are fire-and-forget HTTP POSTs to the backend (no user-facing result needed). They
are no-ops when no partner is active.

### 2e. `flagFalsePositive` IPC handler missing

The preload bridge exposes `flagFalsePositive(at: string)` but no corresponding handler
is registered in `ipc-handlers.ts`. Add:

- `daemon:flag-false-positive` handler — locate the event in the ring buffer by `at`
  timestamp, set `flaggedAsFalsePositive: true`, and forward the flag to `stats-store`
  so the event is excluded from the score histogram and top-windows frequency map

### 2f. Verse data files must be bundled

`decisions/verse-selection.md` specifies that the desktop imports verse pool JSON as static
Vite modules:

- `data/verses/warn.json` — used by `WarnOverlay.tsx` (category → verse mapping)
- `data/verses/gate.json` — used by `OverrideGateView.tsx` (gate verse pool)

Neither file exists in the repo yet. They need to be created from the curated content in
`docs/decisions/verse-pools.md` and verified against the NIV licensing decision described
there (WEB as the safe fallback for the distributed binary).

The files also need a Vite alias or `resolveAlias` so `apps/desktop` can import from
`data/verses/` without a fragile relative path. Add to `vite.config.ts`:

```ts
resolve: {
  alias: {
    "@verses": path.resolve(__dirname, "../../data/verses")
  }
}
```

### 2g. Onboarding flow is absent

`decisions/accountability.md` specifies that partner setup should appear as a named step in
initial onboarding — not buried in settings. The desktop plan has no onboarding view.

Add a minimal onboarding flow triggered on first launch (detected by absence of
`userData/onboarding-complete` marker):

**`src/renderer/src/views/OnboardingView.tsx`**

A step-based full-screen view shown instead of `App.tsx` on first launch. Steps:

1. **Welcome** — brief description of what Holy Blocker does; "Get started" button
2. **Protection mode** — same segmented control as PolicyView; default is `"full"`;
   explanation of each mode; "Next" button
3. **Accountability partner** — "Add a partner" button (triggers invite flow inline) or
   "Skip for now" (solo mode, no friction); short explanation of what solo mode means
4. **Done** — "Start Holy Blocker"; writes `userData/onboarding-complete`, transitions to
   main app shell

Steps 1, 4, and "Skip for now" require no new IPC. Step 2 reuses `setProtectionMode`.
Step 3 reuses `accountability:generate-invite` and the polling flow.

**Add to `main.ts`:** on app launch, check for `userData/onboarding-complete` and pass
a boolean to the renderer via a one-time IPC message so it knows whether to show onboarding
or the main shell.

---

## 3. Implementation order update

The revised implementation order, incorporating the gaps above:

1. `ipc-handlers.ts` — extract existing handler, add `daemon:get-events` stub,
   add `daemon:flag-false-positive` *(was missing)*, wire `registerIpcHandlers` into `main.ts`
2. `daemon-ipc.ts` — named pipe client + reconnect loop; update `DaemonMessage` to
   include optional `capturedFrameB64` on warn events
3. Wire `DaemonIpc` into `ipc-handlers.ts` — live status + ring buffer
4. `stats-store.ts` — event persistence; replace `installTimestamp` with streak tracking;
   add `stats:get-summary` handler
5. Extend `preload.ts` + `src/renderer/src/types.ts`
6. Create `data/verses/warn.json` and `data/verses/gate.json` from the curated pool in
   `docs/decisions/verse-pools.md`; add Vite alias
7. `MonitorView.tsx` — streak display (not "clean since"), weekly summary, histogram,
   top windows, live event list with false-positive flagging
8. `WarnOverlay.tsx` + main-process overlay launcher *(was missing)*
9. `PolicyView.tsx` — protection mode selector, threshold editor, accountability section
   using the invite-token model *(not direct email)*
10. `OnboardingView.tsx` *(was missing)*
11. `voice-adapter-electron.ts` + wire `packages/voice-gate` into `ipc-handlers.ts`;
    replace single `accountability:notify-override-attempt` with two-stage notifications
12. `OverrideGateView.tsx`
13. System tray icon and right-click menu

---

## 4. What this does NOT change

The following aspects of `PLAN.md` are correct and require no amendment:

- `daemon-ipc.ts` reconnect loop and NDJSON parsing
- `ipc-handlers.ts` policy threshold and protection mode handlers
- Atomic write pattern (`.tmp` + rename) for all JSON config files
- `stats-store.ts` score histogram and top-windows frequency map
- Tray icon colour priority table
- `OverrideGateView` retry-once behaviour
- `voice-adapter-electron.ts` IPC channel design (`voice:start`, `voice:stop`, `voice:result`)
