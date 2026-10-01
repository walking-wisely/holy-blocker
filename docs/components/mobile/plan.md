# Mobile (Android) — Implementation Plan

The design rationale lives in [content-interception.md](../../decisions/content-interception.md)
under "Android — the layer order inverts". This document is the build plan for
`apps/mobile/`: what modules to add, in what order, and what each one is responsible for.

## Why Android inverts the desktop order

On desktop, Layer 1 (the MITM proxy) does the heavy lifting and Layer 2 (capture + render
path) supplements it. On Android that ordering flips, and the reason is not a preference:

- **There is no general MITM on stock Android.** Apps targeting API 24+ trust only the
  *system* certificate store. A user-installed CA lands in the user store, which Chrome and
  almost every other app ignore. Installing into the system store needs root or a custom ROM.
  So `scan_body` — the desktop content proxy — has no Android equivalent.
- **What survives at the network layer** is only what is visible without decryption: DNS
  filtering, SNI inspection (eroded over time by Encrypted Client Hello), and IP/port
  blocking. That is net-shield's Phase 1, not the content proxy.
- **Therefore Layer 2 carries the product.** `AccessibilityService` reads on-screen text
  directly from other apps — a non-OCR shortcut for the whole text path — and an overlay
  covers what the policy rejects.

The MVP builds that Layer 2 text path, and nothing else.

## Current state

`apps/mobile/` is a standalone Gradle build (the pnpm workspace does not manage it).

- Gradle wrapper pinned to 8.14.3, AGP 8.13.0, Kotlin 2.2.20, compileSdk 36, minSdk 26 — **Done.**
- `policy/TextPolicy.kt` — domain types (`PolicyAction`, `PolicySource`, `PolicyVerdict`) — **Done.**
- `policy/TextAssembler.kt` — node-tree fragments → one capped string — **Done.**
- `policy/ScanGate.kt` — event filtering, dedupe, debounce, verdict → `CoverState` — **Done.**
- `policy/AccessibilityServiceStatus.kt` — parses `ENABLED_ACCESSIBILITY_SERVICES` — **Done.**
- `policy/NativeTextPolicy.kt` — UniFFI adapter onto `text-policy` — **Done.**
- `ScreenGuardService.kt` — `AccessibilityService` glue; harvests text, applies cover — **Done.**
- `OverlayController.kt` — `TYPE_ACCESSIBILITY_OVERLAY` cover — **Done.**
- `MainActivity.kt` — onboarding, Restricted Settings hint — **Done.**
- `scripts/build-ffi.sh` — Kotlin bindings + per-ABI `.so` — **Done.**
- `scripts/smoke-test.sh` — end-to-end device check — **Done** (passes on android-36 arm64).
- `policy/SettingsGuard.kt` — blocks the screens that would remove the guard — **Done** for the
  AOSP profile, verified on an android-36 arm64 emulator, device-admin identifiers included —
  the activation prompt, and the list, which needed `isAccessibilityTool` before it could be seen
  at all. Xiaomi and Samsung have no profile at all. Evaluates every watched window, not one
  screen, so split screen is covered; the back action is gated on input focus, since
  `GLOBAL_ACTION_BACK` takes no window argument.
- `policy/RescanSchedule.kt` — when to take a second look at a screen that did not match —
  **Done.**
- `policy/ProtectionSchedule.kt` — the protection mode: armed, disarm cooldown, confirm, window;
  monotonic throughout — **Done.**
- `ProtectionStore.kt` — storage edge for the mode — **Done.**
- `policy/TamperLog.kt` — the append-only record: entry format, tolerant parse, coalescing, trim,
  session classification — **Done.**
- `TamperLogStore.kt` — the `filesDir` edge for it — **Done**, verified on an android-36 arm64
  emulator.
- `policy/GuardStatus.kt` — health, the notification message, and when the status service is worth
  running — **Done.**
- `GuardStatusService.kt` — the foreground service: status notification, and the still-alive
  process that records a disable the guard cannot report — **Done**, verified on an android-36
  arm64 emulator.
- `BootReceiver.kt` — restores the status service after a restart or an update, and writes nothing
  — **Done.**
