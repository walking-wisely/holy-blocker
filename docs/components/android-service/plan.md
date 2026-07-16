# Android Service — Implementation Plan

This document is the build plan for `native-modules/android-service/`: the Android edge daemon.

The design rationale — why Android inverts the desktop layer order — lives in
[content-interception.md](../../decisions/content-interception.md#android--the-layer-order-inverts).
For the shared classification pipeline see [content-classification.md](../../architecture/content-classification.md);
for the daemon model across platforms see [edge-daemons.md](../../architecture/edge-daemons.md).

## Why Android is different

On desktop, Layer 1 (the MITM proxy) does true content inspection and Layer 2 (the render path)
covers native apps. On Android that order inverts:

- **Layer 1 collapses to packet filtering.** `VpnService` gives a TUN interface with no root, but
  apps targeting API 24+ trust only the system CA store, so there is no general MITM on stock
  Android. `scan_body` does not exist here. What survives is DNS / SNI / IP filtering —
  net-shield's Phase 1 only.
- **Layer 2 carries everything.** `AccessibilityService` + `MediaProjection` + overlay do all
  content analysis. This is the workhorse.

Distribution is by sideloading; the Play Store is not a target. This removes the policy constraint
that dominates Android content-control apps, and the Restricted Settings friction it creates is
part of the accountability model rather than an obstacle to it (see
[Restricted Settings](#restricted-settings-is-load-bearing) below).

## Responsibility boundary

| Responsibility | Owner |
|---|---|
| Accessibility event capture, filtering, debouncing | `native-modules/android-service/` (Kotlin) |
| Screen capture (`MediaProjection`) session lifecycle | `native-modules/android-service/` (Kotlin) |
| Block overlay rendering | `native-modules/android-service/` (Kotlin + Compose) |
| Device Admin / Device Owner policy application | `native-modules/android-service/` (Kotlin) |
| `VpnService` TUN lifecycle, fd ownership | `native-modules/android-service/` (Kotlin) |
| Packet read loop, domain/SNI/IP dispatch | `packages/net-shield/` (Rust, over JNI) |
| Text normalization, lexicon, scoring, verdict | `packages/text-policy/` (Rust, over JNI) |
| Image classification | `packages/image-sandbox/` (planned) |
| Dashboard UI (config, history, onboarding) | `:ui` process — framework TBD, RN intended (§8) |

The Kotlin layer owns **platform surface and lifecycle**. It does not own policy decisions — those
live in Rust and are called over JNI. Keep it that way: the Kotlin side should be thin enough that
the interesting logic is testable without an emulator.

## Current state

`native-modules/android-service/` does not exist. Nothing is scaffolded.

**The local toolchain is not ready either.** As of writing:

- Android SDK is present at `~/Library/Android/sdk` — platform `android-36.1`, build-tools
  `36.1.0` / `37.0.0`, and `platform-tools/adb`.
- **No JDK installed** (`java` is absent) — required for Gradle.
- **No `cmdline-tools`** — required for `sdkmanager` / `avdmanager`.
- **No system images** and no AVDs — no emulator can be created yet.
- `adb` is not on `PATH`.

See [Bootstrap](#0-bootstrap-toolchain) for the one-time setup.

## Architecture: two processes

```
┌─ main process ────────────────────────────────────┐
│  ShieldVpnService      (VpnService, TUN fd)       │
│  ScreenReaderService   (AccessibilityService)     │
│  CaptureService        (MediaProjection, FGS)     │
│  OverlayController     (WindowManager + Compose)  │
│  PolicyAdmin           (DevicePolicyManager)      │
│  ProtectionStatusStore (single source of truth)   │
│         │                                         │
│         │  JNI                                    │
│         ▼                                         │
│  net-shield (Rust)   text-policy (Rust)           │
└───────────────────────┬───────────────────────────┘
                        │  AIDL (IProtectionService)
┌─ :ui process ─────────▼───────────────────────────┐
│  MainActivity → React Native dashboard            │
└───────────────────────────────────────────────────┘
```

**Why a separate `:ui` process.** The services are long-lived foreground services; the dashboard is
opened occasionally. Splitting them means the dashboard's heap lives and dies with the dashboard
rather than inflating the process that hosts the services.

Do not oversell this. Android ranks processes for killing primarily by component state, and a
foreground service already ranks high — the services are not in danger today, and this does not make
them unkillable. It is memory isolation and defence-in-depth, not a survival guarantee. The more
durable benefit is the second-order one: a process boundary forces the UI/services seam to be a real
IPC contract instead of shared singletons, which is what keeps the dashboard swappable.

**The cost is real and should not be glossed:** a separate process means AIDL, not a same-process
bridge module. Every value the dashboard reads crosses a parcel boundary. This is the main tax of
the arrangement, and it is why `ProtectionStatus` (below) is designed as one coarse snapshot rather
than a set of fine-grained getters.

**The overlay is Compose, not React Native, in every branch.** Hosting RN in a `WindowManager`
overlay requires keeping a `ReactInstanceManager` warm inside the background service, or eating a
multi-second cold start. On a blocker, overlay latency *is* correctness — a cover that takes two
seconds means two seconds of visible content. So the overlay is native. This is a fixed point of the
design, independent of what the dashboard is written in.

## Modules to add

### 0. Bootstrap (toolchain)

One-time, before any code. Not committed — recorded here so the next person doesn't rediscover it.

```bash
brew install --cask temurin@21          # JDK 21 — required by current AGP
brew install --cask android-commandlinetools

export ANDROID_HOME="$HOME/Library/Android/sdk"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"

# NOTE: google_apis, NOT google_apis_playstore — see Device Owner below.
sdkmanager "system-images;android-34;google_apis;arm64-v8a"
avdmanager create avd -n hb-dev -k "system-images;android-34;google_apis;arm64-v8a"
```

### 1. `policy-admin` — Device Admin / Device Owner

```
src/main/kotlin/.../admin/HolyBlockerAdminReceiver.kt
src/main/kotlin/.../admin/PolicyAdmin.kt
src/main/kotlin/.../admin/PolicySpec.kt
```

Responsibilities:

- `HolyBlockerAdminReceiver : DeviceAdminReceiver` — the admin component. Declared in the manifest
  with a `device_admin_receiver.xml` meta-data policy list.
- `PolicySpec` — **pure data**: given a `ProtectionMode`, which user restrictions, always-on-VPN
  setting, and permitted-accessibility list should be in force. No Android imports. This is where
  the test-first rule applies; `PolicyAdmin` itself is a thin applicator.
- `PolicyAdmin` — applies a `PolicySpec` through `DevicePolicyManager`. Wraps:
  - `setAlwaysOnVpnPackage(admin, pkg, lockdownEnabled = true)` — **the highest-value call in the
    package.** Device Owner only. This is what makes the degraded Layer 1 non-bypassable.
  - `addUserRestriction` — `DISALLOW_CONFIG_VPN`, `DISALLOW_INSTALL_UNKNOWN_SOURCES`,
    `DISALLOW_SAFE_BOOT`, `DISALLOW_FACTORY_RESET`.
  - `setPermittedAccessibilityServices(admin, listOf(ourService))` — whitelists ours, excludes
    others.
  - `setUninstallBlocked` for companion packages.

```kotlin
enum class ProtectionMode { OFF, MONITOR, ENFORCE }

data class PolicySpec(
    val userRestrictions:        Set<String>,
    val alwaysOnVpnLockdown:     Boolean,
    val permittedA11yServices:   List<String>?,   // null = no restriction
) {
    companion object { fun forMode(mode: ProtectionMode, selfPkg: String): PolicySpec }
}

class PolicyAdmin(private val dpm: DevicePolicyManager, private val admin: ComponentName) {
    fun isDeviceOwner(): Boolean
    fun apply(spec: PolicySpec)
}
```

**Device Owner cannot silently enable our AccessibilityService.** `setSecureSetting` is limited to a
small allowlist and `enabled_accessibility_services` is not on it. The user consent flow is
unavoidable in production. For tests only, `adb shell settings put secure
enabled_accessibility_services <pkg>/<svc>` shortcuts it — do not design around an auto-enable that
does not exist.

**Provisioning constraints** (these bite, and the emulator is the right place to learn them):

- `dpm set-device-owner` fails if **any** account exists on the device. Use a
  `google_apis` image, not `google_apis_playstore` — GMS provisions an account during setup and the
  call will fail with *"there are already some accounts on the device"*.
- Provision immediately after a wipe:

  ```bash
  emulator -avd hb-dev -wipe-data
  adb install app.apk
  adb shell dpm set-device-owner com.holyblocker/.admin.HolyBlockerAdminReceiver
  ```

- Set `android:testOnly="true"` in the manifest **for dev builds only**. A device owner cannot
  normally be uninstalled; without this flag every iteration costs a full wipe-and-reprovision. With
  it, `adb uninstall` and `dpm remove-active-admin` work. It must not ship.

### 2. `status` — the single observable snapshot

```
src/main/kotlin/.../status/ProtectionStatus.kt
src/main/kotlin/.../status/ProtectionStatusStore.kt
```

The dashboard's real job is **permission orchestration**, not settings: enable Accessibility (via
the Restricted Settings detour), grant overlay, consent to `MediaProjection`, activate Device Admin,
set always-on VPN — then continuously watch for revocation and re-prompt. Every screen reads native
state and launches native intents. Getting this one interface right is what keeps that pleasant
across a parcel boundary; letting it sprout fifteen ad-hoc calls is what makes it miserable.

```kotlin
enum class GrantState { GRANTED, DENIED, RESTRICTED_SETTINGS_BLOCKED, UNAVAILABLE }

@Parcelize
data class ProtectionStatus(
    val accessibility: GrantState,
    val overlay:       GrantState,
    val vpn:           GrantState,
    val deviceAdmin:   GrantState,
    val projection:    GrantState,
    val mode:          ProtectionMode,
) : Parcelable {
    val isFullyProtected: Boolean get() = ...
}
```

- `ProtectionStatusStore` — owns a `StateFlow<ProtectionStatus>` in the main process. Recomputes on
  service lifecycle changes and on resume. **The mapping from raw platform queries to
  `ProtectionStatus` is a pure function** and gets unit tests; the queries themselves are the thin
  edge.
- Exposed to `:ui` over AIDL as one snapshot + a change callback. One parcel, not five round-trips.

### 3. `accessibility` — the text and event path

```
src/main/kotlin/.../a11y/ScreenReaderService.kt
src/main/kotlin/.../a11y/EventFilter.kt
src/main/kotlin/.../a11y/NodeTextExtractor.kt
```

- `ScreenReaderService : AccessibilityService` — subscribes to the three signals
  [edge-daemons.md](../../architecture/edge-daemons.md#android-daemon) calls for:
  `TYPE_WINDOW_STATE_CHANGED`, `TYPE_VIEW_SCROLLED`, and `TYPE_WINDOW_CONTENT_CHANGED`. The last one
  catches subtree updates that the other two miss — a feed loading new items without a scroll — and
  is also by far the noisiest, which is most of why `EventFilter` exists. Per the same document,
  these events are **not** a sufficient correctness mechanism on their own: custom-rendered views may
  still need periodic or diff-triggered scans while a monitored surface is active.
- `EventFilter` — **pure**: debouncing, per-package rate limiting, and same-content suppression.
  Decides *whether* an event warrants a scan. Test-first; no Android imports beyond the event data
  it is handed (pass a plain data class, not `AccessibilityEvent`).
- `NodeTextExtractor` — walks the `AccessibilityNodeInfo` tree into a flat string. This is the
  **non-OCR shortcut** for the text path and should be preferred over capture whenever the text is
  available this way — it is cheaper and exact.
- Result goes to `text-policy` over JNI. A `Block` verdict raises the overlay and may issue
  `performGlobalAction(GLOBAL_ACTION_BACK)`.

### 4. `overlay` — the cover

```
src/main/kotlin/.../overlay/OverlayController.kt
src/main/kotlin/.../overlay/BlockOverlay.kt
```

- `OverlayController` — adds a `ComposeView` to the `WindowManager` with
  `TYPE_APPLICATION_OVERLAY`. Because there is no Activity, the view must be given its own
  `ViewTreeLifecycleOwner`, `ViewTreeViewModelStoreOwner`, and `ViewTreeSavedStateRegistryOwner` or
  Compose will crash on attach. This is the one genuinely fiddly part; it is ~50 lines and then it
  is done forever.
- `BlockOverlay` — the composable. Starts minimal (scrim, reason, dismiss).

**Scope is deliberately open.** If the overlay grows features that overlap the dashboard — verdict
explanation, an appeal flow, a recent-blocks list — that is the signal to reconsider React Native
for the dashboard, because those components would then exist twice in two languages. Nothing else
about RN will produce a warning; it will not be a performance signal. Revisit this consciously at
the point the overlay stops being a scrim.

### 5. `vpn` — TUN lifecycle

```
src/main/kotlin/.../vpn/ShieldVpnService.kt
```

- `ShieldVpnService : VpnService` — builds the interface, owns the `ParcelFileDescriptor`, hands the
  raw fd to net-shield over JNI, and tears down on revoke.
- Reuses net-shield's existing `PacketSink` trait with an Android sink implementation. The domain
  trie, CIDR filter, and SNI parser are already built and platform-neutral — **only the adapter
  layer is new.** This is the payoff for how net-shield was factored.
- No MITM. `FilterAction::Proxy` is not reachable on this platform; the Android sink treats it as
  `Allow` and logs. Confirm this is the intended degradation before wiring it.

### 6. `capture` — MediaProjection

```
src/main/kotlin/.../capture/CaptureService.kt
src/main/kotlin/.../capture/FrameThrottle.kt
```

- `CaptureService` — a foreground service with `foregroundServiceType="mediaProjection"`. Consent is
  **per session** — there is no persistent grant to cache.

  The ordering is strict on modern Android and easy to get wrong. The foreground service must be
  running before `getMediaProjection()`, but *after* the user has consented — it is not started
  first:

  1. `MediaProjectionManager.createScreenCaptureIntent()` → launch it.
  2. Handle the activity result; keep `resultCode` + `data`.
  3. **Only now** start the `mediaProjection`-typed foreground service and call `startForeground()`.
  4. From inside that service, `getMediaProjection(resultCode, data)`.
- `FrameThrottle` — **pure**: perceptual-difference / hash gating so identical frames don't re-run
  OCR or the image model. Test-first.
- Deferred until the accessibility text path works — it is the expensive path and the shortcut
  covers a lot.

### 7. `jni` — the Rust bridge

```
src/main/kotlin/.../jni/TextPolicy.kt
src/main/kotlin/.../jni/NetShield.kt
src/main/jniLibs/…
```

- Thin `external fun` declarations over the Rust cdylibs, cross-compiled for `arm64-v8a` and
  `x86_64` (the latter for emulators on Intel hosts; Apple Silicon uses `arm64-v8a`).
- **Blocked on `packages/text-policy` shipping its FFI surface**, which its own plan lists as the
  next step. Sequence that first.

### 8. `dashboard` — `:ui` process

Framework-independent, true either way:

- `MainActivity` declared `android:process=":ui"`.
- It binds the AIDL `IProtectionService` and consumes exactly three things: a `ProtectionStatus`
  snapshot, a change subscription, and one launcher per grant flow. Resist growing that surface.

**React Native is the current intent**, not a settled conclusion — see the implementation order,
where this is the last step and the deciding criterion is written down. If RN is confirmed:

- Bare workflow / dev builds — Expo Go cannot load custom native code.
- One `TurboModule` wrapping the AIDL binding above.

If the overlay has by then grown surfaces that overlap the dashboard (verdict explanation, appeal
flow, recent-blocks list), those components already exist in Compose and the dashboard should be
Compose too rather than rebuilding them in a second language.

## Restricted Settings is load-bearing

On Android 13+, Accessibility and Device Admin are placed behind **Restricted Settings** for any app
not installed from the Play Store: the first enable attempt is blocked until the user opens
App Info → ⋮ → *Allow restricted settings* and authenticates with the device PIN. If the partner
holds the PIN, the protected user cannot perform that grant on their own — this is the Android
analogue of the macOS admin-held-credential lock.

**Be precise about what this does and does not buy.** The gate covers *granting*, not *revoking*:

- **Gated by the PIN:** the initial grant, and any re-grant after the permission is cleared.
- **Not gated by anything:** turning an already-enabled Accessibility service **off**. A user can
  open Settings → Accessibility and toggle it off without ever seeing the PIN prompt.
  `setPermittedAccessibilityServices` does not help here — it constrains *which* services may be
  enabled, not whether ours stays enabled.

So the property is **detection and re-grant friction, not prevention**. The daemon must notice it has
been disabled (this is what `ProtectionStatusStore` in §2 is for) and the accountability model rests
on the partner being told, plus the fact that turning it back on needs the PIN again. Do not write
copy — or design policy — that claims the service cannot be switched off. It can.

**It is not reproducible via `adb install`.** Shell installs are not attributed to an unknown-sources
installer, so the toggle will simply work and the gate will never appear. To exercise the real flow,
`adb push` the APK and install it through the emulator's Files app. Do this deliberately at least
once before trusting the tamper-resistance story.

## Implementation order

Ordered by risk, not by dependency: the provisioning chain is the biggest unknown in the design and
the cheapest to falsify. Everything through step 4 is identical regardless of what the dashboard is
written in.

1. **Bootstrap the toolchain** (§0) and create a `google_apis` AVD. Confirm `adb shell dpm
   set-device-owner` succeeds at all before writing a line of feature code.
2. **`policy-admin`** — `PolicySpec` pure logic + tests first, then `PolicyAdmin`. Milestone: **Device
   Owner provisions on a wiped emulator and always-on VPN with lockdown holds** — verify a
   non-whitelisted app cannot reach the network with the VPN service stopped. This is the riskiest
   claim in the whole Android design; prove it before building on it.
3. **`status`** — pure mapping + tests, then the store, then AIDL. Observable via `dumpsys` and
   logcat; no UI needed.
4. **`accessibility` + `overlay`** — `EventFilter` tests first, then the service, then the overlay.
   Milestone: **a hard-coded blocked keyword raises the cover.** The overlay appearing is the dev UI
   — this is what tells you the pipeline works, not a dashboard.
5. **`jni` / `text-policy`** — replace the hard-coded keyword with real verdicts. Requires
   text-policy's FFI surface to land first.
6. **`vpn`** — `ShieldVpnService` + Android `PacketSink`. Reuses net-shield wholesale.
7. **`capture` + `FrameThrottle`** — the expensive path, last.
8. **`dashboard`** — the cheapest thing to swap and the last thing needed. Decide RN vs. Compose
   here, informed by what the overlay became in step 4.

## Verification

- Kotlin pure logic (`PolicySpec`, `EventFilter`, `FrameThrottle`, status mapping): JVM unit tests
  via `./gradlew :android-service:test`. No emulator, no device.
- Platform integration: instrumented tests / manual emulator runs.
- Service observability without a dashboard: `adb logcat`, `adb shell dumpsys accessibility`,
  `adb shell dumpsys activity services`.

## Reference documents

### Device Admin / Device Owner

- [`DevicePolicyManager`](https://developer.android.com/reference/android/app/admin/DevicePolicyManager) — canonical API surface.
- [`DevicePolicyManager.setAlwaysOnVpnPackage`](https://developer.android.com/reference/android/app/admin/DevicePolicyManager#setAlwaysOnVpnPackage(android.content.ComponentName,%20java.lang.String,%20boolean)) — Device Owner only; `lockdownEnabled` is the parameter that blocks non-VPN traffic.
- [`DeviceAdminReceiver`](https://developer.android.com/reference/android/app/admin/DeviceAdminReceiver) — receiver contract and meta-data XML format.
- [`UserManager`](https://developer.android.com/reference/android/os/UserManager) — canonical list of `DISALLOW_*` user restriction constants.
- [Build a device policy controller](https://developer.android.com/work/dpc/build-dpc) — provisioning paths and the no-accounts constraint.

### Accessibility

- [`AccessibilityService`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService) — lifecycle, `performGlobalAction`.
- [`AccessibilityEvent`](https://developer.android.com/reference/android/view/accessibility/AccessibilityEvent) — `TYPE_WINDOW_STATE_CHANGED`, `TYPE_VIEW_SCROLLED` semantics.
- [`AccessibilityNodeInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo) — tree traversal and recycling rules.

### Overlay

- [`WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams#TYPE_APPLICATION_OVERLAY) — the only overlay type available since API 26.
- [`Settings.ACTION_MANAGE_OVERLAY_PERMISSION`](https://developer.android.com/reference/android/provider/Settings#ACTION_MANAGE_OVERLAY_PERMISSION) — the grant intent.
- [Compose outside an Activity](https://developer.android.com/reference/kotlin/androidx/compose/ui/platform/ComposeView) — see also `setViewTreeLifecycleOwner` / `setViewTreeSavedStateRegistryOwner`.

### VPN

- [`VpnService`](https://developer.android.com/reference/android/net/VpnService) — `Builder`, `establish()`, `prepare()`, revoke handling.
- [`VpnService.Builder.establish`](https://developer.android.com/reference/android/net/VpnService.Builder#establish()) — returns the `ParcelFileDescriptor` whose fd is handed to net-shield.

### Screen capture

- [`MediaProjectionManager`](https://developer.android.com/reference/android/media/projection/MediaProjectionManager) — consent intent.
- [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) — `mediaProjection` type and its start-order requirement.

### Process separation and IPC

- [`android:process`](https://developer.android.com/guide/topics/manifest/application-element#proc) — manifest attribute semantics.
- [AIDL](https://developer.android.com/develop/background-work/services/aidl) — interface definition and parcelable rules.

### Distribution

- [Restricted settings](https://support.google.com/android/answer/12623953) — user-facing description of the Android 13+ gate.

## What this does not cover

- **MITM / `scan_body` on Android** — not possible on stock Android; see
  [content-interception.md](../../decisions/content-interception.md#android--the-layer-order-inverts).
  Do not add a user-store CA install path expecting it to work.
- **iOS** — a structurally different and lesser product; see the iOS section of the same document.
- **Play Store distribution** — explicitly not a target. Do not add Play policy compromises.
- **Image classification** — `packages/image-sandbox/` (planned). This package only supplies frames.
- **Model training / export** — `machine-learning/`. This package consumes a `.tflite` artifact.
- **QUIC / HTTP3** — same open policy question as net-shield; deferred.
- **OEM variation** — Samsung and Xiaomi diverge in the Accessibility enable flow and in Device Owner
  durability. The emulator cannot tell you about this; it needs real hardware and is a known open
  risk.
