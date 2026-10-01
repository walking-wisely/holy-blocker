# 15. `SettingsGuard` — notice the settings pane, and record it

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/SettingsGuard.swift
```

The macOS counterpart to `apps/mobile`'s `SettingsGuard`, with a deliberately smaller remit. Detects
that System Settings has come forward, records it, and optionally shows the interstitial.

**It costs no permissions.** Verified on macOS 26.5.2 with both grants off:
`NSWorkspace.shared.frontmostApplication` returns bundle identifier, name and PID, so watching for
`com.apple.systempreferences` via `NSWorkspace.didActivateApplicationNotification` needs no TCC at
all. Detecting the *pane* is what costs a grant — of 21 on-screen windows,
`CGWindowListCopyWindowInfo` gave owner names and bounds for all and a non-empty `kCGWindowName` for
**none**, because titles are redacted without Screen Recording.

The general rule this module exists to respect: **capture classifies what is inside a window;
identifying which window is metadata and is cheap.** Never run OCR to learn something a bundle
identifier already answers.

Response options, in order of preference:

1. **Record to the tamper log** and, in partnered mode, notify. This is the primary behaviour.
2. **Show the interstitial** (module 10) — a verse and a pause, which is the product's actual
   mechanism.
3. Cover or `forceTerminate()` — available without TCC, but **rank it as friction, not a lock**, and
   prefer not to. A process that force-quits System Settings behaves like malware, races the user,
   and is a strong candidate for being the reason the product gets uninstalled.

**Do not model this on the Android version's importance.** There, the accessibility screen is the
only door a normal user has, which is what makes `GLOBAL_ACTION_BACK` load-bearing. On macOS the
user has a terminal, so `tccutil`, `launchctl`, `kill` and Recovery all remain open — and
`PermissionGate`'s revocation polling detects the *outcome* of all of them, including this one. This
module is defence in depth on top of that, never a substitute for it.

---