- `policy/NetworkGuard.kt` — the TUN's shape, when the VPN should be up, and which resolvers a
  permitted query is forwarded to — **Done.**
- `NetworkGuardService.kt` — the `VpnService`: DNS filtering over a single-route TUN — **Done**,
  verified on an android-36 arm64 emulator by `scripts/smoke-test-vpn.sh`. SNI/IP filtering is
  still to come, and needs a userspace TCP stack — see step 11 below.
- `policy/ScreenCapture.kt` — the capture surface's size, the luma reduction, the frame hash, and
  when capture should be running — **Done.**
- `policy/FrameGate.kt` — which captured frames are worth analysing: a rate cap and a change
  threshold, both measured against the last frame actually analysed — **Done.**
- `FrameSink.kt` — the seam the image path plugs into, with a counting stand-in — **Done**; the
  analysis behind it waits on `packages/image-sandbox`.
- `ScreenCaptureService.kt` — the `MediaProjection` edge: consent order, virtual display,
  `ImageReader` frames — **Done**, verified on an android-36 arm64 emulator by
  `scripts/smoke-test-capture.sh`. It captures and gates; it does not yet classify.
- The image path — the backbone on LiteRT and the head crate the frames are for — **not started**;
  see §9 and [classifier-head/plan.md](../classifier-head/plan.md). It is deliberately not
  `packages/image-sandbox`, which is the ONNX half of a per-platform split.
- `admin/HolyBlockerAdminReceiver.kt` — device admin, so uninstall is refused until it is
  deactivated — **Done**, verified on an android-36 arm64 emulator.
- Tamper log — **Done**, see step 9 below.

Known bypasses that remain open are tracked in **[backlog.md](backlog.md)**, ranked by how
little effort they take. Read it before extending the guard — several plausible-looking
additions there were checked and found not to be holes.

## The architectural rule this module follows

**Everything decidable is pure Kotlin with its own domain types; the platform is glue.**

`ScanGate`, `TextAssembler`, and `AccessibilityServiceStatus` have no Android imports and no
UniFFI imports, so they run under plain JUnit on the JVM — no emulator, no `.so`. The two
places that touch the outside world (`ScreenGuardService`, `OverlayController`) hold no
decisions worth testing.

This is why the app defines `PolicyAction`/`PolicySource` rather than reusing the
UniFFI-generated enums: the generated file initialises JNA and loads the native library on
class init, which would drag the Rust build into every unit test. `NativeTextPolicy` is the
single mapping point.

## Modules to add

### 1. `policy` — the decision core — **Done**

Specification: [modules/01-policy.md](modules/01-policy.md).

### 2. `ScreenGuardService` — the AccessibilityService — **Done**

Specification: [modules/02-screen-guard-service.md](modules/02-screen-guard-service.md).

### 3. `OverlayController` — the cover — **Done**

Specification: [modules/03-overlay-controller.md](modules/03-overlay-controller.md).

### 4. `MainActivity` — onboarding — **Done**

Specification: [modules/04-main-activity.md](modules/04-main-activity.md).

### 5. `VpnService` — DNS filter — **Done**; SNI/IP still to come

Specification: [modules/05-vpn-service.md](modules/05-vpn-service.md).

### 6. `MediaProjection` — **capture done**, image path still to come

Specification: [modules/06-media-projection.md](modules/06-media-projection.md).

### 7. Tamper resistance — partially built

Specification: [modules/07-tamper-resistance.md](modules/07-tamper-resistance.md).

### 8. `GuardStatusService` — the foreground status surface — **Done**

Specification: [modules/08-guard-status-service.md](modules/08-guard-status-service.md).

### 9. The image path — backbone on LiteRT + the head crate — not yet created

Specification: [modules/09-the-image-path.md](modules/09-the-image-path.md).

## The FFI dependency

`packages/text-policy-ffi` is a UniFFI wrapper over `text-policy`, added for this module. It
produces two things with different prerequisites:

