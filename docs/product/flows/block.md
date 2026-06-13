# Flow: Block

**Trigger:** a scanner returns a Block verdict (score ≥ block threshold) and the current
ProtectionMode is `full`.

In `warn` or `off` mode this flow does not execute — see
[protection-mode-change.md](protection-mode-change.md).

---

## Network path

### Phase 1 — net-shield (domain/IP known)

```
SNI or IP matched in radix tree as known-unholy
  → net-shield drops the packet
  → browser sees "connection refused"
  → no proxy involvement, no further phases
```

No event is emitted to the desktop because the proxy never saw the request.
*(Future: net-shield should push a lightweight drop-event over a local socket so the
desktop can count Phase-1 blocks separately.)*

### Phase 3 — proxy text gauntlet (URL or body scan)

```
scan_url or scan_body → ScanResult::Block { score }
  → proxy returns 403 Forbidden to browser
  → TCP connection to origin is not opened (URL block) or severed (body block)
  → proxy emits scan_event { verdict: "block", score, source: "text" } to desktop over IPC
```

The browser displays its own "403 Forbidden" or the proxy's minimal blocked-page body.

### Phase 4 — image sandbox

```
perceptual hash lookup → known-unholy
  OR ONNX inference → above confidence threshold
  → proxy discards image bytes
  → proxy serves a transparent 1×1 pixel to the browser
  → image slot renders blank; surrounding page content is unaffected
  → hash saved to local SQLite as known-unholy if discovered via ONNX
```

No IPC event is emitted per image block (too high volume); aggregate counts are tracked
in the stats store.

### Phase 5 — video watchdog

```
async ML frame check → flagged
  → proxy closes its socket to the browser for that stream
  → browser's media player freezes and throws a network error
  → no further segment requests are fulfilled for this stream URL
```

---

## Screen-capture path (daemon)

```
ScanLoop::Tick → ScanVerdict { action: Block, raw_action: Block, score, source }
  → daemon sends scan_event { action: "block", score, source, ts } over named pipe
  → (future) daemon signals Electron to spawn full-screen block overlay
```

Until the overlay is implemented the daemon emits the event and the Electron app
records it, but no visual interrupt is shown to the user.

---

## Desktop (all paths)

```
scan_event { verdict/action: "block" } received by daemon-ipc.ts
  → ring buffer appended (capped at 500 entries)
  → stats-store: lifetimeBlocked++, weeklyBlocked++, scoreHistogram updated,
                 topWindows frequency map updated
  → tray icon → red
  → MonitorView event row: red "block" badge, score, window title, timestamp
```

---

## Related

- [warn-interstitial.md](warn-interstitial.md) — what happens when score is in the warn band
- [protection-mode-change.md](protection-mode-change.md) — how mode affects whether this flow runs
- [../decisions/protection-modes.md](../../decisions/protection-modes.md) — why block only fires in Full mode
- [../network-pipeline.md](../../architecture/network-pipeline.md) — phase-by-phase network path detail
