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
  unit tested; **not yet run on a device.** SNI/IP filtering is still to come, and needs a
  userspace TCP stack — see step 11 below.
- `MediaProjection` capture + image path — not yet created.
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

### 5. `VpnService` — DNS filter — **Done**; SNI/IP still to come

Reuses net-shield's `DomainFilter` over the same FFI pattern as the text path
(`packages/net-shield-ffi`, generated by the same `scripts/build-ffi.sh`). No TLS termination —
see the rationale above.

**The split into DNS first and SNI/IP second is forced by the platform, not chosen for
convenience.** A `VpnService` TUN cannot re-inject a packet the way the Windows Wintun path can:
on Android the only way to *permit* a flow is to terminate it in userspace and re-originate it on
a `protect()`ed socket. For UDP that is a socket per flow, which the DNS path does. For TCP it is
a userspace TCP stack, which SNI and IP filtering both require and which is a step of its own
size. Until that stack exists the TUN must claim **one /32 route** — the resolver address it
advertises — because a wider route pulls in traffic there is nothing to forward it with, and the
device reads as offline rather than filtered. `NetworkGuard.ROUTE_PREFIX_LENGTH` is that constant
and it has a test whose only job is to notice it changing.

What the DNS layer buys: it is the step that decides whether a connection is attempted at all, so
one filter covers every app on the device. What it does not: an app speaking DNS-over-HTTPS to a
hardcoded endpoint never asks the system resolver. Android's Private DNS setting is one such
route and is a Settings screen, so it belongs to §7 rather than here.

Decisions worth not relitigating:

- **NXDOMAIN, not a sink address.** A refusal needs no answer section, so it is correct for every
  QTYPE at once — A, AAAA, and the SVCB/HTTPS records (RFC 9460 §14.1) modern clients ask for
  alongside them. An `A` record of `0.0.0.0` answers one of those and leaves the rest to resolve
  normally.
- **No fallback to a public resolver.** Permitted queries go to the DNS servers of the underlying
  non-VPN network. If there are none, they go unanswered until there are — sending this user's
  queries to a third party they never chose is not something to default to.
- **The forwarding path must exclude our own resolver address.** Once the TUN is up,
  `TUN_DNS_SERVER` *is* the system's advertised resolver, so any "read the current DNS servers"
  path hands back our own address and forwarding there writes the query straight back into the
  TUN it came from. Guarded twice: the `NetworkRequest` asks for `NET_CAPABILITY_NOT_VPN`, and
  `NetworkGuard.upstreamResolvers` drops our addresses whatever the system reports.
- **The shipped rule set is a placeholder.** `DnsGuard::with_builtin_rules` blocks one RFC 2606
  reserved name and nothing real, exactly as `text-policy-ffi` ships a placeholder dictionary. A
  blocklist of actual hostnames is not an artifact this repository carries;
  `with_blocked_domains` is the runtime route.

**Verified on an android-36 arm64 emulator** by `scripts/smoke-test-vpn.sh`: a listed name is
refused locally, an unlisted one resolves through the forwarder, the device stays online, the
interface is torn down and recorded when consent is lost, and the block lifts with it. Three
things the unit tests could not have caught, all found by that first run:

- **`ACCESS_NETWORK_STATE` was missing.** `registerNetworkCallback` throws `SecurityException`
  without it, *after* `establish()` has returned — so the TUN was up with a dead service behind
  it. Establishing a VPN and then crashing is worse than never establishing one.
- **`INTERNET` was missing** — this app had never needed network access before. `send()` then
  fails with `SocketException`, so permitted lookups go unanswered while blocked ones still
  resolve. **That failure shape looks exactly like the filter working**, which is why it survived
  a first pass: `tun0` was up, the blocked name was refused, and only checking a permitted name
  showed anything was wrong. Any future change here must assert both directions.
- **Android does not re-establish a `VpnService` after a reboot.** That is what
  `setAlwaysOnVpnPackage` is for and it is device-owner only, which this product deliberately is
  not. Measured: armed, VPN up, reboot, and `tun0` was gone with the blocked name resolving
  normally. `BootReceiver` now restores it alongside the status service.

One rough edge left, and it is benign: for a second or two after boot the guard is established
before the connectivity callback has handed it a resolver, and lookups in that window are logged
as `no upstream resolver` and go unanswered. The client's own resolver retries and it self-heals;
it is not worth holding the interface down for.

