# CLAUDE.md

Guidance for coding agents working in this repository. This is the canonical file; `AGENTS.md` only points here.

## Project Shape

Holy Blocker is an on-device content blocking project. Keep the privacy and local-first model central when making changes: do not add cloud calls, telemetry, remote content analysis, or external dataset dependencies unless the user explicitly asks for them.

## Routing — which loop applies

Classify every request **before touching code**, and say the classification in one line. A request
rarely names its loop; the agent picks it. Follow the first branch that matches:

1. **Nothing covers it, and it adds a capability, changes product behaviour, or makes an
   architecture choice** → run `plan-inception`. It writes a decision record and a `plan.md`, and
   never implements. Do not assume a new feature fits an existing plan.
2. **It matches a step marker** (`<!-- step: ... -->`) in a `docs/components/**/steps.toml`, or the
   request is "next step" / "continue X" → run `step-loop`. Find the step with
   `python -m tools.plan.ledger next docs/components/<component>`, or across every worktree with
   `python -m tools.plan.todos`.
3. **It is a defect.** Reproduce it first. If it regresses a `done` step, file a `kind = "bug"` step
   with `regressed_step` set, then run `step-loop` on it. A fix that is the same file and the same
   test suite as the change in hand is made inline.
4. **No step matches, but the change moves a row in `docs/engineering/coverage.md`** → add the step
   first (through `plan-inception` if it needs a decision, otherwise directly in the component's
   `plan.md` and `steps.toml`), then run `step-loop`.
5. **Anything else** → classify it with the tier table below.

If two tiers both fit, take the stricter one and say so. Never pick a looser tier because the
change feels small.

| Tier | What | Merge |
|---|---|---|
| Full loop (`step-loop`) | any manifest step; any file a `coverage.md` row cites; anything touching preload/IPC, TLS/CA, TUN, named pipes, capture/OCR or OS permissions; every full-loop step gets the security and privacy reviews unconditionally | auto only if the step's `acceptance = "code"`; an `observation` step merges only if a scenario observed it, else it stops at the PR; `product` steps stop at the PR |
| Loop-lite | docs-only, dependency bumps, mechanical refactor, frontend rendering | normal PR; run the package's own check and update `coverage.md` if a row moves |
| Exempt | typo fixes; throwaway spikes that never leave the worktree | none |

Where the owner sits: the owner decides product questions, and architecture questions that are ML,
cross-platform contracts, new trust boundaries or one-way doors; the agent red-teams those first
(`docs/decisions/decision-tiers-and-red-teaming.md`). The owner does not review code or
implementation detail, and meets a feature once, when it is done, through its demo
(`docs/decisions/feature-demos-and-local-e2e.md`). Privileged steps on the dev machine go through
the dispatcher in `docs/decisions/agent-privilege-boundary.md`, never a root shell.

The router is a convention, but its output is machine-checked: the `Loop contract` CI job
(`python -m tools.plan.loop check`) fails a PR that touches a manifest's `paths` without a step id
and non-empty `## Assumption audit` and `## Adversarial review` sections.

## Current State

The packages below **exist in the repo today** and are actively being built:

Full per-package status lives in `docs/status/<package>.md` — read the one for the package you are working on.

| Package | Language | Status |
|---|---|---|
| `apps/desktop` | TypeScript / Electron + React | Skeleton — BrowserWindow, one IPC stub, status UI — [details](docs/status/apps-desktop.md) |
| `packages/text-policy` | Rust | Policy core done; consumed over UniFFI by Android and the mac daemon — [details](docs/status/packages-text-policy.md) |
| `packages/text-policy-ffi` | Rust | UniFFI wrapper (Kotlin + Swift); its API is a cross-platform contract — [details](docs/status/packages-text-policy-ffi.md) |
| `apps/mobile` | Kotlin / Android | Android text, VPN/DNS and MediaProjection paths verified on emulator; LiteRT image path is next — [details](docs/status/apps-mobile.md) |
| `packages/mitm-proxy` | Rust | HTTPS interception with text and image scan hooks and ProtectionMode; two known gaps — [details](docs/status/packages-mitm-proxy.md) |
| `packages/image-sandbox` | Rust | v0 done: decode → tile → ONNX → max → verdict; thresholds are caller-supplied — [details](docs/status/packages-image-sandbox.md) |
| `packages/net-shield` | Rust | Domain/IP filter, SNI, Wintun path, DNS path and signed blocklist loading done — [details](docs/status/packages-net-shield.md) |
| `packages/net-shield-ffi` | Rust | UniFFI wrapper over the DNS path; ships a placeholder rule set — [details](docs/status/packages-net-shield-ffi.md) |
| `packages/domain-normalize` | Rust | Module 0 done: normalize + scope classification — [details](docs/status/packages-domain-normalize.md) |
| `packages/domain-blocklist` | Rust | Module 2 (merge) done; other modules per plan — [details](docs/status/packages-domain-blocklist.md) |
| `packages/image-sandbox-ffi` | Rust | UniFFI wrapper over the raw-frame path; ONNX half of the runtime split — [details](docs/status/packages-image-sandbox-ffi.md) |
| `native-modules/win-daemon` | C++20 | WinEvent hooks + message loop; no capture/OCR/IPC yet — [details](docs/status/native-modules-win-daemon.md) |
| `native-modules/mac-daemon` | Swift / SwiftPM | Layer 1 done; Layer 2 text and image paths working end to end; revocation unobserved — [details](docs/status/native-modules-mac-daemon.md) |

