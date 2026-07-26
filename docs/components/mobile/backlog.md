# Mobile — guard hardening backlog

Findings from an adversarial review of `SettingsGuard` and the surrounding guard code,
verified against the source and (where marked) reproduced on an `android-36` emulator.

The threat model is the one in [plan.md](plan.md) §7: the adversary is the phone's owner,
mid-craving, with unlimited physical access. The bar is not "unbypassable" — it is that removal
costs deliberate, sustained effort rather than a reflex. Items are ranked by
*likelihood a real user finds it* × *how little effort it takes*.

Anything requiring Device Owner is out of scope permanently — see plan.md §7.

## Done

- ~~**Suspend button was the bypass.**~~ Four taps, no research: open the app, tap the release
  button, tap "Open accessibility settings" directly beneath it, toggle off. The release was
  instantaneous and infinitely re-armable. **Fixed** — request-then-release with a cooldown, on
  a monotonic clock. See [plan.md](plan.md) §7.
- ~~**Uninstall dialog was unwatched.**~~ Reproduced on device: launcher long-press → Uninstall
  lands in `com.google.android.packageinstaller`, which `watchesPackage` did not cover, so the
  guard never saw it. **Fixed** — installer packages are watched on a self-mention-only path.
- ~~**`DeviceAdminReceiver` — the other half of the uninstall fix.**~~ **Built** — `adb uninstall`
  now returns `DELETE_FAILED_DEVICE_POLICY_MANAGER` while the admin is active, verified on an
  android-36 emulator. The `DeviceAdminAdd` entry it was blocking is confirmed as a real class,
  but is **not** matched by class: the screen arrives as `android.widget.FrameLayout` and the
  activity class only lands on a later, unreliable event. Matching moved to the
  `admin_name` / `add_msg` / `admin_warning` resource ids, and `SettingsGuard` now exempts that
  surface while the admin is inactive — without which the guard backed the user out of the only
  screen that can turn the admin on. See [plan.md](plan.md) §7.

