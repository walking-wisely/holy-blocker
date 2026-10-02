# mac-daemon

Swift daemon for macOS. Layer 1: system proxy and CA trust for the MITM proxy. Layer 2: screen capture, accessibility text, overlay, window suppression.

| | |
|---|---|
| Code | `native-modules/mac-daemon/` (Swift / SwiftPM). |
| Status | [native-modules-mac-daemon](../../status/native-modules-mac-daemon.md) |
| Next step | `python -m tools.plan.ledger next docs/components/mac-daemon` |
| Decision | [content-interception](../../decisions/content-interception.md) "macOS — what carries over and what changes" |
| Architecture | [edge-daemons](../../architecture/edge-daemons.md) |
| Run tests with | `scripts/test.sh` from the package, never bare `swift test` |

## Files here

- [`plan.md`](plan.md) — Scope, build environment, step markers, implementation order and the live e2e sessions
- [`modules/`](modules/) — One spec per module: 00 signed-bundle, 01–06 Layer 1, 07–16 and 18 Layer 2
- [`layer-1-limits.md`](layer-1-limits.md) — What Layer 1 does not cover on macOS
- [`layer-2-limits.md`](layer-2-limits.md) — What Layer 2 does not cover on macOS
- [`backlog.md`](backlog.md) — Open defects and unobserved coverage
- [`signing-identity.md`](signing-identity.md) — Runbook for the dev signing certificate
- [`steps/`](steps/) — Step ledger, one file per step

## Read for

- Implementing a module: its file under `modules/`.
- TCC, signing or grant problems: `modules/00-signed-bundle.md` and `signing-identity.md`.
- What is and is not protected: the two `layer-N-limits.md` files, then [coverage](../../engineering/coverage.md).
