# Components

One folder per component. Open the component's `README.md` first: it names the code location, the status page, the decisions it rests on, and which file to read for which kind of task. The `plan.md` beside it is the build order and carries the step markers that `steps.toml` tracks; large plans keep each module's specification under `modules/`.

Which step is next, for any component:

```
python -m tools.plan.ledger next docs/components/<component>
```

Exists today means code is in the repo. Detailed state lives in [`../status/`](../status/) and, for what is actually blocked and where, [`../engineering/coverage.md`](../engineering/coverage.md).

| Component | Language | Code | One line |
|---|---|---|---|
| [text-policy](text-policy/README.md) | Rust | `packages/text-policy*` | Text scoring engine, shared over UniFFI |
| [mitm-proxy](mitm-proxy/README.md) | Rust | `packages/mitm-proxy` | HTTPS interception with text and image hooks |
| [net-shield](net-shield/README.md) | Rust | `packages/net-shield*` | Packet, SNI and DNS filtering |
| [image-sandbox](image-sandbox/README.md) | Rust | `packages/image-sandbox*` | Image classification path |
| [domain-blocklist](domain-blocklist/README.md) | Rust | `packages/domain-blocklist`, `domain-normalize` | Offline blocklist build pipeline |
| [classifier-head](classifier-head/README.md) | Rust | planned | Shared head over per-platform backbones |
| [video-watchdog](video-watchdog/README.md) | Rust | planned | HLS/DASH segment sampler |
| [machine-learning](machine-learning/README.md) | Python | `machine-learning/` | Evaluation and export for the backbone |
| [mac-daemon](mac-daemon/README.md) | Swift | `native-modules/mac-daemon` | macOS network and render paths |
| [win-daemon](win-daemon/README.md) | C++20 | `native-modules/win-daemon` | Windows event hooks and scan loop |
| [win-network](win-network/README.md) | C++20 | planned (`native-modules/win-network`, not yet created) | Wintun Windows Service |
| [mobile](mobile/README.md) | Kotlin | `apps/mobile` | Android guard, VPN and capture |
| [desktop](desktop/README.md) | TypeScript | `apps/desktop` | Electron control panel |
| [voice-gate](voice-gate/README.md) | TypeScript | planned | Read-aloud override gate |
| [backend](backend/README.md) | Go | planned | Partner invites and notifications |
| [frontend](frontend/README.md) | TypeScript | planned | Public web app |
| [android-service](android-service/README.md) | — | none | Superseded by `mobile` |