- ~~**A guarded screen sat unguarded because the harvest came back empty.**~~ **Fixed** — the
  cause was neither of the two candidates this item was built around. The device-admin list rows
  are marked `accessibilityDataSensitive` (Android 14, `View#setAccessibilityDataSensitive`), and
  the framework withholds such nodes from every service that does not declare
  `android:isAccessibilityTool="true"`. Declaring it in `accessibility_service_config.xml` is the
  whole fix. See [the closed investigation](#closed-1-the-empty-harvest) below for the evidence
  and for what was ruled out, and plan.md §7 for what the declaration means.

- ~~**Split screen harvested the wrong window.**~~ **Fixed** — every watched window is evaluated
  now, not the first one matching the event's package, and the back action is gated on the matched
  window holding input focus. Verified in real split screen on an android-36 emulator, both
  directions. The premise was right and incomplete: the wrong-window read was real, but the
  reason it mattered most turned out to be the *action*, not the matching. See
  [the closed investigation](#closed-2-split-screen) for what the emulator showed, including two
  things the item did not predict.

- ~~**`OverlayController` could crash the service.**~~ **Fixed** — `addView`/`removeView` are
  wrapped, and a failed `addView` leaves `shownState` at `CLEAR` so the next event retries instead
  of believing a cover is up that is not. Not reproduced on device; this is a hardening fix
  written from the API contract, and it is untested for the same reason the throw is hard to
  provoke deliberately.

## Next

### 1. The device-admin list is unreachable from Settings while armed but not yet an admin

Introduced by the fix above and verified on an `android-36` emulator: now that the list's rows are
visible, the list names this app, so the `SELF_IN_SETTINGS` catch-all ejects the user from it —
including while the admin is *off*, when there is nothing on that screen to protect.

**The protection mode shrank this to a corner.** Nothing is blocked until the user arms
protection, so ordinary setup — enable the service, activate the admin, then arm — never meets it.
What remains is the state where protection is armed but device admin is not yet active, and the
user tries to activate it from Settings rather than from the app's own button.

`SettingsProfiles.AOSP` has an entry for the list's own alias class
(`com.android.settings.Settings$DeviceAdminSettingsActivity`, dumped, not inferred) so the
"only once the admin is active" exemption reaches it. That entry is correct but does not close
the case: the first event for the screen carries `android.widget.FrameLayout`, the catch-all
fires on it, and the back-out has already happened by the time a class-carrying event arrives.
This is the same failure that forced resource-id matching for `DeviceAdminAdd` — but unlike that
screen, the list exposes only generic Settings chrome (`content_parent`, `collapsing_toolbar`,
`recycler_view`), so the same remedy is not available.

**Not blocking, and deliberately left open.** Activation from the app's own onboarding button is
unaffected — verified end to end on device with the guard running: the prompt opens, activation
succeeds, and `adb uninstall` then returns `DELETE_FAILED_DEVICE_POLICY_MANAGER`. The rule in
plan.md §7 is that the feature must remain enable-able, and it does; what is lost is a secondary
route to a screen that does nothing while the admin is off.

Do **not** fix this by exempting `SELF_IN_SETTINGS` whenever the admin is inactive. That is
precisely the pre-onboarding state, and the accessibility list — the primary removal route, which
has nothing to do with device admin — would be unguarded throughout it.

### 2. `app_name` must never be localised

The catch-all matches on the app's own label, which works only because it is a brand string.
There is currently one `res/values/` directory, so the reasoning holds — but adding any
translated `strings.xml` would localise `app_name` and silently degrade the matcher in that
locale, reintroducing from our own side the exact failure §7 warns about for Settings' copy.

Fix: `translatable="false"`, or a dedicated non-localised match constant, plus a test asserting
the matcher label is locale-independent.

### 3. SystemUI is entirely unwatched

`watchesPackage` covers Settings and (now) the installers. `com.android.systemui` hosts Quick
Settings and the accessibility floating panel. On AOSP the a11y panel toggles the *shortcut
assignment* rather than the service enable-state, so it is probably not a direct disable route
on the verified target — but the package is invisible to the guard on builds we have no data
about, and Xiaomi and Samsung both customise SystemUI heavily.

Fix: watch it on a **self-mention-only** path, and **cover rather than back out**. Backing out of
the notification shade or volume panel is indistinguishable from a broken phone.

### 4. Recents — **deferred, blocked on hardware**

The recents-swipe kill cannot be reproduced on the only platform available here (an `android-36`
emulator does not kill a bound accessibility service on a swipe), so a mitigation could be neither
verified nor exercised. It needs real One UI or HyperOS hardware; Samsung Remote Test Lab is the
route. See [plan.md](plan.md#recents-is-blocked-on-hardware) for the full argument, and treat this
as sitting alongside the three bypasses below rather than above them — the tamper log is the
available response.

### 5. Dead `ScanGate.reset()`

Never called — `onServiceConnected` builds a fresh instance. Either call it on reconnect instead
of reallocating, or delete it.

`SettingsGuard.reset()` is **gone**, which was the half of this that carried a real hazard: it
cleared the suspension, so wiring it to a reconnect would have silently cancelled a live release.
That failure cannot recur — the mode now lives in `ProtectionStore` rather than in the guard, so
a reconnected guard reads the user's actual state instead of losing it.

## Cannot be closed at Device Admin level

Recording these is the only available response, and it is the argument for the tamper log.
Do not invent mitigations; do not write copy implying they are covered.

| Bypass | Effort | Note |
|---|---|---|
| **Guest / secondary user** | 5 taps | Accessibility services are per-user; the guard does not exist there. Does not remove the app, but completely defeats the craving. `DISALLOW_USER_SWITCH` is owner-only. **Say this plainly in onboarding.** |
| **Safe mode** | 4 taps, tutorial-tier | Third-party services do not run. `DISALLOW_SAFE_BOOT` is owner-only. A `BOOT_COMPLETED` receiver would let a boot with no service-connect be inferred as a safe-mode session. |
| **adb** | needs a computer | `settings put secure enabled_accessibility_services ""` disables instantly with no UI. Arguably *correct* that this works — it is exactly the deliberate, sustained effort removal is supposed to cost. |

The realistic detection surface for all three is a still-alive process noticing the change
(`AccessibilityServiceStatus` already parses the setting) plus `onDisableRequested`, writing to a
tamper log whose entries survive the app being disabled.

**Both halves are now built.** The log (plan.md step 9) records `onDisableRequested`, and a session
that ended without an unbind is classified as an unclean stop at the next connect — verified
against a force-stop, which is the shape of both the process kill and the OEM recents swipe. The
*still-alive process* half is `GuardStatusService` (step 10): it polls the same secure setting,
watches it through a `ContentObserver`, and writes `guard_unprotected` the moment protection is
armed with nothing enforcing it. An `adb` disable was recorded inside the same second on an
android-36 emulator, rather than at whatever later point the guard happened to be re-enabled.

Two limits worth stating plainly. A **guest session** is still only visible as an absence — the
service runs per user, so nothing of ours is alive in the other one to observe anything. **Safe
mode** remains invisible in both halves for the same reason: nothing of ours runs there to notice
or to record.

## Checked and found not to be holes

Recorded so they are not re-investigated.

- **Clearing app data** is not a release bypass — it clears the request, which makes the guard
  stricter. This stays true only while the stored state is a *request* rather than a grant.
- **Tripping the back-out bound** is not a walk-in. `CoverOnly` still shows the opaque,
  touch-swallowing cover, and `lastSurface` is a single field, so alternating surfaces resets the
  counter rather than accumulating. The alternating-surfaces half of this held only because one
  screen was evaluated per event; `evaluate` now takes the whole window list and folds it to a
  single decision before touching the counter, which is what keeps it true in split screen.
- **Settings' search box** collapses into the existing window-state-event reliability problem
  rather than being a distinct route: results naming the app hit the catch-all, and tapping
  through lands on a guarded activity.
- **The volume-key accessibility shortcut** invokes an assigned service's action; it does not
  toggle the enable-state. Assigning it happens on the guarded per-service page.
- **A boot receiver is not needed for the guard to survive reboot** — the system rebinds enabled
  accessibility services automatically. It is still worth adding for tamper-log gap detection.

## Closed: 2, split screen

Measured on an `android-36 google_apis arm64-v8a` emulator in genuine split screen — Settings
parked on the Accessibility page in one pane, a clock app in the other — with a temporary probe
logging the whole window list on every event.

**The window list is fine; the action was the problem.** `getWindows()` reports both panes
truthfully:

```
id=135 pkg=com.google.android.deskclock active=true  focused=true  type=1
id=147 pkg=com.android.settings         active=false focused=false type=1
```

So `isFocused` is a real signal and not something that has to be inferred. Both branches then
behaved as intended: unfocused reads `window=222 focused=false` and covers, and the instant the
pane is tapped it reads `focused=true` and is backed out — matching `ACCESSIBILITY_SETTINGS`
**by class**, which is also the proof that the event's class is being attached to the event's own
window rather than to a sibling pane.

Two things the item did not predict, both of which matter more than the wrong-window read did:

- **An unfocused pane emits no accessibility events at all.** Not delayed — none. A config change
  (`cmd uimode night`, `font_scale`) produced events from the focused app and from SystemUI and
  nothing whatsoever from the pane sitting beside them. The only way to make it emit was to drag
  the split divider, forcing a re-layout. The consequence is that the cover-when-unfocused branch
  is **defence in depth, not the load-bearing path** — the guard cannot see a quiet pane. What
  carries the protection is that the toggle is not operable without focus, and taking focus emits
  events, which is the case that gets backed out.
- **A focused pane's events were cancelling the re-look for the guarded pane.** `foregroundPackage`
  returned the clock app, so `classifyUnwatchedEvent` read every one of its events as the user
  having left. Fixed by passing the visible-window packages alongside the foreground: a watched
  window visible anywhere means `STILL_WATCHED`. This is the same shape as the SystemUI-chrome bug
  closed in investigation 1, one layer out.

Also observed, and deliberately not "fixed": the cover flaps. It goes up when the unfocused pane
emits, and comes down again as soon as the focused app's next content scan returns `ALLOW`. That
is the ordinary scan path doing its job, and it is what keeps this from becoming the failure §7
forbids — a touch-swallowing cover that cannot be dismissed. Do not make the cover sticky here
without an answer for how the user gets out from under it.

**Not attempted: evaluating watched windows on every unrelated app's event.** It would close the
quiet-pane gap above, and it would also mean any guarded Settings page left open in a pane covers
the whole screen indefinitely, with touches swallowed and no route to the in-app disarm. That is
the device-unusable outcome plan.md §7 rules out, so the gap is documented rather than traded for
it.

Reproduction scripts are not committed — split screen on Android 12+ is driven by the SystemUI
shell, so it has to be entered through the recents UI (`am start --windowingMode 6` only makes one
task multi-window at full-screen bounds, and any `am start` breaks out of an existing split).

## Closed: 1, the empty harvest

Kept in full because four of the five hypotheses were wrong, and two of them were wrong in ways
that would have shipped a change with no effect. Everything below was measured on an `android-36
google_apis arm64-v8a` emulator with the `empty harvest` diagnostic in `ScreenGuardService`.

**The cause.** Android 14 added `View#setAccessibilityDataSensitive`, and the framework withholds
nodes marked with it from any service whose `AccessibilityServiceInfo.isAccessibilityTool()` is
false. AOSP Settings marks the device-admin list rows sensitive — reasonably, since granting
device admin is exactly what a malicious accessibility service would want to drive. Our service
was handed the screen's chrome and an empty `recycler_view`, so the `SELF_IN_SETTINGS` catch-all
had no text to match and the screen that gates uninstall sat unguarded.

**The fix** is `android:isAccessibilityTool="true"` in `accessibility_service_config.xml`, and
nothing else. With it, `findAccessibilityNodeInfosByText` returns the row and the row reports
`isAccessibilityDataSensitive=true` — the mechanism confirming itself.

What the diagnostic ruled out, in the order the item ranked them:

| Hypothesis | Verdict |
|---|---|
| `flagIncludeNotImportantViews` unset — "the leading candidate" | **Wrong, and instructive.** Setting it does change the tree: the non-important containers (`content_frame`, `main_content`, `list_container`) reappear. The rows do not. It is not set today, because it widens what the service ingests in every app and buys nothing. |
| `getChild()` returning null | **Ruled out.** `declared == fetched` at every node of the walk. |
| Wrong window | **Ruled out.** `windows=1`. |
| `refresh()` on the root | **Ruled out**, as the item guessed — `refresh=true` with `childCount` still 0. |
| Stale node cache | Added during the investigation and also **ruled out**: `clearCache()` (API 33+) before each harvest changed nothing. |

Two further things were established while chasing it, and both matter more than the ranking did:

- **It was not slow rendering.** The rows were absent at 400 ms, 1 s, 2 s and again at 25 s, so no
  re-look schedule could ever have caught it.
- **"`uiautomator` sees the row" proves nothing about what this service sees.** `UiAutomation`
  sets its own service info, and `isAccessibilityTool` is part of what differs. The comparison
  that made this item look like a tree-lag bug was invalid from the start.

A separate real defect surfaced on the way and is fixed: an event from an **unwatched package was
treated as the user leaving**, so a `com.android.systemui` status-bar event landing ~200 ms after
the device-admin list opened cancelled that screen's entire re-look budget and reset the back-out
bound. `SettingsGuard.classifyUnwatchedEvent` now separates "chrome over a guarded screen" from a
real departure, with an explicit third answer for a foreground that cannot be resolved.
