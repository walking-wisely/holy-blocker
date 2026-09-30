# macOS daemon review traps

Applies to `native-modules/mac-daemon`.

## TCC and identity

- **Never validate a TCC path from a shell.** A CLI launched from a terminal has its grant
  attributed to the terminal, so it appears to work, covers every other tool run from that shell,
  and evaporates under `launchd`. A verification run from a terminal is not a verification.
- TCC keys grants to a code signature. An ad-hoc `cdhash`-derived signature changes every build,
  so grants die on rebuild; a stable signing identity is a prerequisite, not a later step.
- A TCC entry added by hand through the Settings `+` button **need not match the running process**.
  Settings can read ON while the process's own `AXIsProcessTrusted()` returns false indefinitely,
  and toggling off/on does not repair it. Onboarding must request its own grants.
- There are **no usage-description `Info.plist` keys** for Screen Recording, Input Monitoring, or
  Accessibility on current macOS; `CFBundleName` carries the message. Verify against `tccd`'s
  strings, not documentation.
- `CGPreflightScreenCaptureAccess` returns a `Bool` and cannot distinguish denied from never-asked.
  Only `IOHIDCheckAccess` reports three states. Not-granted must be reported as the recoverable
  state, and the loss of information must be visible in the type.
- Permission loss is a **tamper event to report**, not an error to log.

## Capture

- `CVPixelBuffer` rows are padded (`bytesPerRow ≥ width * 4`); a naive copy shears the frame while
  still scoring as something.
- `SCStreamConfiguration.pixelFormat` must be set explicitly — the default is biplanar `420v`,
  whose `bytesPerRow` is the Y plane's stride, and every downstream check refuses the frame with a
  symptom identical to a missing grant.
- `SCDisplay.width`/`.height` are in **points**. Use `CGDisplayCopyDisplayMode(...).pixelWidth`.
- `SCStream` is change-driven; a static image starves it and `.idle` frames carry no surface. The
  last `.complete` frame must be retained — a still image is exactly what must be caught.
- DRM/HDCP content captures as black by design. That is a coverage limit, and it should be a
  callable signal rather than a silent zero.

## Accessibility text

- A first walk against a browser is legitimately thin; only a repeated walk sees page content.
  What builds Chrome's tree is being an AX client at all, not `AXManualAccessibility` (which Chrome
  rejects) or `AXEnhancedUserInterface` (not implemented).
- Only the frontmost window is walked. Content in any other window is never read — the direct
  mirror of the Android split-screen finding.
- Element text joined with a separator is not a boundary: `text-policy`'s normalisation collapses
  it, so a lexicon phrase can match across two unrelated elements. Evaluating elements separately
  trades a false-positive class for a false-negative one and needs a measurement, not a guess.
- Cycle detection cannot be replaced by the depth bound; a cycle still costs a full walk per branch.
  A real window can nearly saturate a 2000-node bound. An app can report zero windows while running.

## AppKit and the run loop

- A `Timer` scheduled after `NSApplication.run()`, or created off the main run loop, silently never
  fires — the same trap as an `AXObserver` with no run-loop source.
- `Timer.scheduledTimer`'s block is `@Sendable`; capturing plain non-`Sendable` library classes is
  a Swift 6 error. The fix is one `@MainActor` object to capture, not a restructured `async` split.
- Inference must not run on the main thread. A tick with work in flight repeats the **last verdict**,
  never `.allow` — returning allow between scans tears the overlay down and rebuilds it over content
  that never stopped being blocked.
- An overlay is a picture over windows the compositor still draws: clicking the desktop drops focus,
  and Mission Control renders live previews above `.screenSaver`. Covering is not suppressing.

## Bundling and signing

- `codesign -dvv` writes to **stderr**; read stdout only and every bundle looks unsigned.
- Assembly must delete the old bundle — a stale `_CodeSignature` makes `codesign` refuse.
- Nested dylibs and resources must be signed **before** the bundle seals them, or the failure
  appears at launch rather than at build. `--deep` hides this and Apple documents it as not to be
  used.
- cargo stamps `LC_ID_DYLIB` with an absolute path; without an `install_name_tool -id @rpath/...`
  rewrite the app loads the build-tree copy and works only on the build machine. That rewrite
  invalidates the signature — re-sign immediately.
- `DYLD_*` does not survive `swift test` (SIP strips it across Apple's signed test helper); declare
  the rpath on the FFI target.
- Run tests with `scripts/test.sh`, never bare `swift test`.
