# daemon-ipc

Sans-IO Rust core for the channel between the root policy daemon and its clients (the Tauri panel and the user-level agent): framing, message schema, version handshake, session state machine, inbound limits, rate limiting, liveness and the shared test vectors. A UniFFI wrapper serves the Swift daemon; the panel links the crate directly.

| | |
|---|---|
| Code | `packages/daemon-ipc/`, `packages/daemon-ipc-ffi/` (Rust). Planned, not yet created. |
| Status | [packages-daemon-ipc](../../status/packages-daemon-ipc.md) |
| Next step | `python -m tools.plan.ledger next docs/components/daemon-ipc` |
| Decision | [daemon-ipc-and-process-model](../../decisions/daemon-ipc-and-process-model.md) |
| Consumers | [desktop](../desktop/README.md) (panel), [mac-daemon](../mac-daemon/README.md) (root daemon and agent), [win-daemon](../win-daemon/README.md) later |

## Files here

- [`plan.md`](plan.md) — scope, build order, step specifications
- [`steps/`](steps/) — step ledger, one file per step

## Read for

- The wire contract (framing, messages, handshake, limits): the decision record, then `plan.md`, not a consumer's copy.
- A change to a message shape: it is a cross-platform contract. Add it, keep the old shape until every in-tree consumer has moved, raise `MIN_PROTO` to remove it.
- Transport, peer authentication and policy: not here. They stay with each platform.
