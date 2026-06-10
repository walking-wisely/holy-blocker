# Flow: Warn Interstitial

**Trigger:** a scanner returns a Warn verdict (score in `[warn_threshold, block_threshold)`)
and the current ProtectionMode is `full` or `warn`.

In `off` mode no scanning runs and this flow does not execute.

The interstitial is the primary user-visible signal that the system detected something.
It is designed to interrupt without condemning — it shows a relevant verse and gives the
user a deliberate choice rather than silently blocking them.

---

## Current implementation: proxy (web content)

The proxy owns the HTML before the browser sees it, so an overlay can be injected directly
into the response body. No native drawing or window tracking is required.

```
scan_url or scan_body → ScanResult::Warn { score, dominant_category }
  → proxy appends injected block to HTML <body> before forwarding
  → origin bytes reach the browser with the overlay baked in
  → page loads normally behind the overlay
```

### What is injected

A `<style>` + `<div>` block appended to `</body>`:

```
position: fixed; inset: 0; z-index: 2147483647
backdrop-filter: blur(24px)           ← blurs everything underneath
background: rgba(0,0,0,0.55)
```

Content of the overlay card:

- Verse text chosen by `dominant_category` (see
  [../decisions/verse-selection.md](../../decisions/verse-selection.md))
- **"I understand, continue"** button — sets a `__hb_suppress` session cookie for the
  current host; suppresses re-warn for 10 minutes on the same hostname. Removes the
  overlay via JS without reloading the page.
- **"Go back"** button — calls `window.history.back()` via injected script.

The injected overlay does not load any external resources. All styles and the verse text
are inlined at injection time by the proxy.

### Session suppression

The proxy checks for `__hb_suppress=<host>` in the request's `Cookie` header before
running the body scan. If the cookie is present and unexpired the scan result is
downgraded to Allow for that request, and the overlay is not injected.

Suppression is intentionally short (10 minutes) and host-scoped, not domain-wide. It
exists to prevent the overlay from re-triggering on every navigation within a session
the user already chose to continue through.

### IPC event

```
proxy emits scan_event { verdict: "warn", score, source: "text", host } to desktop
```

---

## Future implementation: native windows (daemon)

When the daemon detects a warn-level verdict via OCR or image classification, the proxy
is not involved. The full-screen interstitial approach is used instead of a pinned overlay
because native windows cannot be injected into.

```
ScanLoop::Tick → ScanVerdict { action: Warn, raw_action: Warn, score, source }
  → daemon sends scan_event { action: "warn", score, source, captured_frame_b64, ts }
    over named pipe to Electron
  → Electron main process creates frameless BrowserWindow:
      alwaysOnTop: true
      transparent: true
      bounds: full primary screen
  → BrowserWindow renders WarnOverlay component:
      backdrop: <img src={captured_frame_b64}> with CSS blur(20px)
      card: verse + "I understand" / "Close" buttons
  → "I understand": closes the BrowserWindow, records suppression in memory
    for this window title for 5 minutes
  → "Close": same as "I understand" (user dismissed deliberately)
```

The captured frame comes from the daemon's existing `CaptureWindow()` output, forwarded
in the IPC message. It is used only as a blurred backdrop — it is never stored, logged,
or sent anywhere.

---

## Desktop (all paths)

```
scan_event { verdict/action: "warn" } received by daemon-ipc.ts
  → ring buffer appended
  → stats-store: lifetimeWarned++, weeklyWarned++, scoreHistogram updated,
                 topWindows frequency map updated
  → tray icon → amber (if not already red)
  → MonitorView event row: amber "warn" badge, score, window title / host, timestamp
  → user can flag the event as false positive from MonitorView
```

---

## Warn mode vs Full mode

| | Full mode | Warn mode |
|---|---|---|
| Score ≥ block threshold | Block flow runs | Overlay shown, traffic passes |
| Score in warn band | Overlay shown, traffic passes | Overlay shown, traffic passes |

In `warn` mode the overlay is shown even for Block-range scores. The content still loads.
This mode is intended for initial calibration, not permanent use.

---

## Related

- [block.md](block.md) — what happens when score crosses the block threshold
- [protection-mode-change.md](protection-mode-change.md) — how mode is set and propagated
- [../decisions/verse-selection.md](../../decisions/verse-selection.md) — how verses are chosen
- [../decisions/protection-modes.md](../../decisions/protection-modes.md) — why warn passes through
- `packages/mitm-proxy/src/scan.rs` — proxy injection implementation (step 8 in proxy PLAN)
- `apps/desktop/src/renderer/src/views/WarnOverlay.tsx` — native overlay component (future)
