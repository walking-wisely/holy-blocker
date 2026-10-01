# classifier-head

The classifier head (embedding → score → verdict) in Rust behind UniFFI, shared by Android (LiteRT backbone) and Windows (ONNX backbone).

| | |
|---|---|
| Code | `packages/classifier-head/` — not yet created. |
| Status | No `docs/status/` file; see the plan's own "Current state" |
| Next step | `python -m tools.plan.ledger next docs/components/classifier-head` |
| Decisions | [learning-from-feedback](../../decisions/learning-from-feedback.md), [classifier-operating-point](../../decisions/classifier-operating-point.md) |
| Depends on | [machine-learning](../machine-learning/README.md) for the embedding-only export |

## Files here

- [`plan.md`](plan.md) — Cut point, weights format, parity-fixture rules, modules, build order
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Weights file layout or the backbone/head cut: plan "Where the backbone ends and the head begins".
- What a fixture may contain: plan "Parity fixtures".
