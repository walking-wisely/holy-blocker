# Data classes — the router

The inventory `privacy-review` routes against. Match the changed paths against this table to decide
**whether** the privacy aspect applies and **which** class entries to load. Most changes match zero
or one row.

A change matching no row has no privacy surface. Say so and stop.

Every row answers the same three questions for one class of sensitive data **as it exists in this
repository today**:

- **Special-category (GDPR Art. 9)?** — whether the class reveals a "special category" (racial,
  health, sex life, and — by explicit extension — sexual-content classification about a specific
  person's screen). `?` means the standing genuinely depends on fact, not that the question was
  skipped. This is a data-inventory statement, not a compliance certification — see SKILL.md's
  scope discipline.
- **Retention** — the named constant or file that bounds it, or **unmeasured** as its own finding.
- **Off-device** — `none` only if a pruned search of the runtime trees shows no path; otherwise
  the named path.

| Data class | Where it lives | Art. 9? | Retention | Off-device |
|---|---|---|---|---|
| Captured screen frames — macOS | `native-modules/mac-daemon/Sources/MacDaemon/ScreenCapture.swift` — `CapturedFrame` (l.12), `FrameCache` (l.84, keeps only the last `.complete` frame), `SCShareableContentCapture` (l.156, the ScreenCaptureKit edge) | Y — a frame of a person's screen can contain anything, including sexual content | In-memory only. `FrameCache.lastComplete` holds exactly one frame until the next `.complete` delivery; the capture edge (`SCShareableContentCapture`) never writes a frame | none — no URLSession/NWConnection in mac-daemon sources; the only socket is the loopback health probe (`ProxyProcess.swift` `TCPListenerProbe`) |
| Captured screen frames — Android | `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/ScreenCaptureService.kt` (MediaProjection edge, `onFrame` l.259, `teardown` l.320); `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/FrameSink.kt` (`CapturedFrame` l.15, the `FrameSink` seam, `CountingFrameSink` l.47) | Y — same class of object as the macOS row | In-memory only. `IMAGE_BUFFER_COUNT = 2` (`ScreenCaptureService.kt:460`); `CountingFrameSink` counts and logs width/height, "never a pixel and never a hash" (`FrameSink.kt:45`); `planeBuffer` is cleared in `teardown` | none — no OkHttp/URLConnection/WebView in mobile sources (Kotlin sources contain only doc-reference URLs) |
| Extracted on-screen text (AX) — macOS | `native-modules/mac-daemon/Sources/MacDaemon/AccessibilityText.swift` — walk (`extract` l.195), real probe (`SystemAXProbe` l.289); `AccessibilityScanner.swift` (the scan tier over it) | Y — screen text of a person's screen, may contain anything | In-memory only. The `AXTextWalk.text` string lives for the scan it feeds; no write path (the only Scanner is `NullScanner`, which ignores the frame) | none — same absent network surface as the macOS frames row |
| Extracted on-screen text — Android accessibility | `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/policy/TextPolicy.kt` (`evaluate(text, source)` l.40); `NativeTextPolicy.kt` (UniFFI edge); evaluated over the `ACCESSIBILITY_TREE` events | Y | In-memory only. `PolicyVerdict` (l.34) is a value returned to the caller; no persistence in `policy/` (no `Log.*` calls in the policy package) | none |
| Extracted OCR text | **No OCR module exists anywhere in the repository as of this inventory.** The slot is reserved: `ScanLoop` runs an OCR cadence (`ScanLoop.swift` `ocrMinInterval`/`ocrMaxInterval`, l.19-21) and `ScanSource.ocr` / `PolicySource.OCR_*` name the source, but no `Vision`/`VNRecognizeTextRequest` code exists in mac-daemon, mobile, or `image-sandbox` (`rg Vision` over those trees returns nothing). The class is listed so an OCR module landing later lands *inside* the inventory, not ahead of it | Y — the whole point of this product's OCR is to classify what is on a specific person's screen | n/a — nothing stores it because nothing produces it yet (`image-sandbox`'s OCR pass is planned, `docs/components/mobile/plan.md`) | n/a |
| Classification scores and verdicts — macOS | `native-modules/mac-daemon/Sources/MacDaemon/Scanner.swift` (`ScanVerdict` l.65, `ImageDetection.confidence` l.54); delivered over the local IPC as `ScanEvent` (`DaemonIPC.swift`, outbound `scan_event`); `ScanLoop.lastVerdict` (l.142) | Y — a sexual-content classification score about a specific person's screen is special-category data (the SKILL.md scope note names this as the product's own finding) | In-memory only. `ScanLoop.lastVerdict` holds one verdict; the desktop app caches the last 500 scan events in a ring buffer (`apps/desktop/src/main/ipc-handlers.ts` `RING_BUFFER_CAP = 500`, l.20) and persists none | none — Unix-domain socket daemon↔Electron, then held in-process; no network call in desktop `src/` |
| Classification scores and verdicts — Android / `image-sandbox` | `packages/image-sandbox/src/classifier.rs` (`ClassifyResult.explicit_score` l.25, `softmax(logits)[1]`); `apps/mobile/.../policy/TextPolicy.kt` `PolicyVerdict` (l.34); `FrameGate.kt` (capture gating) | Y — an explicit-content probability about a specific person's screen | No store found. `explicit_score` is a `f32` returned up the call stack; nothing in `image-sandbox/src` writes a score to disk or a log; the mobile policy package has no logging | none |
| Tamper-log entries | `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/policy/TamperLog.kt` (pure: event codes, coalescing, classification) + `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/TamperLogStore.kt` (the `filesDir` edge, appended per event) | N — explicitly content-free: "It records the guard, never the screen" (`TamperLog.kt:185`); a test asserts the vocabulary carries no scanned text, verdict, score, or package | **Finite and stated**: `MAX_ENTRIES = 2_000` at ~60 B/entry, trimmed with a `LOG_TRIMMED` marker (`TamperLog.kt:227`, `TamperLogStore.kt:108`); ~<150 KB worst case, on `filesDir/tamper-log.tsv`, cleared by uninstall | none |
| Domains a device visited (implied by blocklist hits) | Filter: `packages/net-shield/src/blocklist.rs` (`BlocklistArtifact.lookup` l.189, `resolve_action` l.278, in-memory `LruCache` l.558); Android filter edge: `apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/NetworkGuardService.kt` (l.262-266, a blocked name is answered locally and logged as "answered a blocked name locally" — the name is deliberately never logged); the blocklist artifact *rule set* is signed and mmap'd (`blocklist.rs` `ARTIFACT_FILE`/`MANIFEST_FILE`) | N as stored — but *as visited*, a history of blocked domains is near-browsing-data | No hit record found. `LruCache` (bounded, in-memory) caches decisions; nothing persists a hit. **Unmeasured by deployment**: `mitm-proxy`'s `forward.rs:36` writes `tracing::debug!(method, host, port)` per forwarded request to stdout trace — a device-local, unbounded, deployment-dependent host record when debug tracing is on; flagged as a finding below | The proxy *forwards* the user's own traffic (its product function, user-enabled) — that is a transmission of web traffic itself, not of any recorded class. `domain-blocklist` fetchers pull the rule set on the build/VPS side, inbound, not user data |

## Findings the inventory itself records

- **`FrameSink` lives in Android, not mac-daemon.** The step-7 spec attributed `ScreenCapture`/
  `FrameSink` to mac-daemon; `apps/mobile/.../FrameSink.kt` is the only `FrameSink` in the repo, and
  mac-daemon's frame-holding type is `FrameCache` (`ScreenCapture.swift:84`). The rows above map the
  real files; the spec's parenthetical was wrong, not the class.
- **No OCR module exists.** The spec's "extracted AX/OCR text" is AX-only today; OCR is scheduled
  and typed but unimplemented. The slot is inventoried so a landing OCR module is governed from
  day one rather than discovered at audit time.
- **microproxy forwards with a debug host record.** `mitm-proxy`'s per-request `tracing::debug!` of
  `method`, `host`, `port` (`forward.rs:36`) is a shallow visited-domain record when debug tracing
  is on; it is stdout trace, not a bounded file, so its retention is **unmeasured by deployment**.
- **Score-storage claim is asserted-absent, not observed-absent.** "No store found" above is a
  pruned search over current sources; the ring buffer binding verdicts in `ipc-handlers.ts` was
  observed, `image-sandbox`'s score sink was not (nothing consumes `explicit_score` yet).

## Cadence

Per PR, this router is the whole gate: match, apply the checks for the matched classes, done. The
heavier passes below are scheduled, not per-change.

| When | Pass |
|---|---|
| Every full-loop step | Authoring mode, unconditionally (wired in step-loop gate) |
| Per package, once | Audit mode — produce the baseline in `docs/engineering/privacy-backlog.md` |
| Monthly pre-launch | Re-read the class list for new flows the router does not yet cover |
| Every release | Backlog triage: nothing `open` without an explicit accept |