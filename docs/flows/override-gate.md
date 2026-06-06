# Flow: Override Gate

**Trigger:** user attempts to set protection mode to `"off"`, or invokes
"Disable protection…" from the tray menu.

The gate exists to add deliberate friction to disabling protection. It is not a security
system — a determined user can kill the process. Its purpose is to create a moment of
pause and make impulsive disabling harder.

---

## Steps

### 1. Gate initiated

```
protection:set-mode("off") received in main process
  OR tray menu "Disable protection…" clicked
  → main process emits override:gate-started on internal bus
  → renderer is instructed to show OverrideGateView as a full-screen modal
```

OverrideGateView is not dismissable by clicking outside or pressing Escape. The only
exits are completing the gate, failing the gate (with no retries remaining), or killing
the application.

### 2. Accountability pre-notification

```
if accountability partner email is configured:
  → send notification immediately, before the gate outcome is known:
    "An override attempt was started on <device> at <timestamp>"
  → a second notification is sent on completion (pass or fail)
```

Notification is sent regardless of whether the gate passes. This ensures the partner
always knows an attempt was made, even if the user completes it successfully.

### 3. Verse selection

```
VoiceGate.currentVerse()
  → selects a verse from the override verse pool
    (distinct pool from the warn-interstitial verse pool — longer, more reflective passages)
  → verse is displayed in OverrideGateView with a "Start reading" button
  → no timer shown; the user should not be able to time themselves
```

### 4. Voice recording

```
user clicks "Start reading"
  → renderer activates MediaRecorder + Web Speech API
  → animated indicator shown (no countdown)
  → user reads the verse aloud
  → Web Speech API produces running transcript
  → when user stops speaking (or clicks "Done"):
    → renderer sends voice:result { audioBuffer, transcript, durationMs } to main via IPC
```

### 5. Gate evaluation (main process)

```
main process runs three checks against voice:result:

  Transcript match:
    transcript similarity to verse text >= threshold (fuzzy, not exact)
    → fail: "Please read the full verse"

  Rate check:
    durationMs / verse_word_count within expected reading-speed range
    too fast → fail: "Please read at a natural pace"
    too slow → pass (slow is fine; rushing is the red flag)

  Liveness check:
    audioBuffer is not silent (RMS above noise floor)
    → fail: "No audio detected"

All three must pass.
```

### 6. Outcome

**On failure:**

```
  → OverrideGateView shows which check failed
  → one retry offered with a fresh verse (new VoiceGate.currentVerse() call)
  → on second failure: gate closes, protection mode is not changed
  → accountability notification: "Override attempt failed on <device> at <timestamp>"
```

**On success:**

```
  → OverrideGateView closes
  → protection:set-mode("off") proceeds (policy.json written, daemon + proxy notified)
  → tray icon → grey
  → accountability notification: "Protection was disabled on <device> at <timestamp>"
  → override event recorded in stats-store
```

### 7. Re-enabling

```
user clicks "Re-enable protection" in tray menu (visible when mode == "off")
  OR sets mode back to "full" or "warn" in PolicyView
  → no gate required
  → protection:set-mode("full") proceeds immediately
  → tray icon → green
```

---

## What the gate does not protect against

- The user killing the Electron process or daemon directly.
- The user disabling the TUN adapter manually.
- The user uninstalling the app.

The gate is a pause mechanism and accountability signal, not a tamper-proof lock.
Tamper resistance (if ever needed) would require a separate privileged watchdog process
outside the scope of this flow.

---

## Related

- [protection-mode-change.md](protection-mode-change.md) — full mode-change flow that invokes the gate
- [../decisions/protection-modes.md](../decisions/protection-modes.md) — why Off requires a gate but Full/Warn do not
- `packages/voice-gate/` — verse pool, transcript matching, rate check, liveness check
- `apps/desktop/src/renderer/src/views/OverrideGateView.tsx` — the modal UI
- `apps/desktop/src/main/voice-adapter-electron.ts` — IPC bridge for audio/transcript
