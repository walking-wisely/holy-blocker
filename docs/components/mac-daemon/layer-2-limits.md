# What Layer 2 does *not* cover on macOS — honest limits

Part of the [macOS Daemon plan](plan.md).

As with Layer 1, these are inherent to the mechanism and are recorded here rather than discovered
later:

- **DRM-protected video captures as black.** HDCP-protected surfaces (Netflix, Apple TV+, and
  others) are excluded from `ScreenCaptureKit` by design. Streaming services in that category are
  invisible to the image model.
- **Exclusive-display applications cannot be covered.** Where a game or app takes exclusive control
  of the display, no window composites above it. Module 13 is the only lever, and it does not always
  apply.
- **Screen Recording is permanently revocable.** Apple reserves this from MDM specifically so it
  always remains user-consentable, which also means it always remains user-*revocable* by anyone who
  can authenticate as an admin. The mitigation is detection and reporting (module 7), never
  prevention.
- **Nothing is captured while the session is locked or the display is asleep** — correct behaviour,
  but it means the daemon's coverage is not continuous and the event log will have gaps that are not
  evidence of anything.
- **The protected user being a local admin defeats the whole model**, Layer 1 and Layer 2 together.
  See module 7; this is a setup requirement, not something code can fix.
- **The capture path sees everything** — banking sessions, password managers, private messages. This
  is the single largest privacy surface in the project. Frames stay in memory, are never persisted,
  never logged, and never leave the device; the only frame that crosses a process boundary is the
  warn-interstitial backdrop over the local socket. Any future change here needs an explicit
  decision, not an implementation detail.