**A trap for anyone extending the smoke test.** netd answers a cached name without asking any
resolver, so a positive result cached before the VPN came up survives it coming up — the filter
never sees the query and `ping` still reports an address. `ndc resolver flushnetdns` fails
silently for a non-root shell, so the script reboots instead and runs the blocked check on a cold
cache. And assert on the *positive* form of `ping`'s output, never on an error string: `ping` has
several ways of saying a name did not resolve, and matching one of them makes the others read as
success — a test that passes when the guard is broken.

#### Reference documents

- [`VpnService`](https://developer.android.com/reference/android/net/VpnService) and
  [`VpnService.Builder`](https://developer.android.com/reference/android/net/VpnService.Builder) —
  note `addDnsServer` only takes effect for an address reachable through the VPN, so it and
  `addRoute` are one decision
- [`VpnService.protect`](https://developer.android.com/reference/android/net/VpnService#protect(java.net.DatagramSocket)) —
  what keeps the forwarding socket out of our own routes
- [`NET_CAPABILITY_NOT_VPN`](https://developer.android.com/reference/android/net/NetworkCapabilities#NET_CAPABILITY_NOT_VPN) —
  how the underlying network's resolvers are found without asking the VPN about itself
- [RFC 1035](https://www.rfc-editor.org/rfc/rfc1035) §4.1.1 (header), §4.1.2 (question),
  §4.1.4 (compression), §2.3.4 (size limits) — the DNS message format
- [RFC 4343 §3](https://www.rfc-editor.org/rfc/rfc4343#section-3) — DNS names match
  case-insensitively, which is why the filter lowercases
- [RFC 6891 §6.2.5](https://www.rfc-editor.org/rfc/rfc6891#section-6.2.5) — EDNS0 payload sizes,
  which set the upstream read buffer
- [RFC 9460 §14.1](https://www.rfc-editor.org/rfc/rfc9460#section-14.1) — the HTTPS RR type
- [RFC 791 §3.1](https://www.rfc-editor.org/rfc/rfc791#section-3.1) — IPv4 header, including the
  fragment flags the parser refuses
- [RFC 768](https://www.rfc-editor.org/rfc/rfc768) — UDP header and the checksum pseudo-header
- [RFC 1071](https://www.rfc-editor.org/rfc/rfc1071) — the one's-complement checksum
- [RFC 6864 §4.1](https://www.rfc-editor.org/rfc/rfc6864#section-4.1) — why the Identification
  field of a datagram that is never fragmented may be zero
- Still ahead, for SNI/IP: [RFC 6066 §3](https://www.rfc-editor.org/rfc/rfc6066#section-3) — TLS
  SNI extension, and [RFC 8446](https://www.rfc-editor.org/rfc/rfc8446) — TLS 1.3 ClientHello

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

**Split screen is handled** (step 8): every watched window is evaluated rather than one screen,
and the back action is gated on the matched window holding input focus, because
`GLOBAL_ACTION_BACK` takes no window argument and would otherwise be delivered to whatever app the
user is actually in. Read [backlog.md](backlog.md#closed-2-split-screen) before extending it — an
unfocused pane emits no accessibility events at all, so what carries the protection is that the
toggle needs focus and taking focus emits events, not the cover. **Recents is deferred** — see
[Recents is blocked on hardware](#recents-is-blocked-on-hardware).

#### Recents is blocked on hardware

**Deferred, and it is a blocker rather than a choice.** The claim the mitigation would be built on
— that swiping the app out of recents stops the accessibility service — comes from AppBlock's own
documentation, and AppBlock ships to OEMs with aggressive task-killers that AOSP does not have.
On the one platform available here, an `android-36` emulator, a bound accessibility service is
**not** killed by a recents swipe, so the emulator cannot reproduce the bypass and cannot verify a
fix for it either. Writing a guard against an unreproduced behaviour would produce code that looks
like coverage and is never exercised.

What it needs is real One UI or HyperOS hardware — the same dependency as the rest of
[OEM coverage](#oem-coverage--deferred), and Samsung Remote Test Lab is the route. Until then:

- The tamper log (step 9) is the honest response. A service that stops without a clean unbind is
  observable after the fact even where it cannot be prevented, and the boot receiver in step 10
  turns "the guard did not run" into a recorded gap rather than silence.
- Do not write onboarding copy implying recents is covered.

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

#### Protection is a mode the user turns on

**The guard blocks because the user armed it, not because the service is running.** This is the
governing rule of the whole module, and it is what separates the product from the stalkerware
pattern the techniques resemble: nothing about a user's own phone is obstructed until they ask for
it, and asking to stop is a supported operation rather than an attack to be defeated. The intended
way to uninstall this app is through the front door — disarm, then remove it.

`ProtectionSchedule` (pure, unit tested) holds the whole state machine; `ProtectionStore` is the
`SharedPreferences` edge; `SettingsGuard.setGuardActive` is the one input the guard takes from it.
The phases:

| Phase | Guard | How it is left |
|---|---|---|
| `OFF` | idle | the user arms it — one tap |
| `ARMED` | blocking | the user requests a disarm |
| `DISARM_PENDING` | **still blocking** | 15-minute cooldown elapses, or the user cancels |
| `DISARM_READY` | **still blocking** | the user confirms, or the offer expires after 5 minutes |
| `DISARMED` | idle | the 10-minute window runs out and it re-arms itself |

Each of those is load-bearing:

- **Arming is one tap; disarming costs the cooldown.** Protecting yourself must never be the slow
  direction.
- **The cooldown is the mechanism**, not a detail. An urge does not survive fifteen minutes; a
  decision does. A test asserts the constant stays above ten minutes.
- **Reaching the end of the cooldown does not disarm anything.** The user has to come back and
  confirm, otherwise a request made and forgotten spends its window in a pocket and the cooldown
  bought nothing. An unconfirmed request expires, or one cooldown paid once would buy a disarm at
  any point in the future.
- **A disarm is a window, not a switch.** Ten minutes: long enough to deactivate device admin,
  open App info and uninstall without racing a clock, short enough that a user who gets distracted
  ends up protected again. The re-arm needs no action, which is the point.
- **Timing is monotonic.** `SystemClock.elapsedRealtime()` only — never `currentTimeMillis()`.
  The wall clock is user-settable and Settings' date screen is not a guarded surface, so any
  wall-clock dependence would reduce the cooldown to "set the date forward an hour". Both stored
  values are *start* timestamps rather than deadlines, so a stored value in the future means the
  device rebooted; both such cases resolve to `ARMED`. That direction matters for the disarm
  window in particular — a stored deadline read against a clock that just reset to zero would look
  like a disarm with hours left on it, making a reboot a way to stay unguarded indefinitely.
- **What is stored is a request, not a grant.** Clearing app data removes a pending request and
  leaves the mode armed, so it makes the guard stricter, never weaker — worth preserving
  deliberately, since clear-data lives on a screen we can only guard, not prevent.
- **The route to the toggle is hidden while the guard is active.** "Open accessibility settings"
  appears only when the service is off or protection is not blocking. An earlier version of this
  screen released the guard on the tap *and* offered that button directly beneath it: open the
  app, tap release, tap through, toggle off. Four taps, no research — quicker than every bypass
  the guard was built to close, and shipped inside the product. Its own comment claimed "an
  impulse does not survive a wait" while implementing no wait at all.
- **One button, whose meaning follows the phase**, so there is never a live "turn it off now"
  control sitting beside an armed guard. Cancel is a separate button for the same reason.

The mode gates `SettingsGuard` only. Content scanning and the cover keep running while the service
is enabled — that is the product's actual job, and turning *it* off is what disabling the
accessibility service does.

**The stored timestamps outlive their windows, and that bit the first version.** A confirmed
disarm was checked ahead of a pending request unconditionally, so once its window had expired the
spent timestamp swallowed every later request: the phase fell straight back to `ARMED`, the
countdown never appeared, and the app could be disarmed exactly once per install. That is the
worst failure available to this module — the mode is the only supported way to remove the app, so
a second attempt would have met a tool that genuinely could not be uninstalled. A live disarm
outranks a request; a spent one falls through. Found by running the cycle twice on an emulator,
not by reading the code, which is the argument for exercising the whole cycle rather than each
phase in isolation.

**Verifying it on a device: shorten the constants, do not wait out the clock.** A run against the
real values takes 15 minutes to reach the confirm step and another 10 to see the re-arm, which is
slow enough that it does not get repeated — and this is a bug that only appears on the *second*
cycle. Patch `COOLDOWN_MILLIS`/`READY_WINDOW_MILLIS`/`DISARM_WINDOW_MILLIS` down to seconds, run
the full cycle twice, then restore them; the floor assertions in `ProtectionScheduleTest` fail
while they are patched, which is what stops a shortened constant reaching a commit. Editing
`SharedPreferences` directly is *not* an alternative — the service holds them in memory, and
force-stopping the app to reload them disables its own accessibility service.

A partner-held handoff — where disarming requires someone else — is the stronger variant and the
natural successor. **Out of scope for now**; it needs the accountability channel that does not
yet exist, and the delayed disarm is what makes the guard shippable without it.

#### Some nodes are withheld from the service entirely

The screens this guard exists to watch are the same ones Android hardens against accessibility
abuse, and one of those defences applies to us. `View#setAccessibilityDataSensitive` (Android 14)
marks a node as sensitive, and the framework then withholds it from every service whose
`AccessibilityServiceInfo.isAccessibilityTool()` is false. AOSP Settings marks the **device-admin
list rows** this way.

The failure mode is silent and looks like nothing at all: the tree arrives with the screen's
chrome and an empty `recycler_view`, the harvest yields no text, `mentionsSelf` cannot fire, and
the screen gating uninstall is simply not guarded. Nothing errors, and nothing in the node tree
says a node was removed — a filtered child does not appear in `childCount`, so the subtree reads
as genuinely absent.

`android:isAccessibilityTool="true"` in `accessibility_service_config.xml` is what restores it,
and it is therefore load-bearing rather than a label. Two consequences worth keeping in view:

- **It changes how Android presents the service**, and it is a claim about what the service is.
  This product is an accessibility-service-based tool that the user installs on their own device
  for themselves; the declaration is not a workaround for a permission that was withheld from us.
  Distribution is by sideloading, so no store review turns on it either way.
- **Any screen can be marked sensitive**, on any build. A thin harvest on an OEM build is now a
  known shape rather than a mystery, and `ScreenGuardService.logEmptyHarvest` is kept for exactly
  that. See [backlog.md](backlog.md#closed-1-the-empty-harvest) for the measurements.

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

### 8. `GuardStatusService` — the foreground status surface — **Done**

**It is not what keeps the guard alive, and no copy about it should imply otherwise.** An
`AccessibilityService` is bound by the system and is rebound after a reboot for as long as it
stays enabled, so a foreground service makes the guard neither harder to kill nor able to survive
anything it could not survive already. What it adds is a process that is still running *after* the
guard stops:

- **It closes the "still-alive process" half of the detection surface** in
  [backlog.md](backlog.md), "Cannot be closed at Device Admin level". An `adb` disable, a guest
  session, or a disable screen an OEM build hides from the guard all end with the accessibility
  service gone and nothing written by it — by the time the fact is true, the component that would
  record it has been unbound. The status service polls the same secure setting
  `AccessibilityServiceStatus` parses, and a `ContentObserver` on it makes the observation
  immediate rather than up to a poll interval late. Measured: an `adb` disable was recorded inside
  the same second.
- **It is the FGS host steps 10 and 11 need.** `VpnService` and `MediaProjection` both require
  one, and the `MediaProjection` start order (§6) is strict.
- **It is the status surface.** One ongoing, silent notification saying which of *armed*,
  *running* and *recognised device* is actually true — the three disagree routinely, and that
  disagreement is the only thing worth a permanent notification.

`GuardStatus` holds all of it: health, the message priority, and whether the service still has
anything to report. `GuardHealth.UNPROTECTED` — armed with nothing enforcing it — is the only state
that reaches the tamper log, written on the **edge** rather than on each poll, since this is a
timer and an evening with the service off would otherwise push the history that matters past the
cap. A release window is `IDLE`, not a fault: turning the service off during one is the front door.

**The service stops itself once protection is off and the guard is not running**, and declines to
start in that state at all, so an install that never gets past onboarding does not acquire a
permanent notification.

Four things were measured while building it, none of them in the docs:

- **A denied `POST_NOTIFICATIONS` is invisible, not loud.** The service runs, `startForeground`
  succeeds, and the notification is simply never posted — which for a service whose entire output
  is that notification reads exactly like the service failing to start. `dumpsys notification`
  shows `importance=NONE` for the package, and that line is the only evidence. `MainActivity` asks
  on Android 13+; a refusal is not handled, because the durable record is the tamper log either
  way.
- **The mode has nothing to announce a change.** It lives in `SharedPreferences`, so arming left
  the notification claiming protection was off for up to a poll interval — on the one surface whose
  job is to say what is true right now. `MainActivity.refresh()` re-starts the service, which
  delivers `onStartCommand` and re-checks immediately; a resumed activity is also the one call site
  the foreground-start restrictions never refuse.
- **`settings put secure enabled_accessibility_services ""` is rejected on API 36** with "Bad
  arguments". Use `settings delete secure enabled_accessibility_services` to reproduce the `adb`
  disable.
- **`ACTION_MY_PACKAGE_REPLACED` fires on an ordinary `adb install -r`**, and the update kills the
  process without an unbind — so every reinstall during testing writes an unclean stop. That is
  correct, and it means the log from a development session is not a clean sample.

**`specialUse` is the foreground-service type**, which answers the open question this plan carried.
None of the Android 14+ typed categories describes "watches whether this app's own guard is still
running": it is not media, location, or data sync. `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` in the
manifest is the required justification, and Play review is not a factor here — distribution is by
sideloading and Play is an explicit non-goal.

**The boot receiver writes nothing**, and that is the whole design of it. See step 10 below.

#### Reference documents

- [Foreground services](https://developer.android.com/develop/background-work/services/fgs) — including the five-second `startForeground` deadline
- [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) and [`specialUse`](https://developer.android.com/develop/background-work/services/fgs/service-types#special-use)
- [Background start restrictions](https://developer.android.com/develop/background-work/services/foreground-services#background-start-restrictions) — the exemption list that lets `BOOT_COMPLETED` start one
- [`POST_NOTIFICATIONS`](https://developer.android.com/develop/ui/views/notifications/notification-permission)
- [`ContentObserver`](https://developer.android.com/reference/android/database/ContentObserver) and [`Settings.Secure#getUriFor`](https://developer.android.com/reference/android/provider/Settings.Secure#getUriFor(java.lang.String))
- [`NotificationChannel`](https://developer.android.com/reference/android/app/NotificationChannel) — importance, and why `IMPORTANCE_LOW` is right for an ongoing status

#### Reference documents — step 7

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
   with unrecognised-device reporting, bounded back-action, and the in-app disable.~~
   **Done** — AOSP profile verified on an android-36 arm64 emulator: the accessibility list and
   our App Info are blocked consistently, ten unrelated settings screens are not. Device admin
   identifiers were confirmed in step 6, once a receiver existed to open the screen with. Xiaomi
   profile still to be added. The disable path was later reshaped into the protection mode (§7):
   the guard blocks only while the user has armed it, and disarming is the timed operation.
6. ~~Device Admin — `DeviceAdminReceiver` for uninstall friction, plus an `onDisableRequested`
   warning. Plain admin only; no owner-only calls. Also the only way to verify the
   `DeviceAdminAdd` identifier, which cannot be reached until a receiver exists.~~ **Done** —
   uninstall refused (`DELETE_FAILED_DEVICE_POLICY_MANAGER`) on an android-36 emulator. The
   `DeviceAdminAdd` identifier is confirmed, and the screen turned out to need resource-id
   matching rather than the class; see §7.
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
12. `MediaProjection` capture once `image-sandbox` lands.
13. SNI/IP filtering in the VPN, which needs the userspace TCP stack §5 describes. Reuses
    net-shield's `extract_sni` and `IpFilter` over the FFI surface step 11 established.

#### Reference documents — steps 7 and 8

- [`AccessibilityServiceInfo`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo) — the `accessibilityFlags` values, `FLAG_INCLUDE_NOT_IMPORTANT_VIEWS` and `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` among them
- [`AccessibilityServiceInfo#isAccessibilityTool()`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#isAccessibilityTool()) and the [`android:isAccessibilityTool`](https://developer.android.com/reference/android/R.attr#isAccessibilityTool) attribute — what step 7 turned on, and read alongside [`View#setAccessibilityDataSensitive`](https://developer.android.com/reference/android/view/View#setAccessibilityDataSensitive(int)) and [`AccessibilityNodeInfo#isAccessibilityDataSensitive()`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo#isAccessibilityDataSensitive()), which are the filtering it exempts the service from
- [`FLAG_INCLUDE_NOT_IMPORTANT_VIEWS`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#FLAG_INCLUDE_NOT_IMPORTANT_VIEWS) — the step 7 candidate that turned out **not** to be the cause; read alongside [`importantForAccessibility`](https://developer.android.com/reference/android/view/View#attr_android:importantForAccessibility), which is what it overrides
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
- **A node can be withheld from the service and leave no trace.** `accessibilityDataSensitive`
  nodes are not reported to a service without `isAccessibilityTool`, and they do not appear in
  `childCount` either, so the subtree looks genuinely absent rather than filtered. This cost the
  most time of anything here: it presents as a rendering-lag bug, and no amount of waiting,
  `refresh()`, or `clearCache()` moves it.
- **A `uiautomator` dump is not evidence of what the service can see.** `UiAutomation` sets its
  own service info, `isAccessibilityTool` among the differences. "The dump shows the row" is
  compatible with the row being permanently invisible to us, and an investigation built on that
  comparison was chasing the wrong thing for a long time.
- **An event from another package is not the user leaving.** SystemUI fires content-changed events
  for the status bar over whatever is in front; treating one as a departure cancelled a guarded
  screen's whole re-look budget ~200 ms after it opened.
- **`adb shell am start` needs `\$` escaped inside a quoted argument.** The *device* shell expands
  it, so `Settings$DeviceAdminSettingsActivity` silently becomes `.Settings` and launches the
  Settings homepage — which reads exactly like a guard that failed to fire on the right screen.
- **A screen fires several window-state events while rendering** — three in ~800 ms. One
  `GLOBAL_ACTION_BACK` per event pops several stack levels and burns the whole loop budget.
- **Re-fire suppression must know when the user left**, or it becomes the bypass: back out, tap
  straight back in inside the window, and the guard idles. Worse, `evaluate` only runs on events,
  so a static screen that has finished rendering never wakes it again.
- **`ACTION_BOOT_COMPLETED` is not evidence of a boot.** It is delivered when the app leaves the
  force-stopped state — reproduced twice on an android-36 emulator with three minutes of uptime,
  `BootReceiver` logging "boot completed" seconds after an `am force-stop`. Anything inferring a
  restart from it will read a force-stop as a reboot, which is backwards: a force-stop is the
  event worth recording and a reboot is the benign one.
- **A denied `POST_NOTIFICATIONS` fails silently.** The foreground service still runs and
  `startForeground` still succeeds; the notification is just never posted, which is
  indistinguishable from the service not starting. `dumpsys notification | grep <package>` showing
  `importance=NONE` is the only tell.
- **`settings put secure enabled_accessibility_services ""` is rejected on API 36** ("Bad
  arguments"). `settings delete secure enabled_accessibility_services` is how to reproduce the
  `adb` disable.
- **`adb install -r` is not a neutral update.** It delivers `ACTION_MY_PACKAGE_REPLACED` and kills
  the process with no unbind, so every reinstall writes an unclean stop into the tamper log.
- **Force-stopping the app disables its own accessibility service.** `adb shell am force-stop`
  on our package is not a neutral way to restart the UI: the service is dropped from
  `enabled_accessibility_services` and every subsequent guard check silently passes. It reads
  exactly like the emulator clobbering the setting, which is a real and separate problem.
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
  Two more, both hit while verifying the protection mode: `am start` on an activity that is
  already top-most delivers the intent without resuming it, so `onResume` never runs and a
  scripted check reads stale UI (press HOME first); and tapping a control by its visible text can
  hit the *status line* instead, because the copy above the button contains the button's own
  wording — a confirm that silently taps a `TextView` looks exactly like a confirm that does not
  work. Match on `class="android.widget.Button"`, not on text alone.
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
- ~~**Foreground-service category**~~ — answered in step 10: `specialUse`, with
  `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` as the justification. None of the typed categories fits, and
  Play review is not a factor for a sideloaded app.
