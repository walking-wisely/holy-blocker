# image-sandbox

Image classification path: decode → tile → ONNX → max → verdict, called from the proxy's Phase 4 hook and from the macOS daemon over UniFFI.

| | |
|---|---|
| Code | `packages/image-sandbox/`, `packages/image-sandbox-ffi/` (Rust). |
| Status | [packages-image-sandbox](../../status/packages-image-sandbox.md), [packages-image-sandbox-ffi](../../status/packages-image-sandbox-ffi.md) |
| Next step | `python -m tools.plan.ledger next docs/components/image-sandbox` |
| Decisions | [classifier-operating-point](../../decisions/classifier-operating-point.md), [image-corpus-custody](../../decisions/image-corpus-custody.md) |
| Architecture | [network-pipeline](../../architecture/network-pipeline.md) (Phase 4), [content-classification](../../architecture/content-classification.md) |
| Platform note | Android uses LiteRT, not this crate — see [classifier-head](../classifier-head/README.md) |

## Files here

- [`plan.md`](plan.md) — Current state, modules, build order, the screen path
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Tiling or threshold behaviour: plan "Current state" and the operating-point decision.
- Screen-frame input: plan "The screen path".
