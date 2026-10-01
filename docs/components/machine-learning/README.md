# machine-learning

Python evaluation and export pipeline for the frozen image backbone that `image-sandbox` and `classifier-head` consume.

| | |
|---|---|
| Code | `machine-learning/` (Python). |
| Status | No `docs/status/` file; see the plan's own "Current state" |
| Next step | `python -m tools.plan.ledger next docs/components/machine-learning` |
| Constraint | The plan opens with a legal decision: no corpus of real explicit imagery, in any form. Read it before anything else. |
| Decisions | [classifier-operating-point](../../decisions/classifier-operating-point.md), [image-corpus-custody](../../decisions/image-corpus-custody.md) |

## Files here

- [`plan.md`](plan.md) — Corpus decision, baseline evaluation, modules, build order
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Anything touching data: the first plan section, then `image-corpus-custody`.