| Output | Needs | Built by |
|---|---|---|
| Kotlin bindings (`app/src/generated/kotlin`) | cargo only | `scripts/build-ffi.sh` |
| `libtext_policy_ffi.so` per ABI (`app/src/main/jniLibs`) | NDK + cargo-ndk | `scripts/build-ffi.sh` |

The bindings are generated from the *host* cdylib — they are platform independent, so binding
generation does not need the NDK. Only the `.so` does. `scripts/build-ffi.sh` degrades
gracefully: without cargo-ndk it refreshes bindings, skips the `.so`, and says so.

**Both outputs are gitignored, so `scripts/build-ffi.sh` is a required first step on a fresh
clone.** They are build output of `packages/text-policy-ffi`; the Rust source is the single
definition of this surface, and a checked-in copy could only ever drift from it. A Gradle
pre-build check fails with that instruction rather than letting the compiler report an
unresolved reference in `NativeTextPolicy.kt`.

Rerun the script whenever the FFI surface changes. The bindings carry a checksum of the Rust
scaffolding and will fail at load time if they fall out of sync with the `.so`.

## Implementation order

1. ~~Policy core (`TextPolicy`, `TextAssembler`, `ScanGate`) with JVM unit tests.~~ **Done.**
   <!-- step: mobile.policy-core -->
2. ~~`text-policy-ffi` UniFFI crate + `NativeTextPolicy` adapter.~~ **Done.**
   <!-- step: mobile.text-policy-ffi -->
3. ~~`ScreenGuardService` + `OverlayController` + onboarding.~~ **Done.**
   <!-- step: mobile.screenguard-overlay-onboarding -->
4. ~~Build the `.so` (NDK) and validate on a device — the first end-to-end run.~~ **Done** —
   `scripts/build-ffi.sh` builds all three ABIs; `scripts/smoke-test.sh` passes on an
   android-36 arm64 emulator.
   <!-- step: mobile.ffi-device-e2e -->
5. ~~`SettingsGuard` — back out of the Accessibility settings and our own App Info screens (§7),
   with unrecognised-device reporting, bounded back-action, and the in-app disable.~~
   **Done** — AOSP profile verified on an android-36 arm64 emulator: the accessibility list and
   our App Info are blocked consistently, ten unrelated settings screens are not. Device admin
   identifiers were confirmed in step 6, once a receiver existed to open the screen with. Xiaomi
   profile still to be added. The disable path was later reshaped into the protection mode (§7):
   the guard blocks only while the user has armed it, and disarming is the timed operation.
   <!-- step: mobile.settings-guard -->
6. ~~Device Admin — `DeviceAdminReceiver` for uninstall friction, plus an `onDisableRequested`
   warning. Plain admin only; no owner-only calls. Also the only way to verify the
   `DeviceAdminAdd` identifier, which cannot be reached until a receiver exists.~~ **Done** —
   uninstall refused (`DELETE_FAILED_DEVICE_POLICY_MANAGER`) on an android-36 emulator. The
   `DeviceAdminAdd` identifier is confirmed, and the screen turned out to need resource-id
   matching rather than the class; see §7.
   <!-- step: mobile.device-admin -->
