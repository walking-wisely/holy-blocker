# text-policy

Rust text engine: normalize, lexicon, scorer, evaluator, policy. Consumed over UniFFI by Android and the macOS daemon.

| | |
|---|---|
| Code | `packages/text-policy/`, `packages/text-policy-ffi/` (Rust). |
| Status | [packages-text-policy](../../status/packages-text-policy.md), [packages-text-policy-ffi](../../status/packages-text-policy-ffi.md) |
| Next step | `python -m tools.plan.ledger next docs/components/text-policy` |
| Architecture | [content-classification](../../architecture/content-classification.md) |
| Contract | The FFI crate's `Action`, `SourceKind` and `Verdict` are shared by two platforms; changing them breaks both |
| Decision | [learning-from-feedback](../../decisions/learning-from-feedback.md) (why text scoring is demoted) |

## Files here

- [`plan.md`](plan.md) — Current state, modules, build order
- [`steps/`](steps/) — Step ledger, one file per step

## Read for

- Changing the FFI surface: check both consumers first, [mobile](../mobile/README.md) and [mac-daemon](../mac-daemon/README.md).
