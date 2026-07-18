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

## Next

### 1. A guarded screen can sit unguarded because the harvest comes back empty

**The headline case: the device admin list is not guarded.** It names this app, so it should hit
the `SELF_IN_SETTINGS` catch-all — and it does not. Reproduced repeatedly on an android-36
emulator: the list sits open showing our own row, service bound and subscribed, guard idle.

This is *not* an event-delivery problem, which is what it looks like at first. Instrumenting
`onAccessibilityEvent` showed the event arrives normally
(`pkg=com.android.settings type=32`, i.e. `TYPE_WINDOW_STATE_CHANGED`). The failure is entirely
in the harvest. Two distinct causes were measured, and only the first is fixed:

**(a) `rootInActiveWindow` is null — fixed.** On a settings task restored from the background it
returns null and *stays* null: still null 2s after the screen was drawn and interactive. Every
logging and evaluation path sat behind `root != null`, so the guard was silent rather than
visibly failing. Fixed by falling back to enumerating `windows` for a root matching the event's
package (`ScreenGuardService.rootFor`), plus `RescanSchedule` to take a second look after the
events stop. `flagRetrieveInteractiveWindows` was already set, so this cost no new capability.

**(b) The node tree lacks the list rows — open, and this is what still breaks the case above.**
With (a) fixed the re-looks now run against a real tree, and the tree is *still* wrong: the
device-admin list harvests only `content_parent`, `collapsing_toolbar`, `recycler_view` — the
chrome, with no row nodes at all — at 400 ms, 1 s and 2 s after the event, while a `uiautomator`
dump of the same moment plainly shows the "Holy Blocker" row. So this is not slow rendering and
no amount of waiting fixes it; the tree we are handed genuinely does not contain the rows.

Unresolved. Candidates worth testing, cheapest first:

- `AccessibilityNodeInfo.refresh()` on the root before walking — the tree may be a stale snapshot.
- Fetching the root from the **active/focused** window specifically rather than the first
  package match in `windows`; a backgrounded settings window from earlier in the same task would
  present exactly this shape (correct chrome, no current rows).
- Whether the `RecyclerView` children need `getChild()` to be driven differently to force the
  fetch, or are gated by `importantForAccessibility`.

The `texts=` count now in the `settings screen` debug log is the signal to work from: an empty
harvest on a visibly populated screen is this bug, and is otherwise indistinguishable from a
screen that simply did not match.

Note this weakens the catch-all generally, not just for the device-admin list — `mentionsSelf`
cannot fire on a tree with no text in it, on any screen where the harvest comes back empty.

### 2. Split-screen harvests the wrong window

`ScreenGuardService.guardSettingsScreen` takes `packageName`/`className` from the event but
harvests text from `rootInActiveWindow` — the *focused* window. In split screen those differ, so
a Settings event gets the other app's node tree, `mentionsSelf` fails, and the app-label
catch-all that §7 calls load-bearing silently does nothing.

Overlaps item 1(a), which added a `windows` fallback for the *null-root* case only. That fallback
takes the first window whose root matches the package, which is not the same as taking the
event's own window — so split screen is still wrong, and picking the wrong window there is a
live suspect for 1(b).

Fix: resolve the root for the event's own window via `getWindows()` / `event.windowId`, falling
back to `rootInActiveWindow`. Better still, evaluate every window belonging to a watched package
— that closes freeform and picture-in-picture by the same change. `flagRetrieveInteractiveWindows`
is already set in `accessibility_service_config.xml`, so the capability is already paid for.

`ScreenGuardService.rootFor` now reads `windows`, but only to recover from a null
`rootInActiveWindow` — it takes the first window whose root matches the package, which is not the
event's window. That leaves split screen wrong, and is a live suspect for 1(b).

Keep the decision in `SettingsGuard` (pass a list of `ScreenIdentity`, return the strongest
decision) so it stays JVM-testable.

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