7. ~~**The empty harvest** — the catch-all cannot fire on a tree with no text in it, which left
   the device admin list unguarded and silently weakened `mentionsSelf` on *any* screen that
   harvests empty.~~ **Done** — the rows are marked `accessibilityDataSensitive` and were being
   withheld from a service that does not declare `isAccessibilityTool`; declaring it is the whole
   fix, and the device-admin list is now backed out on an android-36 emulator. The leading
   candidate in the backlog (`flagIncludeNotImportantViews`) was wrong and is deliberately still
   unset. Two defects found alongside it are also fixed: an unwatched-package event was treated
   as the user leaving, cancelling the re-look budget, and the now-visible list needed the same
   admin-inactive exemption as the prompt. See [backlog.md](backlog.md#closed-1-the-empty-harvest)
   for the full evidence and what was ruled out.
   <!-- step: mobile.empty-harvest -->
8. ~~Split-screen window resolution~~ **Done**, then recents (§7 and
   [backlog.md](backlog.md#closed-2-split-screen)) — the bypasses that go around step 5 rather
   than defeating it. Ranked after step 7
   because it needs deliberate user intent, while the empty harvest needs none.

   The guard now evaluates every watched window rather than the first one matching the event's
   package, and `GLOBAL_ACTION_BACK` — which takes no window argument — is gated on the matched
   window holding input focus, covering instead when it does not. Verified in real split screen
   on an android-36 emulator, with Settings sitting on the Accessibility page beside a clock app:
   unfocused reads `focused=false` and covers, and the moment the pane is tapped it reads
   `focused=true` and is backed out. Two things found while verifying are worth carrying into the
   recents work: an unfocused pane emits **no accessibility events at all**, so the guard only
   ever sees it when something else makes it re-lay out; and an event from the focused pane was
   being read as the user leaving while a guarded pane sat visible beside it, which cancelled the
   re-look. See [backlog.md](backlog.md#closed-2-split-screen) for both.

   **Recents is deferred**, not skipped — see below.
   <!-- step: mobile.split-screen -->
9. ~~**Tamper log** — append-only local record of guard-state transitions and removal attempts.~~
   **Done** — `policy/TamperLog.kt` (pure: format, tolerant parse, coalescing, trim, session
   classification) and `TamperLogStore.kt` (the `filesDir` edge). Written by the accessibility
   service on connect/disconnect and on every block, by `ProtectionStore` on every mode
   transition, and by the admin receiver including `onDisableRequested`.

   Verified on an android-36 arm64 emulator, all three session paths: a clean off/on through the
   secure setting writes `service_off` then `CLEAN_RESTART`; a force-stop — which delivers no
   unbind, the shape of a process kill or an OEM recents swipe — writes `unclean_stop` then
   `UNCLEAN_STOP`; a fresh install writes `FIRST_RUN`. The log survived an `adb uninstall` that
   the active device admin refused, which is the intended interaction between the two.

   **It records the guard, never the screen.** No scanned text, no verdicts, no scores; details
   are guarded-surface names and nothing else, and a test asserts the event vocabulary stays
   content-free. It survives the service being disabled, force-stop, a process kill, reboot and
   an app update, but **not** clear-data — reachable from App Info, which this product can guard
   but not prevent. Exporting somewhere that survives uninstall means writing user-readable
   history to shared storage and is a product decision, not a storage one.
   <!-- step: mobile.tamper-log -->
10. ~~Foreground service + restart-on-boot.~~ **Done** — `policy/GuardStatus.kt` (pure),
   `GuardStatusService.kt`, and `BootReceiver.kt`; see §8 above for what the service is and is not
   for. **A boot receiver must not write a boot marker the rest of the system can trigger** —
   measured while building step 9 and recorded here because it is not discoverable from the docs:
   `ACTION_BOOT_COMPLETED` is delivered to this app every time it leaves the force-stopped state,
   reproducibly, on an emulator minutes into its uptime. A `BOOT` entry was therefore written by
   the force-stop itself, and `classifyConnect` — which trusted it — read the removal-shaped event
   back as an ordinary restart. The reboot test is the monotonic clock going backwards and nothing
   else. `BootReceiver` accordingly starts the status service and writes nothing at all.

   **`classifyConnect` had to stop reading a single line, and that is the load-bearing change
   here.** The status service outlives the accessibility service by design — noticing that the
   guard stopped is its entire job — so it writes *between* sessions. Reading only the last entry
   would have classified every clean off-and-on as a kill, destroying the one signal step 9 exists
   to produce, and a small `elapsed` written after a reboot but before the guard binds would have
   hidden the clock discontinuity behind it. It now takes the trailing `CLASSIFY_TAIL` entries,
   anchors on the last `sessionBoundary` event, and reads the clock over the window from that
   boundary to now — which also stops an *older* boot inside the tail from excusing the session
   that just ended.

   Verified on an android-36 arm64 emulator: an `adb` disable while armed is recorded within the
   same second (`service_off` then `guard_unprotected`) and the notification says the guard is not
   running; re-enabling reads `CLEAN_RESTART` **past** that intervening entry; a reboot reads
   `AFTER_REBOOT` and `BOOT_COMPLETED` restores the service; a force-stop takes the service with it
   (nothing restarts it, which is what the tamper log is for) and reads `UNCLEAN_STOP` on the next
   connect; and with nothing armed and the guard disabled the service stops itself and posts
   nothing.
   <!-- step: mobile.foreground-status-boot -->
11. ~~`VpnService` DNS filter.~~ **Done for DNS; SNI/IP deferred to step 13.** `policy/NetworkGuard.kt`
    (pure) + `NetworkGuardService.kt`, over a new `packages/net-shield-ffi` wrapping net-shield's
    `dns`, `udp` and `dns_shield` modules. See §5 for why the DNS/SNI split is forced by the
    platform and for the decisions that should not be relitigated.

    **Verified on an android-36 arm64 emulator** — `scripts/smoke-test-vpn.sh`, which is the
    network-path counterpart to `smoke-test.sh` and asserts all five claims: restored after a
    reboot without the app being opened, `tun0` carrying the declared address, a listed name
    refused locally, an unlisted one resolving through the forwarder, and consent loss tearing the
    interface down, recording it, and lifting the block. Two missing manifest permissions and a
    missing reboot restore were found by that run; see §5.

    Rules come from `filesDir/blocklist.txt` through `BlocklistStore` — added here because
    `net-shield-ffi` ships a placeholder rule set by design, so without a runtime source the
    filter had nothing to enforce and no way to be tested against a name that really resolves.

    Without `setAlwaysOnVpnPackage` (owner-only) the VPN can be turned off in Settings like
    anything else. **The identifier for that screen is deliberately not in `SettingsProfiles`
    yet**, because §7's hard rule applies: every entry there must be dumped from a running device,
    never inferred, and the first draft of that table was written from plausible-looking names of
    which almost all were wrong. The self-mention catch-all plausibly already covers it — the VPN
    settings list names the active VPN app — but "plausibly" is the reason to dump it rather than
    the reason not to. `TamperEvent.NETWORK_GUARD_REVOKED` records the removal either way, which
    is the same ceiling every other bypass here sits under.
   <!-- step: mobile.vpn-dns-filter -->
12. ~~`MediaProjection` capture.~~ **Done for capture; the analysis waits on `image-sandbox`.**
    `policy/ScreenCapture.kt` and `policy/FrameGate.kt` (pure) + `ScreenCaptureService.kt` +
    `FrameSink.kt`, with the consent flow in `MainActivity`. See §6 for the start order, the gate,
    and the two teardown paths adb cannot reach.

    **Verified on an android-36 arm64 emulator** — `scripts/smoke-test-capture.sh` asserts six
    claims: the start order lands a `mediaProjection`-typed FGS with its notification and a tamper
    entry; frames arrive at the size `captureSize` derives from the display; the app reports
    scanning as on while it is on; the system stop chip is noticed and recorded as revoked; the
    session's frame count stays in single digits over a static screen; and a reboot neither
    restores capture nor leaves the app claiming it did.

    **The defect that run found is the one worth carrying forward**: `isRunning` was set in the
    service, but `onActivityResult` runs *before* `onResume`, so the onboarding screen re-read the
    flag before `startForeground` had happened and reported "Screen scanning is off" over a live
    projection. It is now set in `start()` before the service is dispatched and cleared on every
    refusal. A status surface that under-reports is the failure mode this product cannot afford —
    it is the same rule as the unsupported-device notice — and it was invisible to unit tests and
    to logcat alike; a screenshot is what showed it.

    Ordered before the SNI/IP work (now step 14) rather than after it because it needs no new
    Rust: the userspace TCP stack SNI filtering wants is a larger piece of work than the whole
    capture path was. What remains of this step is the image path — step 13, and §9.
   <!-- step: mobile.mediaprojection-capture -->
13. **The image path** — §9, and the rest of step 12. Ordered ahead of step 14 because capture
    without it produces frames nobody looks at, which is the one part of this module that
    currently costs battery and returns nothing.

    a. `packages/classifier-head` + its FFI wrapper — pure arithmetic, no artifact needed, and
       the reason this sub-order starts here. See [classifier-head/plan.md](../classifier-head/plan.md).
    b. The LiteRT interpreter on Android over a backbone `.tflite`, emitting the embedding —
       which first needs `machine-learning` to ship an embedding-only export, since the current
       one emits logits.
    c. Model provisioning from `filesDir`, and `FrameSink` wired to the two.

    **Blocked on an artifact for verification, not for building.** `data/models/` is gitignored
    and nothing is on disk, so (a) is fully testable today, (b) and (c) can be built and shown
    to fail open, and only an exported checkpoint makes an end-to-end smoke test possible.
   <!-- step: mobile.image-path -->
14. SNI/IP filtering in the VPN, which needs the userspace TCP stack §5 describes. Reuses
    net-shield's `extract_sni` and `IpFilter` over the FFI surface step 11 established.
    <!-- step: mobile.sni-ip-filter -->

#### Reference documents — steps 7 and 8

- [`AccessibilityServiceInfo`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo) — the `accessibilityFlags` values, `FLAG_INCLUDE_NOT_IMPORTANT_VIEWS` and `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` among them
- [`AccessibilityServiceInfo#isAccessibilityTool()`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#isAccessibilityTool()) and the [`android:isAccessibilityTool`](https://developer.android.com/reference/android/R.attr#isAccessibilityTool) attribute — what step 7 turned on, and read alongside [`View#setAccessibilityDataSensitive`](https://developer.android.com/reference/android/view/View#setAccessibilityDataSensitive(int)) and [`AccessibilityNodeInfo#isAccessibilityDataSensitive()`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo#isAccessibilityDataSensitive()), which are the filtering it exempts the service from
- [`FLAG_INCLUDE_NOT_IMPORTANT_VIEWS`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#FLAG_INCLUDE_NOT_IMPORTANT_VIEWS) — the step 7 candidate that turned out **not** to be the cause; read alongside [`importantForAccessibility`](https://developer.android.com/reference/android/view/View#attr_android:importantForAccessibility), which is what it overrides
- [`AccessibilityService.getWindows()`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService#getWindows()) and [`AccessibilityWindowInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityWindowInfo) — window enumeration for step 8, including `isActive`/`isFocused`
- [`GLOBAL_ACTION_BACK`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService#GLOBAL_ACTION_BACK) — note it takes no window argument, which is the hazard recorded in step 8
- [`AccessibilityNodeInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo) — `getChild`, `refresh`, and what each does and does not re-fetch
- [`UiAutomation.setServiceInfo`](https://developer.android.com/reference/android/app/UiAutomation#setServiceInfo(android.accessibilityservice.AccessibilityServiceInfo)) — why a `uiautomator` dump and a bound service can see different trees

## Gotchas learned the hard way

Specification: [gotchas.md](gotchas.md).

## Verification

- Unit tests: `./gradlew :app:testDebugUnitTest` (no emulator, no NDK required).
- APK: `./gradlew :app:assembleDebug`.
- FFI tests: `cargo test` from `packages/text-policy-ffi`.
- End-to-end, text path: `scripts/smoke-test.sh` against a booted emulator or device.
- End-to-end, network path: `scripts/smoke-test-vpn.sh` (reboots the device — it is the only
  reliable DNS cache flush available to a non-root shell).
- End-to-end, capture path: `scripts/smoke-test-capture.sh` (also reboots, to prove capture does
  *not* come back).

`ANDROID_HOME` must point at the SDK (or add `local.properties`).

### Emulator

Apple Silicon needs an **arm64-v8a** image — which is also an ABI `build-ffi.sh` builds:

```
sdkmanager --sdk_root=$ANDROID_HOME --install "system-images;android-36;google_apis;arm64-v8a"
avdmanager create avd -n holyblocker-test -k "system-images;android-36;google_apis;arm64-v8a"
emulator -avd holyblocker-test -no-window -no-audio -no-snapshot
```

Use `google_apis`, not `google_apis_playstore`: the smoke test enables the service by writing
`enabled_accessibility_services` directly, which a Play-store image does not permit.

Two traps worth knowing, both hit during bring-up:

- **Multiple SDK roots.** Homebrew's `sdkmanager`/`avdmanager` resolve their root from their own
  install location (`/opt/homebrew/share/android-commandlinetools`), not `$ANDROID_HOME`. If the
  NDK or a system image lands there, Gradle and `avdmanager` will not see it. Installing
  `cmdline-tools;latest` into `$ANDROID_HOME` and using that copy keeps one root authoritative.

  This has already bitten once: an NDK installed via Homebrew's `sdkmanager` landed in the
  Homebrew root, and `build-ffi.sh` reported `Could not find any NDK` while the NDK was in fact
  present. Point `ANDROID_NDK_HOME` at it rather than reinstalling:

  ```bash
  export ANDROID_NDK_HOME=/opt/homebrew/share/android-commandlinetools/ndk/27.2.12479018
  ```

  Check both roots before concluding a component is missing.
- **The accessibility setting reverts.** Shortly after first boot the system rewrites the
  accessibility defaults, silently clobbering a `settings put` that reported success. The smoke
  test writes, verifies, and retries for this reason.

## What this does not cover

- **MITM / `scan_body`** — impossible on stock Android; see the rationale above.
- **Browser extension** — survives only on Firefox for Android; Chrome on Android has no
  extensions.
- **Play Store distribution** — explicitly not planned; sideloading is the assumed channel.
- **iOS** — see [content-interception.md](../../decisions/content-interception.md).
- **Real dictionaries** — the FFI ships the same placeholder starter lexicon as `mitm-proxy`'s
  `build_default_engine`. Loading real dictionaries from an embedded asset is a text-policy
  concern, not a mobile one.

## OEM coverage — deferred

Screen guarding (§7) is the one part of this module whose correctness depends on identifiers
that vary per vendor. The obvious mitigation — test on many devices — is not currently
available, and the alternatives are worse than they look:

- **Emulators only cover AOSP.** One UI and HyperOS are proprietary and are not published as
  system images; `sdkmanager` offers AOSP and Google APIs images only. Genymotion's named
  device profiles are AOSP with spoofed build properties — they satisfy `Build.MANUFACTURER`
  while shipping the AOSP Settings app, which makes them actively misleading here.
- **Install-time device gating is not available.** Distribution is by sideloading and Play is
  an explicit non-goal, so there is no Play Console device allowlist to restrict who installs.
- **Device farms work but are per-run.** Samsung Remote Test Lab is free and gives real One UI;
  Firebase Test Lab and BrowserStack cover Xiaomi and others. Useful for a one-shot node-tree
  dump, not for continuous coverage.

**Decision: ship AOSP plus Xiaomi/HyperOS, and fail loudly elsewhere.** AOSP is what the
emulator gives us; Xiaomi is a maintainer's daily device, so its identifiers can be dumped and
regression-tested for real. That is also the pairing shipping blockers converge on — AppBlock
lists Pixel, Samsung and Xiaomi as its supported set for the recents guard.

`SettingsGuard` reports whether it recognises the current device's settings screens, and the app
tells the user that screen protection is unverified on an unrecognised build. Failing visibly on
an untested OEM is correct for a tool whose value is honesty about its own coverage; failing open
and silently is not.

**Samsung is deferred**, not excluded: One UI diverges enough to need real verification, and
Samsung Remote Test Lab provides free access to real hardware when we get to it.

Beyond those, coverage is deferred to **community contribution** — a documented
`uiautomator dump` procedure plus a per-OEM identifier table that outside users can extend, since
the devices needed are exactly the ones contributors already own. This needs a submission and
verification process before it is useful, and that is out of scope until the first two ship.

## Open questions

- **OEM variation in the enable-Accessibility flow** — the Restricted Settings path differs
  across vendors; needs device testing. See [OEM coverage](#oem-coverage--deferred).
- **Durability of Device Owner provisioning** — strongest hold, but requires a fresh device.
- ~~**Foreground-service category**~~ — answered in step 10: `specialUse`, with
  `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` as the justification. None of the typed categories fits, and
  Play review is not a factor for a sideloaded app.
