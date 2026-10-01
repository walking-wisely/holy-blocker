# 2. `ScreenGuardService` — the AccessibilityService — **Done**

Part of the [Mobile plan](../plan.md).

Depth-first walk of `rootInActiveWindow` collecting `text` and `contentDescription`, bounded
by depth (40) and fragment count (400). The bounds are not incidental: web views expose very
deep trees and this runs on the UI-event path, so an unbounded walk jank the foreground app.

## Reference documents

- [`AccessibilityService`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService)
- [`AccessibilityServiceInfo`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityServiceInfo) — the `<accessibility-service>` config attributes
- [`AccessibilityNodeInfo`](https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo)
- [Build a custom accessibility service](https://developer.android.com/guide/topics/ui/accessibility/service)
