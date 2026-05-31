# Architecture

Holy Blocker is a local-first content blocking system. The client should make blocking decisions on-device by inspecting the visible runtime surface instead of relying only on domain-level blocks.

The core runtime path is:

```text
OS event or scan tick
  -> identify active visible surface
  -> capture pixels
  -> image classifier
  -> OCR
  -> text policy classifier
  -> local decision
  -> block, blur, warn, log, or allow
```

The system is intentionally split into platform-specific daemons and shared policy/model components:

```text
apps/
  desktop/                 Electron control panel

native-modules/
  win-daemon/              Windows foreground hooks and capture loop
  android-service/         Android AccessibilityService and capture/control integration

machine-learning/          Training and export pipeline for local image models

packages/                  Future shared runtime packages
  text-policy/             Proposed Rust text classification and rule engine
```

## Design Principles

- Blocking should work without sending screenshots, OCR text, or browsing context to a server.
- Daemons should use OS events to wake quickly, but correctness should not depend only on events.
- Image and text decisions should be explainable enough to debug false positives and false negatives.
- Shared policy logic should be implemented once and exposed to each app through native bindings.
- Sensitive rule packs and eval corpora should be treated as private data assets, not ordinary public source files.

## Runtime Components

### Edge Daemons

Edge daemons are responsible for observing the current user-visible surface, scheduling scans, running local inference, and applying the selected local action.

The Windows daemon should be a native process because it needs reliable access to Win32 events, foreground windows, screen capture APIs, and eventually ONNX Runtime.

The Android daemon should use platform accessibility and capture capabilities appropriate to Android's permission model.

### Image Classifier

The image classifier handles visual content that does not appear as text or that OCR cannot interpret reliably. The first Windows target format is ONNX. The first Android target format is TFLite.

### OCR Provider

OCR extracts text from captured screenshots. The OCR implementation should be behind a provider interface so the project can start with platform-native OCR where convenient and later add broader fallback engines.

### Text Policy Engine

The text policy engine decides whether extracted text should be blocked, warned, logged, or allowed. It should combine deterministic rules with optional model-based classification later.

