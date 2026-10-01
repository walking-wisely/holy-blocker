# 13. `FullscreenControl` — AX `AXFullScreen`

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/FullscreenControl.swift
```

Force-exit fullscreen by setting `kAXFullScreenAttribute` to `false` on the focused window element.
Behind **Accessibility**.

Keep it as the **fallback path only**. A correctly configured overlay (module 10, with both
collection-behaviour flags) already covers a fullscreen Space, and yanking the user out of fullscreen
is a far more violent interaction than covering the screen. Reach for this when the overlay is known
not to work — the clearest case being applications that take an exclusive display capture, where no
window can be composited above them and covering is impossible by construction.

---