The packages below are **planned but not yet created** — do not assume they exist:

- `packages/classifier-head` (+ `packages/classifier-head-ffi`) — the classifier head (embedding → score → verdict) in Rust behind UniFFI, plus a later gradient path. Fully specified in `docs/components/classifier-head/plan.md` (cut point, weights format, parity-fixture rules). It exists because the runtime split is per-platform: LiteRT runs the frozen backbone on Android, ONNX Runtime on Windows, and only the head is shared — see `docs/decisions/learning-from-feedback.md`. Do not wire `apps/mobile` to `packages/image-sandbox`, which is the ONNX half. No threshold ships in the crate; a threshold belongs to a (checkpoint, geometry) pair
- `native-modules/win-network` — Windows Service: Wintun driver install, routing rules, named-pipe IPC for net-shield
- `packages/video-watchdog` — async HLS/DASH segment sampler
- `machine-learning` — MobileNetV3 fine-tuning, evaluation, and ONNX/TFLite export pipeline for the frozen backbone `packages/image-sandbox` and `packages/classifier-head` consume. Planned in `docs/components/machine-learning/plan.md`. A prior implementation existed and was removed; nothing under `machine-learning/` should be assumed to exist

`native-modules/android-service` was planned but never created — the Android work shipped as `apps/mobile/` instead, and it targets plain Device Admin rather than the Device Owner model the old plan assumed. `docs/components/android-service/plan.md` is a superseded stub pointing at `docs/components/mobile/plan.md`; do not build against it.

Each active package has a step-by-step implementation plan in `docs/components/<package>/plan.md`. Read the relevant plan before starting work on a package — it lists the next modules to add, their types, and the correct implementation order.

Current major areas:

## Development Commands

Use `pnpm` for the JavaScript workspace.

- Install JS dependencies: `pnpm install`
- Run the desktop app: `pnpm dev:desktop`
- Build all JS workspace packages: `pnpm build`
- Typecheck all JS workspace packages: `pnpm typecheck`
- Build the desktop package only: `pnpm --filter @holy-blocker/desktop build`
- Typecheck the desktop package only: `pnpm --filter @holy-blocker/desktop typecheck`

### Local Ports and Services (Portless is the default)

