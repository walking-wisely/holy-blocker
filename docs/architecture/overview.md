# Architecture

Holy Blocker is a local-first content blocking system. The client should make blocking decisions on-device by inspecting the visible runtime surface instead of relying only on domain-level blocks.

Two complementary runtime paths work together to provide defense in depth:

**Screen-capture path** — inspects what is already rendered on the user's screen:

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

**Network path** — intercepts web traffic before content reaches the browser:

```text
outbound TCP/UDP
  -> network shield (packet filter + SNI/IP radix tree)
  -> MITM proxy (TLS termination)
  -> text gauntlet (metadata + body scan)
  -> image sandbox (perceptual hash lookup + ONNX fallback)
  -> video watchdog (async stream sampler)
```

The network path stops most bad content before it renders. The screen-capture path acts as a backstop for cached pages, native apps, and any content that arrives outside the proxy. See [Network Pipeline](network-pipeline.md) for the full phase-by-phase description.

The system is split into platform-specific daemons, shared Rust packages, and native network adapter modules. Items marked `[planned]` do not exist in the repo yet.

```text
apps/
  desktop/                 Electron control panel
  mobile/                  Android AccessibilityService text guard and overlay

native-modules/
  win-daemon/              Windows foreground hooks and capture loop
  win-network/             [planned] Wintun driver binding and Windows routing policy
  mac-network/             [planned] macOS NetworkExtension NEPacketTunnelProvider

packages/
  text-policy/             Rust text classification and rule engine
  mitm-proxy/              TLS termination, HTTP/HTTPS proxying, phase routing
  net-shield/              [planned] TUN packet reader, SNI extractor, radix-tree domain/IP filter
  image-sandbox/           [planned] Perceptual hashing, SQLite hash lookup, ONNX inference
  video-watchdog/          [planned] Async segment sampler, frame extraction, ML gate

machine-learning/          Training and export pipeline for local image models
  models/
    image-v1/              Screen-capture classifier
    web-image-v1/          Web image classifier for the network pipeline

data/                      (gitignored — private runtime assets)
  ca/                      Local root CA key and certificate
  hash-db/                 SQLite perceptual hash database
  models/                  Quantized ONNX and TFLite model files
```

## Design Principles

- Blocking should work without sending screenshots, OCR text, or browsing context to a server.
- Daemons should use OS events to wake quickly, but correctness should not depend only on events.
- Image and text decisions should be explainable enough to debug false positives and false negatives.
- Shared policy logic should be implemented once and exposed to each app through native bindings.
- Sensitive rule packs and eval corpora should be treated as private data assets, not ordinary public source files.

## Runtime Components

### Network Pipeline

The network pipeline operates below the browser, intercepting TCP streams via a virtual TUN adapter. It filters by domain and IP at the packet level (Phase 1), decrypts HTTPS via a locally-trusted MITM proxy (Phase 2), and then runs text, image, and video analysis on the plaintext content (Phases 3–5). See [Network Pipeline](network-pipeline.md) for the complete specification.

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

