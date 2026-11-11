# daemon-ipc — Implementation Plan

Decision: [daemon-ipc-and-process-model.md](../../decisions/daemon-ipc-and-process-model.md) (3, 4, 5, 6).

The crate does no I/O. The transport (Unix socket on macOS, named pipe on Windows), the peer checks and every policy decision (cooldown, tamper log) stay with the platform that owns them. What the crate owns is everything that must be byte-identical or rule-identical on both ends, so it is written once and tested against vectors written by hand from the decision.

## Current state

`daemon-ipc.core` is done: `packages/daemon-ipc` holds the frame decoder, the `ToDaemon` and `ToPanel` enums, the handshake and a minimal `Session`, tested against hand-written vectors in `packages/daemon-ipc/tests/vectors/`. The old framing still lives twice, in `native-modules/mac-daemon/Sources/MacDaemon/DaemonIPC.swift` and `apps/desktop/src-tauri/src/frame.rs`, until `desktop.adopt-daemon-ipc` and `mac-daemon.adopt-daemon-ipc-ffi` replace them; the Tauri test now reads the shared framing vectors. No Swift test reads them yet.

## Implementation order

1. ~~`daemon-ipc.core` — frames, messages, handshake, vectors, property tests.~~ **Done.** <!-- step: daemon-ipc.core -->
2. `daemon-ipc.limits` — the session state machine on an injected clock: caps, deadlines, rate limiter, liveness. <!-- step: daemon-ipc.limits -->
3. `daemon-ipc.ffi` — the UniFFI wrapper for Swift, every export returning `Result`. <!-- step: daemon-ipc.ffi -->
4. `daemon-ipc.c-abi` — a cbindgen C ABI for the Windows daemon, when `win-daemon.ipc` starts. <!-- step: daemon-ipc.c-abi -->

## Step specifications

### daemon-ipc.core

Decision (3, 5). Write the message enumeration and the vectors first, from the decision and from
[tauri-desktop-panel.md](../../decisions/tauri-desktop-panel.md) decision 3 and
[custom-app-blocking.md](../../decisions/custom-app-blocking.md) (the add and remove payloads), then the code.

- `frame`: 4-byte big-endian length, then UTF-8 JSON; the length is checked before any body is buffered. The cap is a `Limits` field, not a constant.
- `msg`: two serde enums, `ToDaemon` and `ToPanel`, tagged `type` in snake_case, plus `hello` and the agent-role messages. Messages that carry an app identity hold it only as `AppIdentity`, whose `Debug` prints nothing of it, and no error built from it carries it; `Next::Frame` prints its length, never its body. Tests assert both, including the byte-array form.
- Handshake: `{"type":"hello","proto":N}` first each way, accepted when `MIN_PROTO <= N <= PROTO`; any other message before it closes. Unknown `type` is `Unsupported` and the connection stays open; malformed or oversized closes.
- Vectors: valid, truncated, oversized, at-cap, split across reads, unknown type, pre-hello, version below and above range. The existing `daemon-ipc-framing.json` moves here.
- Property tests: any chunking of a valid stream yields the same events; arbitrary bytes never panic and never buffer more than `max_frame + 4`. A decoder fuzz target runs on the pinned toolchain through proptest, because `cargo-fuzz` needs nightly.
- `panic = unwind` is set and asserted by a test, because the Swift wrapper relies on it.

### daemon-ipc.limits

Decision (6, 9). `Session::new(role, limits, now_ms)`, `receive`, `tick`, with time injected so every rule is deterministic.

- Caps: 4 KiB toward the daemon, 64 KiB toward the panel; 5 s to finish a frame; 60 s idle; close on the first invalid frame.
- Token bucket per connection, 10 messages per second and 1 state-changing message per second; over-limit messages are dropped and counted. `cancel_disarm` and `raise_protection` are exempt.
- A repeated request while one is pending is an `Event::Duplicate` that writes nothing.
- A pre-`hello` deadline, a partial-frame deadline, and a budget of consecutive `Unsupported` frames after which the session closes, so a peer cannot hold a connection or send unknown tags forever. The caller adds the connection cap.
- Liveness: an agent-role session that sends no heartbeat within its window yields `CloseReason::HeartbeatLost`, independent of the connection staying open.
- Counters are memory only and carry a count and a window, never a peer, path or message text; a test asserts the event types hold no string from a frame body.

### daemon-ipc.ffi

Decision (3). `packages/daemon-ipc-ffi` wraps `Session` as a `uniffi::Object` with `Mutex` state, following `packages/text-policy-ffi`. Every export returns `Result`; the first test makes a Rust panic and proves it arrives in Swift as a thrown error, which settles the unverified claim in the decision. Wire the crate into `build-ffi.sh` and `Package.swift`, generate the Swift bindings and typecheck them. No Swift call site uses `try!`.

### daemon-ipc.c-abi

Decision (3, 10). cbindgen is not installed and no Windows IPC code exists, so this waits for `win-daemon.ipc`. It exposes the same `Session` through a plain C ABI and replays the shared vectors from C++.

## What this does not cover

- Transport, peer authentication, code-signature checks, socket paths and permissions: the platform's.
- The cooldown, the list and the tamper log: the server's. The crate carries requests and verdicts, and decides none of them.
- Windows named-pipe ACLs and win-network's newline framing.
- A bug in the shared parser reaches both ends. Vectors are written by hand from the decision, so a round trip between the two ends cannot be the only test.
