# mitm-proxy

Rust HTTPS interception proxy with text and image scan hooks and runtime ProtectionMode.

| | |
|---|---|
| Code | `packages/mitm-proxy/` (Rust). |
| Status | [packages-mitm-proxy](../../status/packages-mitm-proxy.md) |
| Next step | `python -m tools.plan.ledger next docs/components/mitm-proxy` |
| Architecture | [network-pipeline](../../architecture/network-pipeline.md) |
| Flows | [block](../../product/flows/block.md), [warn-interstitial](../../product/flows/warn-interstitial.md) |
| Decision | [protection-modes](../../decisions/protection-modes.md) |

## Files here

- [`plan.md`](plan.md) — Current state, modules, build order
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Known gaps (plain-HTTP path unscanned, leaf cert without Authority Key Identifier): [status](../../status/packages-mitm-proxy.md).
