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

- ~~**A guarded screen sat unguarded because the harvest came back empty.**~~ The device-admin
  list — the one screen that can deactivate the admin — named this app and still did not hit the
  `SELF_IN_SETTINGS` catch-all. **Fixed** by declaring `android:isAccessibilityTool="true"`.

  The cause was `accessibilityDataSensitive`, added in Android 14 (API 34): views marked
  sensitive are served only to services that declare `isAccessibilityTool`, and Settings marks
  those rows that way. `uiautomator` saw them because UiAutomation is exempt from the mechanism.

  **Every candidate this item originally ranked was wrong, and the ranking is worth remembering
  as a caution.** Measured on an android-36 emulator before concluding:

  - `flagIncludeNotImportantViews` — ranked leading, disproven. Setting it grew the tree from
    5 to 16 nodes, adding every intermediate container and not one list row. It is deliberately
    **not** set: it widens what the service ingests on every app and buys nothing here.
  - `getChild()` returning null — disproven. `declared == fetched` at every re-look.
  - Wrong window — disproven, as this item predicted. `windows=1`.
  - `refresh()` on the root — disproven directly rather than dismissed as a red herring.
    `findAccessibilityNodeInfosByViewId` went back to the app, `refresh()` returned true, and
    `childCount` stayed 0. Not a stale node cache.

  Two assumptions in the original write-up were themselves the obstacle. The harvest was never
  *empty* — it returned three text fragments, so `logEmptyHarvest`, gated on `texts.isEmpty()`,
  never fired on the one screen it was written for; it now fires below a text floor. And
  `declared > fetched` was claimed to be the signature of view filtering, which it is not: the
  framework applies both `importantForAccessibility` and `accessibilityDataSensitive` before
  reporting `childCount`, so a withheld subtree shows no gap at all.

  `isAccessibilityTool` is also a Play policy declaration — it asserts the service assists users
  with disabilities — so it is a distribution commitment, not just a manifest attribute.
  Accepted knowingly: short of Device Owner, which [plan.md](plan.md) §7 rules out permanently,
  nothing else reaches those rows.

- ~~**Split screen harvested the wrong window.**~~ `rootFor` took the *first* window whose root
  matched the package, which in split screen need not be the one the user is driving. **Fixed** —
  `policy/WindowResolver.kt` prefers the event's own `windowId`, then focused, then active, then
  first match, and the event-less re-look path relies on that fallback by design.

  The action was fixed alongside the matching, which this item correctly insisted on:
  `GLOBAL_ACTION_BACK` takes no window argument and lands on the focused window, so
  `SettingsGuard.evaluate` degrades an unfocused match to `CoverOnly` instead of pressing BACK
  inside the user's actual foreground app. It does so *before* the back-out bookkeeping, so a
  pane parked unfocused cannot drain the back-out budget without a single BACK being sent.

  **Verification gap, deliberately recorded rather than glossed:** 12 JVM tests cover the
  selection order and the focus gate, but genuine two-app split screen could not be driven on the
  android-36 phone AVD through adb — neither `--windowingMode 6` nor freeform produced two
  independent app windows. Only the single-window path was exercised on device. The two-pane case
  this item exists for **still needs confirming by hand** on hardware or a tablet/foldable AVD.

  Recents, which this item was grouped with, remains open and untouched.

## Next

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

### 7. Dead `reset()` methods

`ScanGate.reset()` and `SettingsGuard.reset()` are never called — `onServiceConnected` builds
fresh instances. Harmless now, but `SettingsGuard.reset()` clears the release window, so wiring
it to a reconnect later would silently cancel an active release mid-window. Either call them on
reconnect instead of reallocating, or delete them.

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
