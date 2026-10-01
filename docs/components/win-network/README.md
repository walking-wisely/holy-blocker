# win-network

Privileged Windows Service that installs and manages the Wintun adapter used by `net-shield`, with a named-pipe IPC server.

| | |
|---|---|
| Code | `native-modules/win-network/` (C++20, CMake). |
| Status | No `docs/status/` file; see the plan's own "Current state" |
| Next step | `python -m tools.plan.ledger next docs/components/win-network` |
| Boundary | Plan "Responsibility boundary" splits this from [net-shield](../net-shield/README.md), which owns the filtering logic |

## Files here

- [`plan.md`](plan.md) — Responsibility boundary, architecture, modules, testing, build system, build order
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Wintun or Win32 API behaviour: plan "Microsoft documentation reference".
