# mobile

Android app: AccessibilityService text path, DNS VPN, MediaProjection capture, tamper resistance on plain Device Admin.

| | |
|---|---|
| Code | `apps/mobile/` (Kotlin). |
| Status | [apps-mobile](../../status/apps-mobile.md) |
| Next step | `python -m tools.plan.ledger next docs/components/mobile` |
| Decision | [content-interception](../../decisions/content-interception.md) "Android — the layer order inverts" |
| Depends on | [text-policy](../text-policy/README.md) and [net-shield](../net-shield/README.md) over UniFFI; [classifier-head](../classifier-head/README.md) for the image path |
| Constraint | Plain Device Admin only. Never design around Device Owner. |

## Files here

- [`plan.md`](plan.md) — Why Android inverts the desktop order, current state, FFI dependency, step markers, verification, open questions
- [`modules/`](modules/) — One spec per module: 01 policy through 09 the-image-path
- [`gotchas.md`](gotchas.md) — Platform traps learned on device
- [`backlog.md`](backlog.md) — Open defects and deferred hardware-blocked items
- [`steps.toml`](steps.toml) — Step ledger

## Read for

- Implementing a module: its file under `modules/`.
- Before touching an Android API: `gotchas.md`.
- Hardware-only items (Recents, OEM screens): `backlog.md`.
