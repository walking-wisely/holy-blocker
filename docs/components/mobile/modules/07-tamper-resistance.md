# 7. Tamper resistance — partially built

Part of the [Mobile plan](../plan.md).

## Device Owner is not available to this product

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

## What Device Admin still buys

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

## The accessibility service is the enforcement mechanism

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

## Recents and split screen are real bypasses

Removing the app from the recents list can stop the accessibility service on several OEMs —
AppBlock documents this directly and tells users to pin the app in recents as a workaround.
Any design that guards only the settings screens has an unguarded path straight through recents.
Treat recents and split-screen entry as guarded surfaces in their own right, not as polish.

**Split screen is handled** (step 8): every watched window is evaluated rather than one screen,
and the back action is gated on the matched window holding input focus, because
`GLOBAL_ACTION_BACK` takes no window argument and would otherwise be delivered to whatever app the
user is actually in. Read [backlog.md](../backlog.md#closed-2-split-screen) before extending it — an
unfocused pane emits no accessibility events at all, so what carries the protection is that the
toggle needs focus and taking focus emits events, not the cover. **Recents is deferred** — see
[Recents is blocked on hardware](#recents-is-blocked-on-hardware).

## Recents is blocked on hardware

**Deferred, and it is a blocker rather than a choice.** The claim the mitigation would be built on
— that swiping the app out of recents stops the accessibility service — comes from AppBlock's own
documentation, and AppBlock ships to OEMs with aggressive task-killers that AOSP does not have.
On the one platform available here, an `android-36` emulator, a bound accessibility service is
**not** killed by a recents swipe, so the emulator cannot reproduce the bypass and cannot verify a
fix for it either. Writing a guard against an unreproduced behaviour would produce code that looks
like coverage and is never exercised.

What it needs is real One UI or HyperOS hardware — the same dependency as the rest of
[OEM coverage](../plan.md#oem-coverage--deferred), and Samsung Remote Test Lab is the route. Until then:

- The tamper log (step 9) is the honest response. A service that stops without a clean unbind is
  observable after the fact even where it cannot be prevented, and the boot receiver in step 10
  turns "the guard did not run" into a recorded gap rather than silence.
- Do not write onboarding copy implying recents is covered.

## What this does and does not achieve

Blocking the disable path is effective against the case the product exists for: an impulsive
attempt to remove the guard, made in the moment, without preparation. That user does not get to
the toggle.

It does not survive a prepared user with a computer. Safe mode, `adb`, and factory reset all
remain, and none of them can be observed or obstructed from an accessibility service. That is
the correct ceiling — consistent with [mission.md](../../../mission.md), the goal is to make removal
cost deliberate effort rather than a reflex, not to make it impossible.

The tamper log therefore remains, but as the **backstop for what gets through** rather than the
primary mechanism: it records what the guard could not prevent, and what happened while the
guard was off. Design it so entries survive the app being disabled, since that is precisely when
they matter.

## Staying on the right side of a real line

Obstructing deactivation is the defining behaviour of Android stalkerware, and the more
aggressive variants of these techniques — locking the screen from `onDisableRequested`, trapping
the user in a back-loop — are documented as malware patterns. What separates this product is
that the user installs it on their own device, for themselves, and can always reach an
off-ramp. Keep it that way deliberately:

- Never obstruct deactivation beyond a warning and a record.
- Never make the device unusable, and never trap navigation (see the back-action bound below).
- Always keep an in-app disable path, even if it is delayed or gated.

A tool that cannot be removed is not an accountability tool.

## Protection is a mode the user turns on

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

## Some nodes are withheld from the service entirely

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
  that. See [backlog.md](../backlog.md#closed-1-the-empty-harvest) for the measurements.

## Identifying the settings screen

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

## Bounding the back action

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

## Reference documents
