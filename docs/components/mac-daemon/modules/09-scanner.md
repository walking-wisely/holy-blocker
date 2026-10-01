# 9. `Scanner` and `ScanLoop` — the interface and the scheduler

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/Scanner.swift
Sources/MacDaemon/ScanLoop.swift
```

Mirrors the Windows daemon's `scanner` + `scan_loop` split, and keeps the same vocabulary so the two
daemons stay comparable:

```swift
enum ScanAction { case allow, warn, block }
enum ScanSource { case ocr, image, text }
enum ProtectionMode { case full, warn, off }

struct NormalizedRect {
    let x: Double, y: Double, width: Double, height: Double  // each 0.0–1.0, frame-relative
}

struct ImageDetection {
    let label:      String
    let confidence: Double        // 0.0–1.0
    let box:        NormalizedRect
}

struct ScanVerdict {
    let action:     ScanAction   // effective action, after applying ProtectionMode
    let rawAction:  ScanAction   // as returned by the scanner, before the mode downgrade
    let score:      Double       // 0.0–1.0
    let source:     ScanSource
    let regions:    [ImageDetection]  // located image detections, empty for text-sourced
                                       // verdicts and for image verdicts where nothing
                                       // localized — see content-classification.md's
                                       // "Image Localization" section. An empty list on a
                                       // non-allow verdict means "cover the whole surface",
                                       // never "nothing to do".
}

protocol Scanner { func scan(_ frame: CapturedFrame) -> ScanVerdict }

struct NullScanner: Scanner { /* always .allow, empty regions — the only implementation at first */ }
```

The loop is the scheduler bridging `EventHooks` callbacks and the scanner:

- **Debounce** — discard events arriving within the debounce window of the previous one; only the
  last event in a burst produces a capture.
- **Cadence split**, per [edge-daemons.md](../../../architecture/edge-daemons.md): the image classifier
  every ~500 ms while the surface is eligible; OCR every 1000–2000 ms, plus immediately after a
  foreground change or a meaningful frame difference. Text policy runs only when OCR returns
  meaningfully changed text.
- **Frame differencing** to avoid re-running OCR on an identical screen. Note this is *not* the
  same as trap 2 above: `SCStream` going idle tells you the screen did not change, which is exactly
  the cheap frame-difference signal the Windows daemon has to compute by hashing. Use it.
- **Skip** when there is no eligible surface, the display is asleep, or the session is locked.
- **Apply `ProtectionMode`** to produce `action` from `rawAction`, per
  [protection-modes.md](../../../decisions/protection-modes.md): in `warn` mode a block-range score is
  downgraded to a warn; in `off` mode no scan runs at all.

`ScanLoop` must be **pure with respect to time** — `tick(now:)` takes the clock as a parameter, as
the Windows version does, so the debounce and cadence rules are unit-testable with an injected
clock and no capture at all. This is the module with the most testable logic in Layer 2 and it needs
tests first.

## Where the mode→action decision should live — **decided: in Rust, over UniFFI**

The mapping from *(score, thresholds, mode)* to an action is policy, and this would be its **third**
independent implementation: `packages/text-policy` has it in Rust, the Windows daemon plans it in
C++, and this would add Swift. Three implementations of a thresholding rule is three chances for the
platforms to disagree about what "block" means.

`packages/text-policy-ffi` already exposes `PolicyEngine`/`evaluate` over UniFFI, and **UniFFI
generates Swift bindings natively** — Swift is a first-class target, unlike the Kotlin path that
needed the `apps/mobile/scripts/build-ffi.sh` scaffolding. So the cheap correct answer is to consume
the existing Rust policy from Swift rather than reimplement it, and this is the first platform where
that costs almost nothing.

**Resolved in module 16 and consumed by `AccessibilityScanner`** (session 4 below). The Rust engine
scores text and picks an `Action`; Swift does not re-derive it. Two things the decision did *not*
cover, both worth stating because they look like the same question and are not:

- **`ScanLoop.applyProtectionMode` stays plain Swift.** The `full`/`warn`/`off` downgrade is a
  product setting this daemon owns — the user's chosen protection level — not the scoring rule the
  Rust crate owns. Pushing it across the boundary would mean teaching `text-policy` about a mode it
  has no other reason to know.
- **The 5→3 action mapping is a real narrowing, and it lives here.** Rust has
  `Block`/`Blur`/`Warn`/`Log`/`Allow`; `ScanAction` has three. `Blur → .warn` in particular loses
  information, because this daemon has no partial-cover visual yet — see `PolicyMapping` in
  `AccessibilityScanner.swift`, which carries its own named test for exactly that reason.

The Windows daemon is unaffected by this and still has the choice in front of it; what changed is
that one platform has now paid the integration cost and found it small.

## Where image localization should live — open

`ScanVerdict.regions` is the interface the rest of Layer 2 (`Overlay`, `DaemonIPC`) is written
against, but nothing today populates it beyond an empty list. Per
[content-classification.md](../../../architecture/content-classification.md#image-localization), the
target is a real detector model; until `machine-learning` has one, the pragmatic first
implementation is the coarse-grid fallback described there, run inside the real `Scanner`
implementation that eventually replaces `NullScanner` — this module's job is only to make sure the
type carries the information once it exists, not to implement the detection itself.

---
