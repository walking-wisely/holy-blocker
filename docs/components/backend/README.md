# backend

Go service for the partner-accountability flow: invites and notifications only. No scanning data, URLs or content ever reach it.

| | |
|---|---|
| Code | Not yet created. Planned layout is in plan §2 (`backend/`). |
| Status | No `docs/status/` file; see the plan's own "Current state" |
| Next step | `python -m tools.plan.ledger next docs/components/backend` |
| Flows | [partner-setup](../../product/flows/partner-setup.md) |
| Decision | [accountability](../../decisions/accountability.md) |
| Sync design | [desktop/sync-plan.md](../desktop/sync-plan.md) |

## Files here

- [`plan.md`](plan.md) — Data model, API, rate limits, email templates, config, implementation order
- [`steps/`](steps/) — Step ledger, one file per step

## Read for

- API or data model: plan §3 and §5.
- Offline behaviour: plan §4, together with the desktop sync plan.
