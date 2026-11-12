# `packages/daemon-ipc` — status

Language: Rust

`daemon-ipc.core` and `daemon-ipc.limits` are done. The decision is accepted ([daemon-ipc-and-process-model](../decisions/daemon-ipc-and-process-model.md)) and the plan is in [`docs/components/daemon-ipc/plan.md`](../components/daemon-ipc/plan.md): `daemon-ipc.core`, `daemon-ipc.limits`, `daemon-ipc.ffi` for the MVP and `daemon-ipc.c-abi` for Windows later.

## What exists

- `frame`: a streaming decoder that reads only what the current frame needs, so the buffer never exceeds the 4-byte prefix plus the cap; the cap is `Limits::max_frame`, not a constant.
- `msg`: `ToDaemon` and `ToPanel` (snake_case `type` tag), `AppIdentity` (validated, `Debug` prints nothing of it), the `hello` handshake with `MIN_PROTO..=PROTO` (both 1), unknown type reported as `Unsupported` and a malformed body of a known type as `Malformed`.
- `session`: a `Session` per connection for four ends (daemon facing a panel, daemon facing an agent, panel, agent), on an injected monotonic clock: `Session::new(end, limits, now_ms)`, `receive(bytes, now_ms)`, `tick(now_ms)`. Closes on a pre-`hello` message, a protocol outside the range, a second `hello`, a malformed or oversized frame and a message from the wrong role, and now also on a missed pre-`hello` deadline (5 s), a frame unfinished 5 s after its first byte, 60 s without bytes from an agent (300 s for the other ends), more than 8 consecutive unknown types, and an agent with no heartbeat for 15 s (`HeartbeatLost`).
- Limits: `Limits::to_daemon()` (4 KiB) and `Limits::to_panel()` (64 KiB); a token bucket of 10 messages per second and 1 state-changing message per second on the daemon end, with over-limit messages reported as `Event::Throttled` and `cancel_disarm` and `raise_protection` exempt; a repeated `request_disarm` or `request_remove_app` while one is pending is `Event::Duplicate` (the server calls `settle_disarm` or `settle_removal` when it resolves one the session cannot see); `Limits::validate` rejects contradictory values; `drain_counters` returns a throttle count, a duplicate count and the window, with no peer or message text.
- Tests: hand-written vectors replayed whole and in 1- and 3-byte chunks, proptest properties (any chunking gives the same events, arbitrary bytes never panic or over-buffer), redaction, and `panic = unwind` asserted for the test profile only: a consumer's release profile can still set `abort`, so `daemon-ipc.ffi` asserts it again in its own build.

## Not yet

Everything after `daemon-ipc.limits` in the plan. Nothing is observed against a real socket, and no Swift test reads the vectors.
