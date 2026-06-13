# Holy Blocker Documentation

Plain Markdown under `docs/`. Generator-neutral — renderable on GitHub today, portable to MkDocs, Docusaurus, or VitePress later. Use relative links between pages. Do not embed sensitive blocklists, explicit eval cases, private datasets, or generated adult-content screenshots.

## Layout

### [`mission.md`](mission.md)
Why this project exists. The covenant rationale, Christian mission, and what that requires technically. Read this first.

### [`product/`](product/)
What the product is and how it behaves from a user perspective. Platform-neutral.

- [`product/flows/`](product/flows/) — runtime behavior traces: what happens step by step when a block fires, a warn interstitial appears, an override is attempted, etc.

### [`architecture/`](architecture/)
How the system is designed technically — cross-component concerns that no single component owns.

- [`overview.md`](architecture/overview.md) — component map, two-path runtime model, local-first privacy constraint
- [`network-pipeline.md`](architecture/network-pipeline.md) — five-phase interception path spanning net-shield, mitm-proxy, text-policy, image-sandbox, video-watchdog
- [`content-classification.md`](architecture/content-classification.md) — how image classification, OCR, and text policy work together
- [`edge-daemons.md`](architecture/edge-daemons.md) — Windows and Android daemon strategy

### [`engineering/`](engineering/)
How the codebase is built, tested, and maintained. Audience: contributors.

- [`evaluation-and-ci.md`](engineering/evaluation-and-ci.md) — eval layers, CI tiers, private eval pack strategy
- [`security-backlog.md`](engineering/security-backlog.md) — secure SDLC, trust-boundary hardening, release-integrity work

### [`decisions/`](decisions/)
Architecture Decision Records — one file per significant discrete choice. Records what was decided, why, and what was rejected. Never deleted, only superseded.

### [`components/`](components/)
One folder per component. Each contains a `plan.md` (implementation phases, current state, what's next) and platform-specific files where the implementation meaningfully differs per OS.

| Component | Language | Plan |
|---|---|---|
| `text-policy` | Rust | [plan.md](components/text-policy/plan.md) |
| `mitm-proxy` | Rust | [plan.md](components/mitm-proxy/plan.md) |
| `net-shield` | Rust | [plan.md](components/net-shield/plan.md) |
| `image-sandbox` | Rust | [plan.md](components/image-sandbox/plan.md) |
| `video-watchdog` | Rust | [plan.md](components/video-watchdog/plan.md) |
| `win-daemon` | C++20 | [plan.md](components/win-daemon/plan.md) |
| `win-network` | C++ / Windows Service | [plan.md](components/win-network/plan.md) |
| `machine-learning` | Python | [plan.md](components/machine-learning/plan.md) |
| `desktop` | TypeScript / Electron | [plan.md](components/desktop/plan.md) |
| `backend` | Go | [plan.md](components/backend/plan.md) |
| `frontend` | TypeScript / React | [plan.md](components/frontend/plan.md) |
| `voice-gate` | Rust + platform adapters | [plan.md](components/voice-gate/plan.md) |
