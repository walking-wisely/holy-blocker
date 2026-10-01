# win-daemon

C++20 Windows daemon: WinEvent hooks and message loop today; capture, OCR and IPC planned.

| | |
|---|---|
| Code | `native-modules/win-daemon/` (C++20, CMake). |
| Status | [native-modules-win-daemon](../../status/native-modules-win-daemon.md) |
| Next step | `python -m tools.plan.ledger next docs/components/win-daemon` |
| Architecture | [edge-daemons](../../architecture/edge-daemons.md) |
| Flows | [block](../../product/flows/block.md), [warn-interstitial](../../product/flows/warn-interstitial.md) |
| Convention | Keep Win32 callback glue small; decision logic goes in testable helpers |

## Files here

- [`plan.md`](plan.md) — Current state, modules, build order
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Cadence and responsibilities shared with macOS: edge-daemons.
