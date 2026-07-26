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

### 2. Split-screen harvests the wrong window

`guardSettingsScreen` goes through `currentScreenIdentity` → `rootFor`, which already enumerates
`windows` filtered by package. The remaining gap is narrower: `rootFor` takes the *first* window
whose root matches the package, which is not necessarily the event's own window. In split screen
two Settings-adjacent windows can coexist, so the guard can evaluate the wrong one, `mentionsSelf`
fails, and the app-label catch-all §7 calls load-bearing silently does nothing.

**This was never the empty-harvest bug**, and an earlier planning pass treating the two as one
cost real time. The diagnostic settled it on device: `windows=1` on the device-admin list, and the
cause was `accessibilityDataSensitive` filtering, which is per node and has nothing to do with
which window is picked. This item stands on its own and is still unverified — split screen has not
been exercised on the emulator.

Fix: resolve the root for the event's own window via `getWindows()` / `event.windowId`, falling
back to `rootInActiveWindow`. **Note this mechanism is unavailable on the re-look path** —
`evaluateCurrentScreen` is deliberately event-less, so there is no `windowId` to match; it needs
a separate criterion (prefer `isActive`/`isFocused` among matches).

Do **not** "evaluate every window belonging to a watched package" without also fixing the action:
`applyDecision` fires `performGlobalAction(GLOBAL_ACTION_BACK)`, which is global and lands on the
*focused* window. Matching a non-focused Settings pane would press BACK in whatever app the user
is actually using, and would never dismiss the pane that matched, so it loops until the bound
trips — ~3.6 s of stray BACK presses, then a cover over the innocent app. Gate `BackOut` on the
matched window being focused; cover instead when it is not.

`flagRetrieveInteractiveWindows` is already set in `accessibility_service_config.xml`, so the
capability is paid for.

Keep the decision in `SettingsGuard` (pass a list of `ScreenIdentity`, return the strongest
decision) so it stays JVM-testable — but this is not just a signature change. `evaluate` mutates
`consecutiveBackOuts`, `lastSurface` and `lastBackOutAtMillis` on every call, so iterating a list
through it breaks the one-decision-per-event invariant in both directions: N identities burn N
increments and hit `MAX_CONSECUTIVE_BACK_OUTS` within about two events, while two windows matching
two *different* surfaces alternate `lastSurface` and reset the bound forever, so it never trips.
The note under "checked and found not to be holes" that calls alternating surfaces safe holds
only for the single-screen case.

`match()` is already a pure private function, so the shape is: match every identity, merge, then
call the counter-updating decision **once**. Define "strongest" carefully — `CoverOnly` is the
*give-up* state, weaker enforcement than `BackOut` despite being the escalation, so ranking it
higher lets one window's exhausted budget silence a window that still has one.

### 3. `app_name` must never be localised

The catch-all matches on the app's own label, which works only because it is a brand string.
There is currently one `res/values/` directory, so the reasoning holds — but adding any
translated `strings.xml` would localise `app_name` and silently degrade the matcher in that
locale, reintroducing from our own side the exact failure §7 warns about for Settings' copy.

Fix: `translatable="false"`, or a dedicated non-localised match constant, plus a test asserting
the matcher label is locale-independent.

### 4. SystemUI is entirely unwatched

`watchesPackage` covers Settings and (now) the installers. `com.android.systemui` hosts Quick
Settings and the accessibility floating panel. On AOSP the a11y panel toggles the *shortcut
assignment* rather than the service enable-state, so it is probably not a direct disable route
on the verified target — but the package is invisible to the guard on builds we have no data
about, and Xiaomi and Samsung both customise SystemUI heavily.

Fix: watch it on a **self-mention-only** path, and **cover rather than back out**. Backing out of
the notification shade or volume panel is indistinguishable from a broken phone.

### 5. `OverlayController` can crash the service

`hide()` calls `removeView` uncaught on the accessibility callback path; it throws
`IllegalArgumentException` when the view is not attached. A throw there kills the event, and
repeated throws can take the service down — a bypass by way of a crash.

Fix: wrap `addView`/`removeView`, and on `addView` failure leave `shownState` as `CLEAR` so the
next event retries rather than believing a cover is up that is not.

### 6. Foreground service

Does not keep the guard alive (plan.md, implementation order) and does not close recents-swipe on
Pixel, where a bound accessibility service is not killed by swiping. It does raise priority
against low-memory kills and gives an always-visible status signal. Verify the recents-swipe
claim on real Samsung hardware before writing any mitigation for it — the claim in §7 comes from
AppBlock's docs, and AppBlock ships to OEMs with aggressive task-killers that AOSP does not have.

### 7. Dead `ScanGate.reset()`

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

## Checked and found not to be holes

Recorded so they are not re-investigated.

- **Clearing app data** is not a release bypass — it clears the request, which makes the guard
  stricter. This stays true only while the stored state is a *request* rather than a grant.
- **Tripping the back-out bound** is not a walk-in. `CoverOnly` still shows the opaque,
  touch-swallowing cover, and `lastSurface` is a single field, so alternating surfaces resets the
  counter rather than accumulating.
- **Settings' search box** collapses into the existing window-state-event reliability problem
  rather than being a distinct route: results naming the app hit the catch-all, and tapping
  through lands on a guarded activity.
- **The volume-key accessibility shortcut** invokes an assigned service's action; it does not
  toggle the enable-state. Assigning it happens on the guarded per-service page.
- **A boot receiver is not needed for the guard to survive reboot** — the system rebinds enabled
  accessibility services automatically. It is still worth adding for tamper-log gap detection.

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
