# desktop

Electron + React control panel: daemon status, block log, settings, tray, and the override gate.

| | |
|---|---|
| Code | `apps/desktop/` (TypeScript). |
| Status | [apps-desktop](../../status/apps-desktop.md) |
| Next step | `python -m tools.plan.ledger next docs/components/desktop` |
| Flows | [block](../../product/flows/block.md), [warn-interstitial](../../product/flows/warn-interstitial.md), [override-gate](../../product/flows/override-gate.md), [protection-mode-change](../../product/flows/protection-mode-change.md) |
| Decisions | [protection-modes](../../decisions/protection-modes.md), [accountability](../../decisions/accountability.md) |

## Files here

- [`plan.md`](plan.md) — Modules to add, tray integration, build order
- [`sync-plan.md`](sync-plan.md) — Desktop ↔ backend sync design
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Daemon IPC or verdict handling: plan "What to add".
- Talking to the backend: sync-plan.md.
