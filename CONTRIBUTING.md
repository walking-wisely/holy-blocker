# Contributing to Holy Blocker

Thank you for your interest in contributing. This document covers everything you need to get oriented and submit good work.

---

## Understanding the project first

Before writing any code, read these two documents:

1. [docs/foundation.md](docs/foundation.md) — *why* this project exists and the values that shape its design decisions
2. [docs/architecture.md](docs/architecture.md) — the two-path blocking model and the overall component layout

The theological rationale in `foundation.md` is not incidental. It directly determines the scope of what gets blocked, how the accountability model works, and why certain trade-offs (like local-only decisions) are non-negotiable. Design suggestions that conflict with those principles will not be accepted regardless of their technical merit.

---

## How to pick up work

Each active package has a step-by-step implementation plan:

| Plan | Package |
|---|---|
| [docs/text-policy/PLAN.md](docs/text-policy/PLAN.md) | Rust text classification engine |
| [docs/proxy/PLAN.md](docs/proxy/PLAN.md) | Rust MITM proxy |
| [docs/win-daemon/PLAN.md](docs/win-daemon/PLAN.md) | C++ Windows daemon |
| [docs/machine-learning/PLAN.md](docs/machine-learning/PLAN.md) | Python ML pipeline |
| [docs/desktop/PLAN.md](docs/desktop/PLAN.md) | Electron control panel |
| [docs/net-shield/PLAN.md](docs/net-shield/PLAN.md) | Rust TUN packet filter |
| [docs/image-sandbox/PLAN.md](docs/image-sandbox/PLAN.md) | Rust image classifier |
| [docs/video-watchdog/PLAN.md](docs/video-watchdog/PLAN.md) | Rust video segment sampler |

Each plan lists the next modules to add, their types and responsibilities, and the correct implementation order. Start with the plan, not the code.

When you complete a step, mark it done in the plan file (strike through the item, add **Done.**).

---

## Branch conventions

Branch names follow the pattern `<prefix>/<short-slug>`:

| Prefix | When to use |
|---|---|
| `feat` | new feature or capability |
| `fix` | bug fix |
| `refactor` | restructuring without behaviour change |
| `infra` | build system, CI, tooling, scaffolding |

Examples: `feat/net-shield-basic-impl`, `fix/mitm-proxy-tls-cert-chain`, `infra/cargo-workspace`

---

## Commit style

Commits follow [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<prefix>(<scope>): <imperative summary>
```

- Subject line: under 72 characters, imperative mood ("add", "fix", "remove" — not "added" or "fixes")
- Body: explain *why* the change was made, not *what* it does (the diff shows that)
- Scope: the package or module affected (e.g. `text-policy`, `mitm-proxy`, `win-daemon`, `desktop`)

Examples:
```
feat(net-shield): add radix-tree domain filter with CIDR support
fix(mitm-proxy): handle CONNECT tunnels that send data before 200 OK
refactor(text-policy): extract scorer into its own module
```

---

## Test-first rule for logic

For any new business-logic function, write focused unit tests first, then implement the function. This applies to:

- Classification thresholds and policy decisions
- Text matching, normalization, scoring, or allow/block decisions
- ML pipeline configuration and artifact-selection logic
- Daemon event filtering, debouncing, IPC message shaping, and state transitions
- Electron main/preload logic that affects daemon status, local data, or policy decisions

Frontend-only rendering changes do not need test-first treatment by default, but any extracted non-UI logic should still get unit tests.

When a test framework is missing, add the smallest appropriate test setup for the package you are changing instead of leaving new logic untested.

---

## Code conventions

- **Local-first**: no network access in runtime blocking paths. All decisions happen on-device.
- **Language boundaries**: do not move daemon, ML, policy, or UI responsibilities across layers without a clear reason stated in the PR description.
- **Pure functions for policy**: keep classification and rule decisions in pure functions. Put side effects at the edges.
- **Electron security**: preserve context isolation. Do not enable Node integration in the renderer.
- **Rust**: keep policy logic in testable modules instead of burying it in `main`.
- **Python**: keep training/export orchestration thin and move reusable behavior into importable functions.
- **C++**: keep Win32 callback glue small and move decision logic into testable helpers.
- **Comments**: write no comments by default. Only add one when the *why* is non-obvious — a hidden constraint, a subtle invariant, a workaround for a specific external bug.
- **Sensitive data**: do not commit blocklists, private evaluation corpora, explicit test fixtures, or generated adult-content screenshots to this repository.

---

## Verification before submitting

Run the narrowest relevant checks for your change:

| Change type | Command |
|---|---|
| Desktop TypeScript | `pnpm --filter @holy-blocker/desktop typecheck` |
| Desktop build | `pnpm --filter @holy-blocker/desktop build` |
| Rust packages | `cargo test` from the relevant package directory |
| Python ML | `pytest` from `machine-learning/` |
| Windows daemon | `cmake -B build && cmake --build build` from `native-modules/win-daemon` |

If a relevant check cannot be run in your environment, say so clearly in the PR description — do not skip it silently.

---

## Privacy constraint

This is a hard rule: **do not add cloud calls, telemetry, remote content analysis, or external dataset dependencies** to the runtime blocking path. The system's value proposition depends entirely on no user content leaving the device. Any contribution that violates this will be rejected.
