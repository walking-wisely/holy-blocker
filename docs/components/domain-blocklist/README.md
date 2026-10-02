# domain-blocklist

Offline pipeline that fetches, merges, revalidates and signs the domain blocklist into an mmap'd FST artifact for `net-shield`. Also covers the shared `domain-normalize` crate (module 0).

| | |
|---|---|
| Code | `packages/domain-blocklist/`, `packages/domain-normalize/` (Rust). |
| Status | [packages-domain-blocklist](../../status/packages-domain-blocklist.md), [packages-domain-normalize](../../status/packages-domain-normalize.md) |
| Next step | `python -m tools.plan.ledger next docs/components/domain-blocklist` |
| Decision | [domain-blocklist-sourcing](../../decisions/domain-blocklist-sourcing.md) — read first; it carries the legal boundary |
| Consumer | [net-shield](../net-shield/README.md) (its module 6 is specified here) |

## Files here

- [`plan.md`](plan.md) — Current state, why a separate crate, build order, benchmarks, references
- [`modules/`](modules/) — One spec per module: 00 domain-normalize, 01 sources, 02 merge, 03 liveness, 04 fst-build, 05 gates, 06 net-shield, 07 cli, 08 overlay
- [`archive/`](archive/) — Point-in-time review records; not current state
- [`steps/`](steps/) — Step ledger, one file per step

## Read for

- Implementing a module: its file under `modules/`, then the matching entry in plan "Implementation order" for what already landed.
- Why the pipeline is shaped this way: the decision record, not the plan.
