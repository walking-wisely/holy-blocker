# desktop

Tauri + React control panel. MVP on macOS: daemon status, content-free block counts, lowering protection through the cooldown, and the custom-app list. The daemon owns all state.

| | |
|---|---|
| Code | `apps/desktop/` (TypeScript renderer, Rust core). |
| Status | [apps-desktop](../../status/apps-desktop.md) |
| Next step | `python -m tools.plan.ledger next docs/components/desktop` |
| Flows | [block](../../product/flows/block.md), [warn-interstitial](../../product/flows/warn-interstitial.md), [override-gate](../../product/flows/override-gate.md), [protection-mode-change](../../product/flows/protection-mode-change.md) |
| Decisions | [tauri-desktop-panel](../../decisions/tauri-desktop-panel.md), [protection-modes](../../decisions/protection-modes.md), [accountability](../../decisions/accountability.md), [custom-app-blocking](../../decisions/custom-app-blocking.md) |

## Files here

- [`plan.md`](plan.md) — Shape, MVP contents, steps, what is out
- [`sync-plan.md`](sync-plan.md) — Desktop ↔ backend sync design (deferred, predates the Tauri decision)
- [`steps/`](steps/) — Step ledger, one file per step

## Read for

- Daemon IPC or any state the panel shows: plan "Shape" and the decision record.
- Talking to the backend: sync-plan.md.
