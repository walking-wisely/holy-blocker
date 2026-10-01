# 6. `MediaProjection` — **capture done**, image path still to come

Part of the [Mobile plan](../plan.md).

`ScreenCaptureService.kt` over `policy/ScreenCapture.kt` (pure) and `policy/FrameGate.kt` (pure):
a `mediaProjection`-typed foreground service that mirrors the display into a small `ImageReader`
surface, reduces each frame to a 64-bit hash, and hands the few that survive the gate to a
`FrameSink`.

**The sink is empty on purpose.** The classifier is `packages/image-sandbox`, which landed on
master while this was being built (decode → preprocess → ONNX → verdict, wired into
`mitm-proxy`); nothing on the Android side reaches it yet, so frames go to `CountingFrameSink`
and are dropped. What the sink should call is **not** `packages/image-sandbox` — see §9, which is the rest of
this step. Until that exists, `CountingFrameSink` is the honest state: the capture path runs,
and nothing claims to be classifying. The capture half
was built first because it is the half that cannot be got right without a device — everything
below was measured on an android-36 arm64 emulator, and three of the five items were wrong in
the first version.

Consent is **per session** — there is no persistent grant to cache, it cannot run silently, and
the token is single-use from Android 14. That has consequences the UI has to state rather than
paper over: capture does not survive a reboot, there is nothing for `BootReceiver` to restore
(unlike the VPN), and `ScreenCaptureService.isRunning` is a process-lifetime flag rather than
something read back from the system.

**The start order is strict on modern Android and easy to get wrong.** The foreground service
must be running before `getMediaProjection()`, but *after* the user has consented — it is not
started first:

1. `MediaProjectionManager.createScreenCaptureIntent()` → launch it.
2. Handle the activity result; keep `resultCode` + `data`. **The VPN's habit of ignoring the
   result does not transfer** — `VpnService.prepare` can be re-asked and is the authority on its
   own grant, but here the result `Intent` *is* the grant.
3. **Only now** start the `mediaProjection`-typed foreground service and call `startForeground()`.
4. From inside that service, `getMediaProjection(resultCode, data)`.

`registerCallback` must come **before** `createVirtualDisplay` on Android 14+, or the projection
throws. It is also how a user-stopped projection is noticed, which is not optional: the system's
status-bar chip ends a projection in one tap and there is no permission that prevents it —
recorded as `SCREEN_CAPTURE_REVOKED`, the same ceiling as every other bypass in §7.

## The gate is a durability feature, not an optimisation

`MediaProjection` delivers a frame per composition — 60 to 120 a second while anything moves —
and the path behind it will be a downscale, an OCR pass and a model. `FrameGate` applies a hard
one-per-second rate cap and a change threshold over the frame's difference hash, and **compares
against the last frame actually analysed rather than the previous frame**, so a slow dissolve
cannot walk past it a few bits at a time. Measured on the emulator: a session left on a static
screen for ~45 seconds analysed **4 frames**.

The hash is a *gradient* comparison (`dHash`), not an absolute one, so auto-brightness and a
dark-theme animation do not read as a new screen. The reduction also throws away everything but
72 luma samples, which is the right property for the one value that outlives the frame.

## What could not be verified from adb

Two teardown paths are exercised by code review only, and the reasons are worth recording so
nobody re-tries them:

- **The disarm-driven stop.** Flipping the protection mode from a script needs the app process
  down (`SharedPreferences` caches in memory), and a force-stop kills the projection first — so
  the graceful path can never be the one observed.
- **Revoking the appop mid-session.** `appops set PROJECT_MEDIA deny` does **not** stop a live
  projection; the op is checked when the grant is issued. Measured, not assumed.
- `am stopservice` is refused for a non-exported service, so there is no third way in either.

The revoke path *is* verified, through the system's own stop-sharing chip — see
`scripts/smoke-test-capture.sh`, which drives it.

## Reference documents

- [`MediaProjection`](https://developer.android.com/reference/android/media/projection/MediaProjection) — `registerCallback`, and its ordering requirement against `createVirtualDisplay`
- [`MediaProjectionManager`](https://developer.android.com/reference/android/media/projection/MediaProjectionManager)
- [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) — the `mediaProjection` type, its start-order requirement, and the `FOREGROUND_SERVICE_MEDIA_PROJECTION` permission
- [`ImageReader`](https://developer.android.com/reference/android/media/ImageReader) and [`Image.Plane`](https://developer.android.com/reference/android/media/Image.Plane) — `rowStride`/`pixelStride`, which is why nothing assumes a tightly packed buffer
- [`DisplayManager#VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR`](https://developer.android.com/reference/android/hardware/display/DisplayManager#VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR)
- [ITU-R BT.601-7](https://www.itu.int/rec/R-REC-BT.601) §2.5.1 — the luma coefficients in `ScreenCapture.lumaGrid`
