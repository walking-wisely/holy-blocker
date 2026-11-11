# `packages/daemon-ipc` — status

Language: Rust

Planned, not yet created. The decision is accepted ([daemon-ipc-and-process-model](../decisions/daemon-ipc-and-process-model.md)) and the plan is in [`docs/components/daemon-ipc/plan.md`](../components/daemon-ipc/plan.md): `daemon-ipc.core`, `daemon-ipc.limits`, `daemon-ipc.ffi` for the MVP and `daemon-ipc.c-abi` for Windows later.

Today the framing exists twice, in `native-modules/mac-daemon/Sources/MacDaemon/DaemonIPC.swift` and in `apps/desktop/src-tauri/src/{frame,backoff,message,link}.rs`, and the Rust vectors are read by no Swift test. Nothing is observed against a real agent socket.
