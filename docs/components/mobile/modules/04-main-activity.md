# 4. `MainActivity` — onboarding — **Done**

Part of the [Mobile plan](../plan.md).

Reads `Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES` on resume (the user returns straight
from the toggle) and explains the Restricted Settings detour.

**Sideload friction is a feature here.** From Android 13, Accessibility sits behind
Restricted Settings for an app not installed through an app store (Android classifies by
installer/package source, not Play alone); Device Admin is listed among the restricted settings by
the Android 15 CDD. The first enable
attempt is blocked until the user opens App info → ⋮ → "Allow restricted settings" and
authenticates with the device PIN. If the partner holds the PIN, the protected user cannot
enable — or later disable — the service without them. That is the Android analogue of the
macOS admin-held-credential lock, and it is why the parsing in
`AccessibilityServiceStatus` is a real parser rather than a `contains` check: OEM builds vary
in whitespace, trailing separators, and whether the entry is fully qualified.

## Reference documents

- [`Settings.Secure#ENABLED_ACCESSIBILITY_SERVICES`](https://developer.android.com/reference/android/provider/Settings.Secure#ENABLED_ACCESSIBILITY_SERVICES)
- [Restricted settings](https://support.google.com/android/answer/12623953)
