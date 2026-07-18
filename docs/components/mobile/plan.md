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
  AOSP profile, verified on an android-36 arm64 emulator, device-admin identifiers included.
  Xiaomi and Samsung have no profile at all.
- `policy/ReleaseSchedule.kt` — request → cooldown → window timing, monotonic — **Done.**
- `GuardSuspension.kt` — storage edge for the exit path — **Done.**
- `VpnService` DNS/SNI filter — not yet created.
- `MediaProjection` capture + image path — not yet created.
- `admin/HolyBlockerAdminReceiver.kt` — device admin, so uninstall is refused until it is
  deactivated — **Done**, verified on an android-36 arm64 emulator.
- Tamper log — not yet created. See [backlog.md](backlog.md).

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

Feeds OCR and the image classifier once `packages/image-sandbox` exists. Consent is **per
session** — there is no persistent grant to cache, and it cannot run silently.

**The start order is strict on modern Android and easy to get wrong.** The foreground service
must be running before `getMediaProjection()`, but *after* the user has consented — it is not
started first:

1. `MediaProjectionManager.createScreenCaptureIntent()` → launch it.
2. Handle the activity result; keep `resultCode` + `data`.
3. **Only now** start the `mediaProjection`-typed foreground service and call `startForeground()`.
4. From inside that service, `getMediaProjection(resultCode, data)`.

A frame throttle — perceptual-difference or hash gating, so identical frames do not re-run OCR
or the image model — is pure logic and gets unit tests like the rest of the decision core.

#### Reference documents

