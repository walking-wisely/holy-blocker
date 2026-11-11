# 20. `BlockCounts` — content-free counts for the panel

Part of the [macOS Daemon plan](../plan.md). Step: `mac-daemon.block-counts`. Decision:
[tauri-desktop-panel.md](../../../decisions/tauri-desktop-panel.md) (6).

One integer per calendar day for text and domain blocks, 30 days retained, mode 0600, in the agent's own store. No window titles,
application names, scores or histograms.

- A test asserts that custom-app enforcement never increments a count and that neither the store nor the socket output carries a
  bundle identifier ([custom-app-blocking.md](../../../decisions/custom-app-blocking.md) decision 13).
- Pruning at start and at day rollover; a clock moving backwards never inflates a day.
- Adds a counts row to `.claude/skills/privacy-review/references/data-classes.md` in the same PR.
