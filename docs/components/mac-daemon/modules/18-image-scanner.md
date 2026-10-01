# 18. `ImageScanner` — the ONNX classifier over UniFFI. **Done.**

Part of the [macOS Daemon plan](../plan.md).

**The first thing in this daemon that looks at a pixel.** `AccessibilityScanner` reads what
applications *declare*; this reads what is actually drawn. `packages/image-sandbox` is consumed over
a new `packages/image-sandbox-ffi` wrapper, exactly as `text-policy-ffi` is — so the geometry, the
reduction and the threshold all stay in Rust and Swift never re-derives any of them.

It exists here rather than in `machine-learning` or on Android because of the runtime split in
[learning-from-feedback.md](../../../decisions/learning-from-feedback.md): ONNX Runtime is the desktop
half, LiteRT is the Android half. `apps/mobile` must not be wired to this crate.

## What was built

- **`packages/image-sandbox/src/raw.rs`** — `image_from_raw(pixels, width, height, layout)`, reading
  a tightly packed 4-channel framebuffer as an RGB image. Alpha is dropped (the tensor is RGB and a
  screen framebuffer is opaque); a short buffer is refused rather than read past; a longer one is
  accepted with its tail ignored, matching `PixelBufferCopy.depad`'s own `>=` guard.
- **`ImageSandbox::check_raw`** — the same tile-max geometry, threshold and fail-open contract as
  `check`, with the decode step replaced. It returns a new `ScoredVerdict` rather than a bare
  `ImageVerdict`; see the score note below.
- **`packages/image-sandbox-ffi`** — `ImageGuard` (`withModel`/`disabled`/`classifyFrame`/
  `threshold`), `ImageOutcome`, `FramePixelLayout`. Construction is fallible; classification is
  not, because `image-sandbox` already fails open and making every caller reimplement that rule is
  how the rule gets got wrong.
- **`ImageScanner.swift`** — `ImageClassifying` (the seam) with `RealImageClassifier` and
  `FakeImageClassifier`, `ClassificationDispatching` with a background queue and an inline
  dispatcher, `ImageMapping.scanVerdict(for:)`, and the scanner itself.
- **`ScanLoop` now takes two scanners**, one per cadence. The single-scanner initializer is kept as
  a convenience, because the scheduling tests are about *when* a scan happens rather than which
  scanner runs.
- **The model ships inside the signed bundle** — `AppBundle.assemble` gained a `resources:`
  argument and `AppBundle.classifierModelName`; `scripts/bundle.sh` passes the artifact path.
- An **`image-scan <model.onnx> [size]`** verb, and `scripts/build-ffi.sh` rewritten as a loop over
  both FFI crates.

## Six things worth carrying

1. **Classification must not run on the calling thread.** `ScanLoop.tick` is driven by a `Timer` on
   the main run loop — the same thread AppKit draws the overlay on. Measured with the real model on
   this machine: **5.3 ms for one tile** (a square frame), 9.6 ms at 1024×1024, and a 1.54-aspect
   display frame is three tiles. Twenty milliseconds of inference on the main thread twice a second
   is a visible stutter in the interstitial the daemon is trying to put up. So `scan(_:)` dispatches
   the work to a serial background queue and returns the most recent *completed* verdict; the cost
   is one cadence of latency, well inside the debounce the loop already applies. The dispatcher is
   an injected protocol so tests stay synchronous and deterministic.
2. **A tick with work in flight repeats the last verdict, never `.allow`** — the rule
   `AccessibilityScanner` established, load-bearing for the same reason: the overlay is driven by
   what comes back here, and an `.allow` between classifications would tear the interstitial down
   and rebuild it over content that never stopped being blocked. The in-flight guard is an atomic
   test-and-set, so ticks cannot queue up behind an inference slower than the cadence.
3. **`Allow` carries an *optional* score, and the option is load-bearing.** `None` means nothing was
   classified at all — no model, unreadable buffer, below the size floor, inference fault.
   `Some(0.02)` means the model ran and saw nothing. Collapsing them would make a silently broken
   image path indistinguishable from a clean screen, which is exactly the failure the first live
   pass spent a session on with `SCStreamConfiguration.pixelFormat`. This is why `check_raw` returns
   `ScoredVerdict`: the network path has no long-running caller and keeps the plain verdict.
4. **Whatever threshold is configured, its provenance does not extend to screen frames.** It is
   calibrated on a corpus of images. A screen frame is a different distribution and nothing here
   covers it — recorded in
   [classifier-operating-point.md](../../../decisions/classifier-operating-point.md) rather than
   patched over. The daemon logs the score on every verdict so the margin is observable, and
   requires the threshold to be configured explicitly rather than shipping a default.
5. **There is no `warn` band on this path.** A probability has no warn range, and inventing one
   would be this daemon making up an operating point the measurement never established.
   `ProtectionMode` in `ScanLoop` is where a block legitimately becomes a warn.
6. **`regions` stays empty.** The tiled geometry knows which tile scored highest, but a 224-wide
   band of the frame is not a located object, and
   [content-classification.md](../../../architecture/content-classification.md) is explicit that an
   empty list on a non-allow verdict means "cover the whole surface". Populating it with tile bounds
   would claim a localization the model does not do.

## Two smaller findings

- **`Scanner` collides with `Foundation.Scanner`.** A test double written as
  `final class X: Scanner` subclasses Foundation's and compiles, then fails at every call site with
  "does not conform to expected type 'Scanner'". Qualify it as `MacDaemon.Scanner`.
- **Both FFI targets declare the same rpaths**, so the linker emits a `duplicate -rpath ... ignored`
  warning on every build. Harmless — the rpath is genuinely needed by each target independently, and
  neither can rely on the other being linked.

## Verified live

Real model, from Swift, with no TCC grant needed: `image-scan` loads
`data/models/baseline-v0.onnx`, reports the threshold as 0.465, hands over a 512×512 BGRA buffer and
gets `allow(score: 0.26607183)` back in 6.9 ms. The signed bundle passes
`codesign --verify --deep --strict` with **both** nested dylibs validated, and the bundled binary
runs the model **with the build tree's `.ffi/lib` moved away entirely** — proving the load is
bundle-relative. Appending one byte to the bundled model makes `codesign` report *"a sealed resource
is missing or invalid"*, so a swapped model invalidates the bundle rather than silently disabling
the image path.

**Not yet verified:** the classifier has never run against a real captured frame, because that needs
the agent under `launchd` with a Screen Recording grant and a human at a screen. Everything between
`SCStream` and `ImageGuard` is exercised, but the two ends have only met through a synthetic buffer.

## Reference documents

- ONNX Runtime Rust bindings (`ort`): <https://ort.pyke.io>
- Apple, `CVPixelBuffer` — `CVPixelBufferGetBytesPerRow`:
  <https://developer.apple.com/documentation/corevideo/1456964-cvpixelbuffergetbytesperrow>
- Apple, Embedding Nonstandard Code Structures in a Bundle:
  <https://developer.apple.com/documentation/xcode/embedding-nonstandard-code-structures-in-a-bundle>

---
