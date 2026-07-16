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
- `VpnService` DNS/SNI filter — not yet created.
- `MediaProjection` capture + image path — not yet created.
- Device Admin / tamper resistance — not yet created.

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

```
app/src/main/kotlin/com/holyblocker/mobile/policy/
```

`ScanGate` is the load-bearing piece. `AccessibilityService` fires window-content and scroll
events far faster than a human scrolls — several per frame while a list moves — and each one
would otherwise mean a full normalize + lexicon pass on the UI-event path. The gate drops:

| Skip reason | Rule |
|---|---|
| `SELF_PACKAGE` | never scan our own overlay — its text would re-trigger the cover |
| `NO_TEXT` | the node tree yielded nothing usable |
| `DUPLICATE` | identical text within the same app — the verdict cannot differ |
| `DEBOUNCED` | under 300 ms since the last scan *of the same app* |

An app switch bypasses both dedupe and debounce: the whole screen just changed, and that is
the highest-signal moment there is. Only a real evaluation advances the debounce clock, so a
stream of duplicates cannot hold the window open and starve a genuine change.

`BLUR` maps to `COVER` because the MVP overlay is opaque and has no partial-obscure mode;
erring toward covering matches the formation model's "tune blocking for recall".

### 2. `ScreenGuardService` — the AccessibilityService — **Done**

Depth-first walk of `rootInActiveWindow` collecting `text` and `contentDescription`, bounded
by depth (40) and fragment count (400). The bounds are not incidental: web views expose very
deep trees and this runs on the UI-event path, so an unbounded walk jank the foreground app.

#### Reference documents

- [`AccessibilityService`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService)
- [`AccessibilityServiceInfo`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo) — the `<accessibility-service>` config attributes
- [`AccessibilityNodeInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo)
- [Build a custom accessibility service](https://developer.android.com/guide/topics/ui/accessibility/service)

### 3. `OverlayController` — the cover — **Done**

Uses `TYPE_ACCESSIBILITY_OVERLAY`, which an accessibility service may draw *without* a
separate "display over other apps" grant, and which sits above `TYPE_APPLICATION_OVERLAY`.
The opaque cover swallows touches so content underneath cannot be interacted with blind; the
warn tint stays passive (`FLAG_NOT_TOUCHABLE`).

#### Reference documents

- [`WindowManager.LayoutParams`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams) — overlay types and flags
- [`TYPE_ACCESSIBILITY_OVERLAY`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams#TYPE_ACCESSIBILITY_OVERLAY)
- [`SYSTEM_ALERT_WINDOW`](https://developer.android.com/reference/android/Manifest.permission#SYSTEM_ALERT_WINDOW) — needed only for the fallback path

### 4. `MainActivity` — onboarding — **Done**

Reads `Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES` on resume (the user returns straight
from the toggle) and explains the Restricted Settings detour.

**Sideload friction is a feature here.** On Android 13+, Accessibility and Device Admin sit
behind Restricted Settings for any app not installed from the Play Store. The first enable
attempt is blocked until the user opens App info → ⋮ → "Allow restricted settings" and
authenticates with the device PIN. If the partner holds the PIN, the protected user cannot
enable — or later disable — the service without them. That is the Android analogue of the
macOS admin-held-credential lock, and it is why the parsing in
`AccessibilityServiceStatus` is a real parser rather than a `contains` check: OEM builds vary
in whitespace, trailing separators, and whether the entry is fully qualified.

#### Reference documents

- [`Settings.Secure#ENABLED_ACCESSIBILITY_SERVICES`](https://developer.android.com/reference/android/provider/Settings.Secure#ENABLED_ACCESSIBILITY_SERVICES)
- [Restricted settings](https://support.google.com/android/answer/12623953)

### 5. `VpnService` — DNS/SNI/IP filter — not yet created

Reuses net-shield's `DomainFilter`/`IpFilter` and `extract_sni` over the same FFI pattern.
No TLS termination — see the rationale above.

#### Reference documents

- [`VpnService`](https://developer.android.com/reference/android/net/VpnService)
- [RFC 6066 §3](https://www.rfc-editor.org/rfc/rfc6066#section-3) — TLS SNI extension
- [RFC 8446](https://www.rfc-editor.org/rfc/rfc8446) — TLS 1.3 ClientHello

### 6. `MediaProjection` — capture + image path — not yet created

Feeds OCR and the image classifier once `packages/image-sandbox` exists. Per-session user
consent; cannot run silently.

#### Reference documents

- [`MediaProjection`](https://developer.android.com/reference/android/media/projection/MediaProjection)
- [`MediaProjectionManager`](https://developer.android.com/reference/android/media/projection/MediaProjectionManager)

### 7. Tamper resistance — not yet created

Device Admin can prevent uninstall; Device Owner is stronger but needs fresh-device
provisioning. The service can also detect its own Accessibility/VPN toggle being switched off.

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
2. ~~`text-policy-ffi` UniFFI crate + `NativeTextPolicy` adapter.~~ **Done.**
3. ~~`ScreenGuardService` + `OverlayController` + onboarding.~~ **Done.**
4. ~~Build the `.so` (NDK) and validate on a device — the first end-to-end run.~~ **Done** —
   `scripts/build-ffi.sh` builds all three ABIs; `scripts/smoke-test.sh` passes on an
   android-36 arm64 emulator.
5. Foreground service + restart-on-boot, so the guard survives.
6. `VpnService` DNS/SNI filter.
7. `MediaProjection` capture once `image-sandbox` lands.
8. Device Admin tamper resistance.

## Verification

- Unit tests: `./gradlew :app:testDebugUnitTest` (no emulator, no NDK required).
- APK: `./gradlew :app:assembleDebug`.
- FFI tests: `cargo test` from `packages/text-policy-ffi`.
- End-to-end: `scripts/smoke-test.sh` against a booted emulator or device.

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

## Open questions

- **OEM variation in the enable-Accessibility flow** — the Restricted Settings path differs
  across vendors; needs device testing.
- **Durability of Device Owner provisioning** — strongest hold, but requires a fresh device.
- **Foreground-service category** — which `foregroundServiceType` Android 14+ will accept for
  a guard that is neither media nor location.
