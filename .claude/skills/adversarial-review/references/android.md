# Android review traps

Applies to `apps/mobile`.

## Verification discipline

- **Assert both directions.** A missing `INTERNET` permission produced a filter that refused
  blocked names and silently failed permitted ones — indistinguishable from working. Every guard
  test asserts a block *and* a pass, on positive output (a success line), never on the absence of
  an error string.
- Unit tests could not catch any of the three defects found on the first emulator run. Manifest
  permissions, service lifecycle, and platform restore behaviour need a device.
- Emulator-only verification is not universal: OEM skins (One UI, HyperOS) differ, and a mitigation
  for a behaviour that cannot be reproduced cannot be verified. Mark unverifiable rather than
  shipping an unobservable mitigation.

## Services and lifecycle

- Android does **not** re-establish a `VpnService` after reboot (`setAlwaysOnVpnPackage` is
  owner-only) — a `BootReceiver` must restore it.
- `ACTION_BOOT_COMPLETED` is also delivered when the app leaves the force-stopped state, so a boot
  receiver must never be what identifies a boot.
- A `MediaProjection` consent token is per-session and single-use; capture cannot be restored after
  a reboot, and pretending otherwise is worse than not trying.
- Start order matters: consent → `mediaProjection` FGS → `getMediaProjection`, and `registerCallback`
  must precede `createVirtualDisplay` on Android 14+.
- `onActivityResult` runs before `onResume`; state set in the wrong place makes a running service
  report as stopped.
- The system rebinds an enabled accessibility service by itself. A foreground service is a status
  surface and a still-alive witness, not a keep-alive.

## Guards and the tamper model

- Plain Device Admin only — never design around Device Owner.
- A settings list can harvest zero nodes for reasons unrelated to window selection: rows marked
  `accessibilityDataSensitive` need `android:isAccessibilityTool="true"`.
- Every watched window must be evaluated. `GLOBAL_ACTION_BACK` takes no window argument, so it is
  only valid when the matched window holds input focus — otherwise cover. An unfocused split-screen
  pane emits no accessibility events at all.
- Session classification must anchor on the last session boundary from a tail of entries; a
  single-line read turns every clean stop into an apparent kill once anything else writes between
  sessions.
- Screens whose identifiers cannot be dumped from a real device are not guarded — identifiers are
  dumped, never inferred. Where a screen is unguardable, the revocation is *recorded*, and the
  review should say whether recording is the intended answer or the available one.

## VPN and DNS path

- Once the VPN is up, our own TUN address is the system's advertised resolver — any "read the
  current DNS servers" path loops back into the TUN unless guarded on both the `NetworkRequest`
  (`NET_CAPABILITY_NOT_VPN`) and the returned address list.
- NXDOMAIN covers A, AAAA and HTTPS/SVCB in one refusal; a sink address does not.
- There is deliberately no fallback to a public resolver.
- The TUN claims a single `/32` and filters DNS only. See `dns.md` for what that does and does not
  cover, and for the unconnected-socket defect in `NetworkGuardService.ask()`.
- `protect()` the forwarding socket, and treat a refusal as a visible failure rather than a log line.

## Smoke-test traps

- netd answers from cache without asking the resolver, and `ndc resolver flushnetdns` fails silently
  for a non-root shell — reboot for a cold cache.
- `FrameGate` compares against the last frame *analysed*, not the previous frame, so a slow dissolve
  cannot walk past the change threshold.
