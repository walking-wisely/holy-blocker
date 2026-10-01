# Holy Blocker Documentation

Plain Markdown under `docs/`. Generator-neutral — renderable on GitHub today, portable to MkDocs, Docusaurus, or VitePress later. Use relative links between pages. Do not embed sensitive blocklists, explicit eval cases, private datasets, or generated adult-content screenshots.

## Layout

### [`mission.md`](mission.md)
Why this project exists. The covenant rationale, Christian mission, and what that requires technically. Read this first.

### [`product/`](product/)
What the product is and how it behaves from a user perspective. Platform-neutral.

- [`product/outcomes.md`](product/outcomes.md) — user-facing capabilities, their honest status, and the packages that carry them
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
- [`aspect-skills-plan.md`](engineering/aspect-skills-plan.md) — plan for privacy/compliance, testing, and security agent skills
- [`coverage.md`](engineering/coverage.md) — the coverage ledger: what actually gets blocked, on which platform, by which layer, and what does not
- [`secrets-for-agents.md`](engineering/secrets-for-agents.md) — proposal: how agent-run end-to-end work gets credentials without `.env` files

### [`decisions/`](decisions/)
Architecture Decision Records — one file per significant discrete choice. Records what was decided, why, and what was rejected. Never deleted, only superseded.

### [`components/`](components/)
One folder per component, indexed in [`components/README.md`](components/README.md). Each has its own `README.md` (code location, status, decisions, what to read for which task), a `plan.md` (build order and step markers), a `steps.toml` ledger, and, for large components, a `modules/` folder with one specification per module. Start from the index, not from a plan.

### [`status/`](status/)
Per-package current state, one file per package, named `<area>-<package>.md`. This is where "what exists and what is verified" lives; `CLAUDE.md` carries only a one-line summary and a link.

## Where to look

| You want | Open |
|---|---|
| Why the project exists | [`mission.md`](mission.md) |
| What a component is and where its code lives | [`components/README.md`](components/README.md) → the component's `README.md` |
| The next step to build | `python -m tools.plan.ledger next docs/components/<component>` |
| A module's specification | `components/<component>/modules/` |
| What exists today and what is verified | [`status/`](status/), then [`engineering/coverage.md`](engineering/coverage.md) |
| Why a design choice was made | [`decisions/README.md`](decisions/README.md) |
| How a runtime behaviour plays out | [`product/flows/`](product/flows/) |
| Cross-component design | [`architecture/overview.md`](architecture/overview.md) |

Old review records and superseded snapshots live in a component's `archive/` folder and are not current state. The root [`PLAN.md`](../PLAN.md) is the original vision roadmap; it predates the component plans and is not a build order.

## Keeping it navigable

`python -m tools.plan.doclint` fails on a dead relative link or anchor, a component without a `README.md`, and a component missing from `components/README.md`. CI runs it on every docs change.
