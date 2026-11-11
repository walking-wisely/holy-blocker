# `packages/daemon-ipc` — status

Language: Rust

`daemon-ipc.core` is done. The decision is accepted ([daemon-ipc-and-process-model](../decisions/daemon-ipc-and-process-model.md)) and the plan is in [`docs/components/daemon-ipc/plan.md`](../components/daemon-ipc/plan.md): `daemon-ipc.core`, `daemon-ipc.limits`, `daemon-ipc.ffi` for the MVP and `daemon-ipc.c-abi` for Windows later.

## What exists

- `frame`: a streaming decoder that reads only what the current frame needs, so the buffer never exceeds the 4-byte prefix plus the cap; the cap is `Limits::max_frame`, not a constant.
- `msg`: `ToDaemon` and `ToPanel` (snake_case `type` tag), `AppIdentity` (validated, `Debug` prints nothing of it), the `hello` handshake with `MIN_PROTO..=PROTO` (both 1), unknown type reported as `Unsupported` and a malformed body of a known type as `Malformed`.
- `session`: a `Session` per connection for four ends (daemon facing a panel, daemon facing an agent, panel, agent). Closes on a pre-`hello` message, a protocol outside the range, a second `hello`, a malformed or oversized frame and a message from the wrong role. No clock, caps, rate limits or liveness yet: those are `daemon-ipc.limits`, and the default 4 KiB and 64 KiB caps arrive with them.
- Tests: hand-written vectors replayed whole and in 1- and 3-byte chunks, proptest properties (any chunking gives the same events, arbitrary bytes never panic or over-buffer), redaction, and `panic = unwind` asserted.

## Not yet

Everything after `daemon-ipc.core` in the plan. Nothing is observed against a real socket, and no Swift test reads the vectors.
