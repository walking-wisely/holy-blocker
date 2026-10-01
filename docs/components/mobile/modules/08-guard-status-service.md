# 8. `GuardStatusService` — the foreground status surface — **Done**

Part of the [Mobile plan](../plan.md).

**It is not what keeps the guard alive, and no copy about it should imply otherwise.** An
`AccessibilityService` is bound by the system and is rebound after a reboot for as long as it
stays enabled, so a foreground service makes the guard neither harder to kill nor able to survive
anything it could not survive already. What it adds is a process that is still running *after* the
guard stops:

- **It closes the "still-alive process" half of the detection surface** in
  [backlog.md](../backlog.md), "Cannot be closed at Device Admin level". An `adb` disable, a guest
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

## Reference documents

- [Foreground services](https://developer.android.com/develop/background-work/services/fgs) — including the five-second `startForeground` deadline
- [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) and [`specialUse`](https://developer.android.com/develop/background-work/services/fgs/service-types#special-use)
- [Background start restrictions](https://developer.android.com/develop/background-work/services/foreground-services#background-start-restrictions) — the exemption list that lets `BOOT_COMPLETED` start one
- [`POST_NOTIFICATIONS`](https://developer.android.com/develop/ui/views/notifications/notification-permission)
- [`ContentObserver`](https://developer.android.com/reference/android/database/ContentObserver) and [`Settings.Secure#getUriFor`](https://developer.android.com/reference/android/provider/Settings.Secure#getUriFor(java.lang.String))
- [`NotificationChannel`](https://developer.android.com/reference/android/app/NotificationChannel) — importance, and why `IMPORTANCE_LOW` is right for an ongoing status

## Reference documents — step 7

- [`DeviceAdminReceiver`](https://developer.android.com/reference/android/app/admin/DeviceAdminReceiver) — and [`onDisableRequested`](https://developer.android.com/reference/android/app/admin/DeviceAdminReceiver#onDisableRequested(android.content.Context,%20android.content.Intent))
- [Device administration overview](https://developer.android.com/work/device-admin) — the admin/owner capability split
- [Device admin deprecation](https://developers.google.com/android/work/device-admin-deprecation) — what is dead and what still works
- [`AccessibilityNodeInfo#getViewIdResourceName`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo#getViewIdResourceName())
- [`AccessibilityServiceInfo#FLAG_REPORT_VIEW_IDS`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo#FLAG_REPORT_VIEW_IDS)
- [`AccessibilityService#GLOBAL_ACTION_BACK`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService#GLOBAL_ACTION_BACK)
