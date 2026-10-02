# net-shield

Rust packet-level filter: domain/IP trie, SNI parser, Wintun path, and the DNS path used by the Android VPN, with signed-blocklist loading.

| | |
|---|---|
| Code | `packages/net-shield/`, `packages/net-shield-ffi/` (Rust). |
| Status | [packages-net-shield](../../status/packages-net-shield.md), [packages-net-shield-ffi](../../status/packages-net-shield-ffi.md) |
| Next step | `python -m tools.plan.ledger next docs/components/net-shield` |
| Architecture | [network-pipeline](../../architecture/network-pipeline.md) |
| Blocklist integration | [domain-blocklist module 6](../domain-blocklist/modules/06-net-shield.md) |
| Windows driver side | [win-network](../win-network/README.md) |

## Files here

- [`plan.md`](plan.md) — Current state, modules, build order, references
- [`steps/`](steps/) — Step ledger, one file per step

## Read for

- Blocklist loading or precedence: the domain-blocklist module 6 spec, not this plan.