This repo runs many parallel git worktrees (see "Working Rhythm" below), and a hardcoded port
number is a collision waiting to happen the moment two worktrees run the same dev server or
proxy at once. **[Portless](https://github.com/vercel-labs/portless)** (`portless run -- <cmd>`)
is the default way to run anything that binds a local TCP port:

- It assigns a free ephemeral port via the `PORT` env var and exposes the resulting address as
  `PORTLESS_URL`, so nothing in a script or config should hardcode a port number.
- It detects git worktrees automatically and namespaces the URL by branch, so `pnpm dev:desktop`
  in two different worktrees never collides.
- One-time per machine, not per worktree: the Portless proxy daemon must be started once before
  first use, and `portless run` cannot start it itself non-interactively (it wants `sudo` for a
  privileged port and there's no TTY to prompt). Run `portless proxy start --port 1355 --https`
  once — this also installs Portless's CA into the system trust store, which is what lets
  Electron/Chromium and `curl` trust `*.localhost` HTTPS without extra config.
- `apps/desktop`'s `pnpm dev` script and `vite.config.ts` already use this pattern
  (`process.env.PORT`, `$PORTLESS_URL`) — follow the same shape for any new package that binds a
  TCP port for local development or manual testing (e.g. `packages/mitm-proxy`'s `--listen`
  flag: run it as `portless run --name mitm-proxy -- sh -c 'cargo run -- --listen
  127.0.0.1:$PORT'` rather than the hardcoded `127.0.0.1:8080` default).
- Portless auto-injects `--port` for frameworks that ignore `PORT` (Vite, Astro, etc.), but this
  only works when the framework binary is the literal top-level command — not when it's a quoted
  sub-command inside `concurrently` or another shell wrapper. In that case (as with
  `apps/desktop`), have the underlying tool read `process.env.PORT` itself instead of relying on
  auto-injection.
- **Quoting trap, hit live while wiring `apps/desktop`:** a `$PORTLESS_URL` reference inside a
  *double*-quoted sub-command string (e.g. one of `concurrently`'s quoted args) gets expanded
  immediately by the outer shell that runs the whole `pnpm` script — before `portless run` has
  even started and set the variable — so it silently resolves to an empty string. Single-quote
  any segment that needs `$PORTLESS_URL`/`$PORT` so expansion is deferred to the child shell
  `concurrently` (or whatever wraps it) spawns later, once Portless has actually set the env var.
- **Verified live 2026-09-19:** `pnpm dev:desktop` under this scheme renders the real Electron
  window (title "Holy Blocker", the Monitor/Local Data UI) loaded from
  `https://<worktree>.desktop.localhost:1355` with no certificate warning — Electron trusts
  Portless's CA via the system trust store the same way `curl` does; no `NODE_EXTRA_CA_CERTS`
  wiring was needed on the Electron side. The `--no-tls` fallback is not needed.
- Portless is macOS/Linux only. `native-modules/win-daemon`/`win-network` will need a different
  mechanism (e.g. binding port 0 and passing the OS-assigned port through env/config) once they
  grow a local TCP listener.
- **Docker Compose is not covered by `portless run`** — it only wraps a single launched process
  and cannot inject a port into a static `ports:` mapping in a compose file. If a package
  introduces Compose: never hardcode `container_name:` (let it default from the worktree's
  directory name, which is already unique), bind host ports as `ports: ["127.0.0.1::5432"]` (no
  host port — Docker assigns a free one), discover it with `docker compose port <service>
  <container_port>`, and register it with `portless alias <name> <port>` so callers still get a
  stable hostname instead of needing to know the number.

For Rust policy code:

- From `packages/text-policy`, use `cargo test` for tests.
- Use `cargo run` only when validating executable behavior.

For Python ML code:

- Not yet built. See `docs/components/machine-learning/plan.md` for the intended package layout under `machine-learning/src/holy_blocker_ml`.
- Prefer small, importable functions over script-only code so behavior can be unit tested.
- Place tests under `machine-learning/tests` and wire a standard runner such as `pytest` before relying on it.

For the Windows daemon:

- Build with CMake from `native-modules/win-daemon`.
- Keep platform APIs isolated from portable decision logic where practical, so pure behavior can be unit tested separately from Win32 event plumbing.

## Test-First Rule For Logic

For any new business-logic function, write focused unit tests first, then implement the function. This applies especially to:

- classification thresholds and policy decisions;
- text matching, normalization, scoring, or allow/block decisions;
- ML pipeline configuration and artifact-selection logic;
- daemon event filtering, debouncing, IPC message shaping, and state transitions;
- Electron main/preload logic that affects daemon status, local data, or policy decisions.

Frontend-only rendering changes do not need test-first treatment by default, but extracted non-UI logic should still get unit tests.

When a test framework is missing, add the smallest appropriate test setup for the package you are changing instead of leaving new logic untested. Keep tests deterministic and avoid private datasets, explicit sensitive corpora, screenshots, or generated adult-content fixtures in the public repo.

## Code Conventions

- Preserve the existing language boundaries. Do not move daemon, ML, policy, or UI responsibilities into another layer without a clear reason.
- Keep code local-first. Avoid network access in runtime paths unless explicitly requested.
- Prefer pure functions for policy and classification decisions. Put side effects at the edges.
- Keep Electron security settings strict: preserve context isolation and avoid enabling Node integration in the renderer.
- In the renderer, follow the existing React + TypeScript style and use `lucide-react` icons where icons are needed.
- In Rust, keep policy logic in testable modules instead of burying it in `main`.
- In Python, keep training/export orchestration thin and move reusable behavior into importable functions.
- In C++, keep Win32 callback glue small and move decision logic into testable helpers when the daemon grows.

## Specification References

Any code that implements a network protocol, binary wire format, or OS-level interface must be traceable to its authoritative specification. This applies to packet parsers, TLS record handling, IP/TCP header field offsets, IANA registry values, Win32 API contracts, and similar low-level work.

**In code:** every magic number, byte offset, or field layout must have an inline comment citing the document and section it comes from — for example `// RFC 791 §3.1` or `// Wintun API docs — Session::receive_blocking`. Name constants instead of repeating literals, and put the citation on the constant.

**In plan files (`docs/components/<package>/plan.md`):** each module section that touches wire formats or OS interfaces must include a "Reference documents" subsection listing the specs an implementer needs to read. Link to the canonical online version of each document so it can be consulted directly:

- IETF RFCs: `https://www.rfc-editor.org/rfc/rfcNNNN` (the RFC Editor HTML version is easier to navigate than the plain-text original).
- IANA registries: link the specific registry page, not just the top-level site.
- Microsoft Win32/WinRT docs: link the specific `learn.microsoft.com` page for the API or concept.
- Wintun: `https://www.wintun.net` and the repository README at `https://github.com/WireGuard/wintun`.

When in doubt about a field value or offset, consult the linked document rather than inferring from existing code. The web versions of RFCs are searchable and have anchor links per section.

## Documentation

Navigate docs top-down, not by search: [`docs/README.md`](docs/README.md) → [`docs/components/README.md`](docs/components/README.md) → the component's own `README.md`, which says what to read for which kind of task. Large plans keep their step markers in `plan.md` and their module specifications under `modules/`; a component with no `README.md` fails `doclint` in CI.

Docs are plain Markdown under `docs/`. Keep them generator-neutral and use relative links between pages. Do not add sensitive blocklists, private datasets, explicit evaluation samples, generated adult-content screenshots, or other private moderation artifacts to documentation.

Update docs when changing architecture, daemon responsibilities, classification flow, evaluation strategy, or public development workflows.

When a planned step in any `docs/components/<package>/plan.md` is completed, mark it done in that file (strike the item through and add **Done.**) and update that package's `docs/status/<package>.md` (the **Current State** table above carries only a one-line summary and a link; change the summary only when the headline changes, so concurrent PRs do not collide on one row). If the user asks to revert a completion marker, remove the strike-through and restore the original wording. This keeps the plans accurate without needing a separate sync pass.

## Branch and Commit Conventions

Branch names follow the pattern `<prefix>/<short-slug>` where the slug is a brief kebab-case description of the work. The fuller description lives in the first commit message. Use these prefixes:

| Prefix | When to use |
|---|---|
| `feat` | new feature or capability |
| `fix` | bug fix |
| `refactor` | restructuring without behaviour change |
| `infra` | build system, CI, tooling, scaffolding |

Examples:
- `feat/net-shield-basic-impl`
- `fix/mitm-proxy-tls-cert-chain`
- `refactor/text-policy-scorer-module`
- `infra/cargo-workspace`

Commit messages follow the same prefix convention with the conventional-commits format:
`<prefix>(<scope>): <imperative summary>`

The body should explain *why* the change was made — the what is visible in the diff. Keep the subject line under 72 characters.

## Working Rhythm — Worktree In, PR Out

Once the [router](#routing--which-loop-applies) has classified the work, every piece of it that is
not exempt starts in its **own git worktree** and ends as an **open pull request**, with no
prompting needed for either. Both halves are the default, not a thing to propose and wait on.

### Start: a separate worktree, branched from the base branch

Do not work in the main checkout, and do not branch from whatever happens to be checked out. Create
a worktree on a new branch off the branch that carries the changes this work depends on:

```
git worktree add .claude/worktrees/<slug> -b <prefix>/<slug> <base>
```

`<base>` is **almost always `master`** (this repository's default branch). Use a different base only
when this work genuinely builds on an unmerged branch — for example a session that needs an earlier
session's commits. **If which base is correct is ambiguous, ask before creating the worktree**; the
cost of asking is one question, and the cost of guessing is a branch that has to be redone.

Run every command from the worktree. Note that the git stash stack is shared across worktrees, so
prefer a temporary WIP commit over `git stash`.

### Finish: commit, push, and open the PR immediately

When the work is complete and the [verification checks](#verification-expectations) pass, go
straight through to a PR in one pass — commit, `git push -u origin <branch>`, `gh pr create` — and
then report what was done. Do not stop after the commit to ask whether to push, and do not stop
after the push to ask whether to open the PR. "Complete" means the whole task is done and checked,
including the plan and status-table updates the [Documentation](#documentation) section requires.

**The one exception:** if the request asked to review, inspect, or hold the change before it goes
out — anything of the shape "let me look first", "don't push yet", "just commit" — stop where asked
and leave the rest to them. An earlier request to hold does not carry over to later, separate work.

If a check fails or something in the work is genuinely blocked, that is not "complete": say so
instead of opening a PR over it.

## Tooling — Plan Management Scripts

Internal Python tools under `tools/plan/` drive the worktree lifecycle and step-tracking. Run them
from the repo root. **Always pull master before running any of these**:

```
git pull origin master
```

### Worktree lifecycle

List worktrees and their classification (merged, abandoned, open, dirty, etc.):

```
python -m tools.plan.worktrees report
```

Reap merged/abandoned worktrees (dry run first, then add `--yes`):

```
python -m tools.plan.worktrees reap            # dry run
python -m tools.plan.worktrees reap --yes      # actually remove
python -m tools.plan.worktrees reap --yes --force   # also reap worktrees with ignored state
```

Branches are never deleted by the reaper — that decision is left to a human.

### Pending-step discovery

Scan every worktree for `steps.toml` manifests and report pending steps alongside each
worktree's health state:

```
python -m tools.plan.todos               # pending steps only
python -m tools.plan.todos --all         # all steps (pending + done)
python -m tools.plan.todos --blockers    # only worktrees with issues (dirty, open, etc.)
```

### Per-package step ledger

Validate that a package's `steps.toml` is consistent with its `plan.md`, render the step
table, or find the next actionable step:

```
python -m tools.plan.ledger validate docs/components/text-policy
python -m tools.plan.ledger render   docs/components/text-policy
python -m tools.plan.ledger next     docs/components/text-policy
```

Run `validate` on any package whose steps you modify, and run `next` before starting a new
implementation session to confirm what is unblocked.

## Verification Expectations

Before finishing a code change, run the narrowest relevant checks:

- Desktop TypeScript changes: `pnpm --filter @holy-blocker/desktop typecheck`
- Desktop build or bundling changes: `pnpm --filter @holy-blocker/desktop build`
- Rust policy changes: `cargo test` from `packages/text-policy`
- Python logic changes: run the package's unit tests, adding a test command if needed
- Windows daemon changes: build with CMake and run any added unit tests
- Docs changes (any `docs/**` or root `*.md` edit): `python -m tools.plan.doclint` from the repo root
- macOS daemon changes: `./scripts/test.sh` from `native-modules/mac-daemon`. Do **not** use bare
  `swift test` — under a Command Line Tools toolchain it cannot locate swift-testing, and moving
  the needed flags into `Package.swift` as `unsafeFlags` makes SwiftPM silently run zero tests
  while still exiting 0.

If a relevant check cannot be run, report the reason clearly.

## Before And After The Code

Two skills bracket implementation work, and they exist because of a measured pattern: modules here
rest on claims about the outside world, and when those claims go unverified they are wrong often
enough to invalidate the module rather than a detail of it.

- **Before implementing a plan step**, run the `assumption-audit` skill. It enumerates the external
  facts the step depends on, attaches one falsifying command to each, runs them, and reports before
  any code is written. Its case log records the instances that motivated it — a `Verdict::Dead` made
  unreachable by NSEC3 opt-out, a capture path defeated by a default pixel format, `Info.plist` keys
  that do not exist, two wrong constants in one module.
- **After writing code**, run the `adversarial-review` skill on the diff, branch, or PR. Its
  catalogue is this repository's own recurring failure modes, ordered by frequency, with the two
  rules that make a review trustworthy: reproduce or label, and cite or omit.

**Any change that alters what a layer covers updates
[`docs/engineering/coverage.md`](docs/engineering/coverage.md) in the same PR.** Each component here
is scoped narrowly and honestly and records its own narrowing; the ledger is the only place the
union is computed, and the union is the product. `Unverified` is promoted to `Covered` by an
observation, never by an argument.
