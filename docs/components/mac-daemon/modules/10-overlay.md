# 10. `Overlay` — borderless `NSWindow`

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/Overlay.swift
```

The response surface. Two distinct visual states, and the sketch conflated them:

| State | Mouse events | Purpose |
|---|---|---|
| Warn interstitial | **Must be swallowed** | Blur + verse + a deliberate choice, per [warn-interstitial.md](../../../product/flows/warn-interstitial.md) |
| Block cover | **Must be swallowed** | Full interrupt, per [block.md](../../../product/flows/block.md) |
| Pre-blur / passive | `ignoresMouseEvents = true` | A hint that does not interrupt |

`ScanVerdict.regions` (module 9) makes a fourth, narrower state possible — a cover limited to the
detected `NormalizedRect`s instead of the whole window or display — but treat full-surface cover as
the only implemented behavior for now. A region cover needs the window's on-screen bounds *at
capture time* to convert a frame-normalized box into screen coordinates, and the window can move or
resize between capture and render; get the simple case (whole-surface cover, ignoring `regions`)
correct and verified first, and revisit region-level cover once `Scanner` actually populates
`regions` instead of always returning an empty list.

`ignoresMouseEvents` belongs to the passive case only. An interstitial that lets clicks through to
the content underneath is not an interstitial — the user can keep interacting with exactly what was
just flagged.

Requirements:

- Transparent, borderless, `NSWindow.Level` above normal windows; `.screenSaver` level clears the
  menu bar and Dock.
- `collectionBehavior` must contain **both** `.canJoinAllSpaces` **and** `.fullScreenAuxiliary`.
  With only one, the overlay silently fails to appear over native-fullscreen video — which is
  precisely the case that matters most, and it fails by simply not showing rather than by erroring.
- **One overlay per `NSScreen`.** A single window covers one display; on a two-monitor setup the
  second screen is left showing the content. Observe
  `NSApplication.didChangeScreenParametersNotification` and rebuild the set when displays are
  connected, disconnected, or rearranged.
- **The process needs an `NSApplication` run loop.** This package is currently a plain SwiftPM
  executable with no `NSApplication`, and an `NSWindow` created without one will never appear.
  Layer 2's GUI-session half must call `NSApplication.shared`, set
  `setActivationPolicy(.accessory)` — so it draws overlays without a Dock icon or menu bar — and run
  the main run loop. This is another reason Layer 1 and Layer 2 are separate processes.
- The blurred backdrop uses the captured frame per
  [warn-interstitial.md](../../../product/flows/warn-interstitial.md). **That frame is never persisted,
  never logged, and never leaves the device** — it is a texture, and the local-first rule in the
  root `AGENTS.md` applies to it with full force.

Keep the drawing thin: the pure part is *which* overlays should exist for a given verdict and screen
configuration, and that is a testable function returning a set of frames. AppKit does the rest.

**Built.** `Overlay.swift` is the pure half (`OverlayPlan.plan`, plus `OverlayReconciliation.diff`,
which the AppKit half needs and which is logic rather than drawing); `OverlayWindow.swift` is the
AppKit half (`AppLifecycle`, `ScreenConfiguration.current()`, the level/collection-behavior
mappings, `OverlayController`, `OverlayContentView`). See step 4 of the implementation order below
for what was learned and what is still unverified.

## Reference documents

- [Apple — `NSWindow.CollectionBehavior`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior) — `.canJoinAllSpaces` and `.fullScreenAuxiliary` semantics for overlays over fullscreen Spaces.
- [Apple — `NSWindow.Level`](https://developer.apple.com/documentation/appkit/nswindow/level) — the ordering constants including `.screenSaver`.
- [Apple — `NSApplication.ActivationPolicy`](https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy) — `.accessory` for a UI-bearing process with no Dock presence.

---
