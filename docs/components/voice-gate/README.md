# voice-gate

Override mechanism: to disable protection the user reads a scripture passage aloud. Shared package plus platform adapters.

| | |
|---|---|
| Code | `packages/voice-gate/` (TypeScript) — not yet created. |
| Status | No `docs/status/` file; see the plan's own "Current state" |
| Next step | `python -m tools.plan.ledger next docs/components/voice-gate` |
| Flow | [override-gate](../../product/flows/override-gate.md) |
| Decisions | [protection-modes](../../decisions/protection-modes.md), [verse-selection](../../decisions/verse-selection.md), [verse-pools](../../decisions/verse-pools.md) |

## Files here

- [`plan.md`](plan.md) — Architecture, VoiceAdapter interface, modules, testing, build order
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Adapter contract: plan "VoiceAdapter interface".
