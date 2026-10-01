# 11. `EventHooks` — window lifecycle and scroll

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/EventHooks.swift
```

Supplies the fast wakeups the scan loop debounces, per
[edge-daemons.md](../../../architecture/edge-daemons.md). Two sources:

- **`AXObserver`** for window lifecycle and geometry —
  `kAXFocusedWindowChangedNotification`, `kAXWindowMovedNotification`,
  `kAXWindowResizedNotification`, plus `NSWorkspace.didActivateApplicationNotification` for the
  application-level change. Requires **Accessibility**. An `AXObserver` is inert until its run-loop
  source is added via `CFRunLoopAddSource(_, AXObserverGetRunLoopSource(observer), .defaultMode)` —
  omitting that yields an observer that registers successfully and never fires.
- **Scroll**, feeding the scroll-delta debounce.

**On scroll, prefer `NSEvent.addGlobalMonitorForEvents(matching: .scrollWheel)` over `CGEventTap`.**
The daemon only needs to *observe* scroll, never to consume or modify it, and the global monitor is
the read-only API for exactly that. It avoids the `CGEventTap` failure mode that is missed almost
universally: **the system disables a tap that takes too long to respond**, delivering a
`kCGEventTapDisabledByTimeout` event, and unless the callback watches for that event type and calls
`CGEventTapEnable` again, scroll detection dies silently mid-session and never recovers. If a tap
turns out to be necessary anyway, that re-enable path is mandatory, not defensive.

The monitor route also likely avoids requesting **Input Monitoring** at all. Every additional TCC
prompt is onboarding friction on a permission the user can revoke, so the fewer requested the
better. Which permission a listen-only scroll monitor actually requires on macOS 26 should be
**measured on the real system before it is written into onboarding** — the documentation is not
crisp on the Accessibility/Input Monitoring boundary for non-keyboard events, and this plan should
not assert what it has not run.

## Reference documents

- [Apple — Accessibility API / `AXUIElement`](https://developer.apple.com/documentation/applicationservices/axuielement_h) — `AXObserver` creation, notification registration, and run-loop source attachment.
- [Apple — `NSEvent` global monitors](https://developer.apple.com/documentation/appkit/nsevent/1535472-addglobalmonitorforevents) — the read-only observation path preferred here.
- [Apple — Quartz Event Services (`CGEventTap`)](https://developer.apple.com/documentation/coregraphics/quartz_event_services) — tap creation, the timeout-disable behaviour, and `CGEventTapEnable`.

---