- [`MediaProjection`](https://developer.android.com/reference/android/media/projection/MediaProjection)
- [`MediaProjectionManager`](https://developer.android.com/reference/android/media/projection/MediaProjectionManager)
- [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) — the `mediaProjection` type and its start-order requirement

### 7. Tamper resistance — partially built

#### Device Owner is not available to this product

Device Owner provisioning requires a factory-reset device and grants a level of control that a
user installing a self-imposed accountability tool should not reasonably be asked for — the
request itself reads as hostile. **This module targets plain Device Admin and must not be
designed around owner-only capability.**

That rules out, in full: `addUserRestriction` (and therefore `DISALLOW_APPS_CONTROL`,
`DISALLOW_CONFIG_VPN`, `DISALLOW_SAFE_BOOT`, `DISALLOW_FACTORY_RESET`),
`setUninstallBlocked`, `setUserControlDisabledPackages`, `setPermittedAccessibilityServices`,
and `setAlwaysOnVpnPackage`. Each is documented as callable by a device owner, profile owner,
or delegate; a legacy device admin is none of those. Device admin was additionally deprecated
for enterprise use in Android 9, so most of its remaining policy surface is dead weight.

**There is therefore no enforcement primitive available to this product. Not one.** Nothing we
can call prevents force-stop, prevents disabling the accessibility service, or blocks uninstall
outright.

#### What Device Admin still buys

Two things survive, and both are worth having:

- **Uninstall requires deactivation first.** With an admin active, Android refuses the uninstall
  with "This app is an active device administrator and must be deactivated before uninstalling."
  This is built-in framework behaviour rather than a policy call, so the deprecation does not
  touch it. On Android 13+ activating the admin is itself behind Restricted Settings for a
  sideloaded app, so the *grant* needs the device PIN.
- **`onDisableRequested()`** fires after the user confirms deactivation but before it takes
  effect, and may return a warning string. It is the last reliable moment to record the event.

Both are built (`admin/HolyBlockerAdminReceiver`) and verified on an `android-36` emulator:
`adb uninstall` returns `DELETE_FAILED_DEVICE_POLICY_MANAGER` while the admin is active. The
receiver declares **no** `<uses-policies>` — neither property needs one, and every tag declared
would ask the user to grant a power the product never exercises.

**`DeviceAdminAdd` is one activity for both directions**, and this is the trap in guarding it.
It is the activation prompt while the admin is off and the deactivation prompt once it is on, so
guarding it unconditionally makes the feature impossible to enable. `SettingsGuard` therefore
exempts that surface while `isDeviceAdminActive()` is false.

The exemption cannot be keyed on the activity class. Opening the prompt emits events carrying
`android.widget.FrameLayout`; the real class arrives only on a later event that is not reliably
sent. A class-keyed exemption silently misses, the screen falls through to the `SELF_IN_SETTINGS`
catch-all, and the guard ejects the user from the screen that turns the admin on — observed on
device, not theorised. The match is keyed on `admin_name` / `add_msg` / `admin_warning` resource
ids instead, which are present on every event for that screen.

#### The accessibility service is the enforcement mechanism

The absence of a *policy API* lever does not mean disabling cannot be blocked. It means the
block is implemented by the accessibility service watching for the screens that would remove it,
and ejecting the user before they arrive. This is what shipping blockers do — AppBlock's Strict
Mode offers "block device settings", "disable AppBlock uninstalling", "block recent apps"
(listed as available on Pixel, Samsung and Xiaomi) and "block split screen", none of which
require Device Owner.

**Back out; do not merely cover.** `performGlobalAction(GLOBAL_ACTION_BACK)` on
`TYPE_WINDOW_STATE_CHANGED` removes the user from the screen before the toggle is reachable, so
the race between the window rendering and an overlay attaching never arises. The cover is a
secondary affordance for explaining what happened, not the mechanism.

The screens to guard:

| Surface | Why |
|---|---|
| Settings → Accessibility (list and our entry) | the disable toggle |
| **The system uninstall dialog for our package** | **reached by long-pressing the launcher icon — never touches Settings, so no settings identifier applies. Three taps and pure muscle memory.** |
| Settings → Device admin apps | deactivation, which gates uninstall |
| App Info for our own package | force-stop, clear data, uninstall |
| Recents | swiping the app away can stop the service (see below) |
| Split screen | lets a guarded screen be driven beside another app |

The uninstall dialog is matched on **self-mention only, never by class**: the installer is shared
by every uninstall on the device, and blocking anything but our own package would stop the user
managing their own phone, which is well outside what this tool may do.

#### Recents and split screen are real bypasses

Removing the app from the recents list can stop the accessibility service on several OEMs —
AppBlock documents this directly and tells users to pin the app in recents as a workaround.
Any design that guards only the settings screens has an unguarded path straight through recents.
Treat recents and split-screen entry as guarded surfaces in their own right, not as polish.

#### What this does and does not achieve

Blocking the disable path is effective against the case the product exists for: an impulsive
attempt to remove the guard, made in the moment, without preparation. That user does not get to
the toggle.

It does not survive a prepared user with a computer. Safe mode, `adb`, and factory reset all
remain, and none of them can be observed or obstructed from an accessibility service. That is
the correct ceiling — consistent with [mission.md](../../mission.md), the goal is to make removal
cost deliberate effort rather than a reflex, not to make it impossible.

The tamper log therefore remains, but as the **backstop for what gets through** rather than the
primary mechanism: it records what the guard could not prevent, and what happened while the
guard was off. Design it so entries survive the app being disabled, since that is precisely when
they matter.

#### Staying on the right side of a real line

Obstructing deactivation is the defining behaviour of Android stalkerware, and the more
aggressive variants of these techniques — locking the screen from `onDisableRequested`, trapping
the user in a back-loop — are documented as malware patterns. What separates this product is
that the user installs it on their own device, for themselves, and can always reach an
off-ramp. Keep it that way deliberately:

- Never obstruct deactivation beyond a warning and a record.
- Never make the device unusable, and never trap navigation (see the back-action bound below).
- Always keep an in-app disable path, even if it is delayed or gated.

A tool that cannot be removed is not an accountability tool.

#### The exit path — request, then release

An in-app exit is required rather than optional, and it ships *with* `SettingsGuard` rather than
after it. Until the per-OEM identifiers are verified on real hardware, a matcher that is wrong on
an untested build would otherwise leave the user locked out of their own settings with `adb` or
safe mode as the only recovery. It is the safety net for our own bugs before it is anything else.

**It must be a delayed release, not a switch.** The first version released the guard the instant
the button was tapped, which made the exit path the single fastest way *through* the guard: open
the app, tap release, tap the "Open accessibility settings" button sitting directly beneath it,
toggle off. Four taps, no research — quicker than every bypass the guard was built to close, and
shipped inside the product. Its own comment claimed "an impulse does not survive a wait" while
implementing no wait at all.

So the shape is:

- **Request → cooldown → short window.** `ReleaseSchedule` (pure, unit tested) holds the timing:
  15 minutes before a request opens, then 60 seconds of access. An unused request expires rather
  than staying armed, or one cooldown would buy unlimited later access.
- **The cooldown is the mechanism**, not a detail. A release arriving sooner than the urge fades
  is decoration. There is a test asserting the constant stays above ten minutes.
- **Timing is monotonic.** `SystemClock.elapsedRealtime()` only — never `currentTimeMillis()`.
  The wall clock is user-settable and Settings' date screen is not a guarded surface, so any
  wall-clock dependence would reduce the cooldown to "set the date forward an hour". A stored
  value greater than now means the device rebooted, and the request is voided rather than
  guessed at, which keeps the wall clock out of the decision entirely.
- **What is stored is a request, not a grant.** Clearing app data therefore removes a pending
  request and makes the guard stricter, never weaker — worth preserving deliberately, since
  clear-data lives on a screen we can only guard, not prevent.
- **The route to the toggle is hidden while the guard runs.** "Open accessibility settings" is
  shown only before the service is enabled or during an open window. Offering it beside the
  release button is what turned a considered exit into a four-tap bypass.

A partner-held handoff — where releasing requires someone else — is the stronger variant and the
natural successor. **Out of scope for now**; it needs the accountability channel that does not
yet exist, and the delayed release is what makes the guard shippable without it.

#### Identifying the settings screen

This section was rewritten after building it. The design that looked obvious on paper — match
the activity class, fall back to resource ids — does not work on AOSP, and the reasons are worth
recording because they will apply to every OEM added later.

**What was measured** on `android-36 google_apis arm64-v8a`:

| Signal | Reality |
|---|---|
| Node resource ids | **Useless here.** The accessibility screens expose only generic Settings chrome — `recycler_view`, `content_frame`, `collapsing_toolbar`, `app_bar` — identical on every sub-page including Wi-Fi. |
| `event.className` | Correct when present, but **not reliably delivered.** The activity class rides on `TYPE_WINDOW_STATE_CHANGED`; opening the accessibility list can produce nothing but content-changed events carrying `android.widget.FrameLayout`, leaving the screen unguarded. |
| Host activities | Generic. The page holding our own on/off switch is `com.android.settings.SubSettings`, and App Info is `com.android.settings.spa.SpaActivity` — both shared with unrelated pages. |
| **Our own app label** | **The signal that actually holds.** Language independent, present on every settings screen that concerns this app, and unaffected by which event type arrived. |

So the rule is inverted from the original plan: **the app's own label is the primary signal**, and
the activity class refines *which* surface it is when it happens to be available.

**Do not match on Settings' own copy.** A `contains("Accessibility")` check fails silently on any
device not in English — a failure that never appears when testing on one phone. Our label is a
brand string rather than localised copy, which is exactly why it survives this.

The cost is deliberate over-reach: every settings page naming this app is guarded, including our
notification and battery pages. All of them are removal-adjacent, and the back-out bound limits
what a false positive costs. Measured against ten unrelated settings screens — Wi-Fi, Bluetooth,
display, battery, security, date/time, storage, sound, another app's App Info, and the all-apps
list — none were blocked.

**Never infer an identifier.** Of six identifiers written from plausible-looking names in the
first draft, two were real. `InstalledAppDetailsTop` does not host App Info here;
`Settings$DeviceAdminSettingsActivity` is an alias that resolves to `com.android.settings/.Settings`,
so an entry for it would have matched every settings screen and locked the user out of their own
device. Dump each one:

```bash
adb shell dumpsys activity activities | grep topResumedActivity   # on the screen in question
adb logcat -s ScreenGuard | grep "settings screen"                # what the service actually sees
```

The service logs every settings screen it observes for exactly this purpose.

Identifiers are **data, not code** — a per-OEM table with per-device test cases — and an
unrecognised build is reported to the user as unverified rather than silently failing open.

#### Bounding the back action

Two bounds, both found by running it rather than by reasoning about it.

**Re-fire suppression.** A screen emits several window-state events while it renders — three in
~800 ms for the accessibility list. Firing `GLOBAL_ACTION_BACK` on each pops several levels of
the navigation stack instead of one, and spends the entire loop budget on a single visit. A
fired back action is given ~1.2 s to land before the same screen counts as a second attempt.

**Suppression must know when the user actually left, or it becomes the bypass.** The first
version keyed only on "same surface within the window", which meant backing out and tapping
straight back in landed inside the window and was ignored. The severe form is worse than a
1.2 s gap: the guard only evaluates when an accessibility event fires, so once a static screen
has finished rendering nothing wakes it again and the toggle stays reachable for as long as the
user leaves it open. `SettingsGuard.onUnguardedScreen()` must therefore be called for **every**
screen outside the settings app — the service calls it on the same early-return path that skips
harvesting — so that a return counts as a fresh arrival rather than a continuation.

A stronger variant is available and not yet taken: suppress on **event type** rather than
elapsed time, since the render burst is content-changed noise while a genuine arrival is a
window-state change. That removes the timing heuristic altogether and is the right follow-up if
this area is touched again.

**Consecutive-attempt bound.** If a matcher is over-broad on an untested build, or back does not
dismiss the window, the result is a loop that ejects the user from Settings entirely — including
from the App Info page they would need to uninstall us. After three real attempts the guard
degrades to cover-only, releasing navigation. When it trips, that round is lost; the value is
the record, not the cover.

#### Reference documents

- [`DeviceAdminReceiver`](https://developer.android.com/reference/android/app/admin/DeviceAdminReceiver) — and [`onDisableRequested`](https://developer.android.com/reference/android/app/admin/DeviceAdminReceiver#onDisableRequested(android.content.Context,%20android.content.Intent))
- [Device administration overview](https://developer.android.com/work/device-admin) — the admin/owner capability split
- [Device admin deprecation](https://developers.google.com/android/work/device-admin-deprecation) — what is dead and what still works
- [`AccessibilityNodeInfo#getViewIdResourceName`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo#getViewIdResourceName())
- [`AccessibilityServiceInfo#FLAG_REPORT_VIEW_IDS`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#FLAG_REPORT_VIEW_IDS)
- [`AccessibilityService#GLOBAL_ACTION_BACK`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService#GLOBAL_ACTION_BACK)

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
5. ~~`SettingsGuard` — back out of the Accessibility settings and our own App Info screens (§7),
   with unrecognised-device reporting, bounded back-action, and the timed in-app disable.~~
   **Done** — AOSP profile verified on an android-36 arm64 emulator: the accessibility list and
   our App Info are blocked consistently, ten unrelated settings screens are not, and the timed
   disable both releases the guard and resumes when it expires. Device admin identifiers were
   confirmed in step 6, once a receiver existed to open the screen with. Xiaomi profile still to
   be added.
6. ~~Device Admin — `DeviceAdminReceiver` for uninstall friction, plus an `onDisableRequested`
   warning. Plain admin only; no owner-only calls. Also the only way to verify the
   `DeviceAdminAdd` identifier, which cannot be reached until a receiver exists.~~ **Done** —
   uninstall refused (`DELETE_FAILED_DEVICE_POLICY_MANAGER`) on an android-36 emulator. The
   `DeviceAdminAdd` identifier is confirmed, and the screen turned out to need resource-id
   matching rather than the class; see §7.
7. **The empty harvest** ([backlog.md](backlog.md) item 1b) — the catch-all cannot fire on a tree
   with no text in it, which currently leaves the device admin list unguarded and silently
   weakens `mentionsSelf` on *any* screen that harvests empty. Evidence first: the
   `empty harvest` diagnostic in `ScreenGuardService` discriminates the causes, and the leading
   one is `flagIncludeNotImportantViews` being unset rather than anything to do with windows.
   Scoped deliberately to exclude split screen — an earlier pass treated the two as one bug on
   the strength of a stale backlog line, and they are not related.
8. Split-screen window resolution, then recents (§7 and [backlog.md](backlog.md) item 2) — the
   bypasses that go around step 5 rather than defeating it. Ranked after step 7 because it needs
   deliberate user intent, while the empty harvest needs none. Note `GLOBAL_ACTION_BACK` is
   global, so multi-window evaluation needs the action fixed before the matching is widened.
9. **Tamper log** — append-only local record of guard-state transitions and removal attempts.
   The backstop for what steps 5–8 cannot prevent (guest user, safe mode, adb); entries must
   survive the app being disabled.
10. Foreground service + restart-on-boot. **Note the real reason:** an `AccessibilityService`
   is system-bound and already restarts on boot while it stays enabled, so this neither makes
   the guard harder to kill nor is required for it to survive a reboot. What it provides is an
   always-visible status surface, a health check for removal routes we cannot observe, and the
   FGS host that the last two steps require. It may also reduce the recents-swipe kill in step 6.
11. `VpnService` DNS/SNI filter. Note that without `setAlwaysOnVpnPackage` (owner-only) the VPN
    can be turned off in Settings like anything else — guard that screen the same way.
12. `MediaProjection` capture once `image-sandbox` lands.

#### Reference documents — steps 7 and 8

- [`AccessibilityServiceInfo`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo) — the `accessibilityFlags` values, `FLAG_INCLUDE_NOT_IMPORTANT_VIEWS` and `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` among them
- [`FLAG_INCLUDE_NOT_IMPORTANT_VIEWS`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#FLAG_INCLUDE_NOT_IMPORTANT_VIEWS) — the step 7 candidate; read alongside [`importantForAccessibility`](https://developer.android.com/reference/android/view/View#attr_android:importantForAccessibility), which is what it overrides
- [`AccessibilityService.getWindows()`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService#getWindows()) and [`AccessibilityWindowInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityWindowInfo) — window enumeration for step 8, including `isActive`/`isFocused`
- [`GLOBAL_ACTION_BACK`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService#GLOBAL_ACTION_BACK) — note it takes no window argument, which is the hazard recorded in step 8
- [`AccessibilityNodeInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo) — `getChild`, `refresh`, and what each does and does not re-fetch
- [`UiAutomation.setServiceInfo`](https://developer.android.com/reference/android/app/UiAutomation#setServiceInfo(android.accessibilityservice.AccessibilityServiceInfo)) — why a `uiautomator` dump and a bound service can see different trees

## Gotchas learned the hard way

Each of these cost real time and none is discoverable by reading the API docs.

- **Never infer a settings identifier — dump it.** Of six written from plausible-looking names,
  two were real. `Settings$DeviceAdminSettingsActivity` is an *alias* resolving to
  `com.android.settings/.Settings`, so guarding it would have matched every settings screen and
  locked the user out of their own device.
- **`event.className` is not reliably delivered.** It rides on `TYPE_WINDOW_STATE_CHANGED`, and
  opening the accessibility list can produce only content-changed events carrying
  `android.widget.FrameLayout`. Class-only matching leaves the screen unguarded some of the time,
  which is why the app-label catch-all exists and is load-bearing.
- **Resource ids are useless on AOSP Settings.** Every sub-page exposes the same chrome.
- **A screen fires several window-state events while rendering** — three in ~800 ms. One
  `GLOBAL_ACTION_BACK` per event pops several stack levels and burns the whole loop budget.
- **Re-fire suppression must know when the user left**, or it becomes the bypass: back out, tap
  straight back in inside the window, and the guard idles. Worse, `evaluate` only runs on events,
  so a static screen that has finished rendering never wakes it again.
- **The exit path is the most dangerous code here.** An instant release beside an "open
  accessibility settings" button was a four-tap bypass shipped inside the product — faster than
  anything it was built to stop.
- **Timing that gates access must be monotonic.** Wall clock is user-settable and the date screen
  is not guarded.
- **Test harness trap:** `am start` reuses an existing task and silently shows the previous
  screen ("Activity not started, its current task has been brought to the front"), which reads as
  a guard failure. Remove tasks via `am stack list` + `am task remove` between cases.
  `--activity-new-task` is not a valid `am` option. And adb round-trips are slower than a
  sub-second suppression window, so some timing bugs cannot be reproduced through adb at all.
- **The NDK may not be under `$ANDROID_HOME`** — see the multi-root trap below.

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
- **Foreground-service category** — which `foregroundServiceType` Android 14+ will accept for
  a guard that is neither media nor location.
