# 3. `OverlayController` — the cover — **Done**

Part of the [Mobile plan](../plan.md).

Uses `TYPE_ACCESSIBILITY_OVERLAY`, which an accessibility service may draw *without* a
separate "display over other apps" grant, and which sits above `TYPE_APPLICATION_OVERLAY`.
The opaque cover swallows touches so content underneath cannot be interacted with blind; the
warn tint stays passive (`FLAG_NOT_TOUCHABLE`).

## Reference documents

- [`WindowManager.LayoutParams`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams) — overlay types and flags
- [`TYPE_ACCESSIBILITY_OVERLAY`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams#TYPE_ACCESSIBILITY_OVERLAY)
- [`SYSTEM_ALERT_WINDOW`](https://developer.android.com/reference/android/Manifest.permission#SYSTEM_ALERT_WINDOW) — needed only for the fallback path
