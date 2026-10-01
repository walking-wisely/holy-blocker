# Gotchas learned the hard way

Part of the [Mobile plan](plan.md).

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
- **`onActivityResult` runs before `onResume`.** Any state the activity re-reads on resume must
  already be true by the time the result handler returns — a flag set later, inside the service
  the result starts, is read too early and the screen reports the opposite of what is happening.
  See step 12.
- **The `PROJECT_MEDIA` appop is checked when consent is granted, not while it is used.** Allowing
  it makes the consent activity return `RESULT_OK` without showing UI, which is what makes the
  capture path scriptable at all; *denying* it does nothing to a projection already running.
- **`am stopservice` cannot touch a non-exported service** — "Error stopping service", by design.
  There is no adb route into a service that is correctly locked down, which is a constraint on
  what a smoke test can assert rather than a bug to fix.
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
- **AGP 9 needs Gradle 9.6 and brings its own Kotlin.** Applying `org.jetbrains.kotlin.android`
  alongside it is a hard error, so the app module no longer applies it; the root build still
  declares the plugin `apply false` because that is what pins the Kotlin version AGP uses (2.4.20,
  not AGP's 2.2.10 default). `sourceSets["main"].java.srcDirs(...)` no longer feeds Kotlin
  compilation — the generated UniFFI bindings fail as `Unresolved reference 'uniffi'` until they
  are added through `kotlin.directories`.
