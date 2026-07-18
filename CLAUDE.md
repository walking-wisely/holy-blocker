# AGENTS.md

Guidance for coding agents working in this repository.

## Project Shape

Holy Blocker is an on-device content blocking project. Keep the privacy and local-first model central when making changes: do not add cloud calls, telemetry, remote content analysis, or external dataset dependencies unless the user explicitly asks for them.

## Current State

The packages below **exist in the repo today** and are actively being built:

| Package | Language | Status |
|---|---|---|
| `apps/desktop` | TypeScript / Electron + React | Skeleton — BrowserWindow, one IPC stub, status UI |
| `packages/text-policy` | Rust | normalize + lexicon + verdict + scorer + evaluator + policy done; FFI surface done (see `packages/text-policy-ffi`) |
| `packages/text-policy-ffi` | Rust | UniFFI wrapper over text-policy — PolicyEngine + evaluate exposed; Kotlin bindings generated for Android |
| `apps/mobile` | Kotlin / Android | AccessibilityService text path MVP — policy core + ScanGate + overlay + onboarding done; text-policy wired in over UniFFI; SettingsGuard blocks the accessibility/App Info screens that would remove the guard, with a timed in-app disable as the exit path; all verified on an android-36 arm64 emulator. Plain Device Admin only — never design around Device Owner. Foreground service + VpnService/MediaProjection next |
| `packages/mitm-proxy` | Rust | Plain HTTP forwarding + TLS state/cert generation + CONNECT handler + HTTP/1.1 tunnel loop with phase 3/4/5 scan hooks done; text-policy wired into scan_url/scan_body; ProtectionMode next |
| `packages/net-shield` | Rust | radix domain/IP filter done; SNI parser done; tun adapter + PacketSink dispatch done; NetShield struct + run loop done (Windows Wintun path); smoke-test done — all 5 plan steps complete |
| `native-modules/win-daemon` | C++20 | WinEvent hooks + message loop; no capture/OCR/IPC yet |
| `machine-learning` | Python | MobileNetV3 model + ONNX export skeleton; no real training loop yet |

The packages below are **planned but not yet created** — do not assume they exist:

- `native-modules/win-network` — Windows Service: Wintun driver install, routing rules, named-pipe IPC for net-shield
- `packages/image-sandbox` — perceptual hashing + ONNX image classifier
- `packages/video-watchdog` — async HLS/DASH segment sampler

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

For Rust policy code:

- From `packages/text-policy`, use `cargo test` for tests.
- Use `cargo run` only when validating executable behavior.

For Python ML code:

- The package lives under `machine-learning/src/holy_blocker_ml`.
- Prefer small, importable functions over script-only code so behavior can be unit tested.
- If you add Python tests, place them under `machine-learning/tests` and wire a standard runner such as `pytest` before relying on it.

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

Docs are plain Markdown under `docs/`. Keep them generator-neutral and use relative links between pages. Do not add sensitive blocklists, private datasets, explicit evaluation samples, generated adult-content screenshots, or other private moderation artifacts to documentation.

Update docs when changing architecture, daemon responsibilities, classification flow, evaluation strategy, or public development workflows.

When a planned step in any `docs/components/<package>/plan.md` is completed, mark it done in that file (strike the item through and add **Done.**) and update the corresponding status row in the **Current State** table above. If the user asks to revert a completion marker, remove the strike-through and restore the original wording. This keeps the plans accurate without needing a separate sync pass.

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

## Verification Expectations

Before finishing a code change, run the narrowest relevant checks:

- Desktop TypeScript changes: `pnpm --filter @holy-blocker/desktop typecheck`
- Desktop build or bundling changes: `pnpm --filter @holy-blocker/desktop build`
- Rust policy changes: `cargo test` from `packages/text-policy`
- Python logic changes: run the package's unit tests, adding a test command if needed
- Native daemon changes: build with CMake and run any added unit tests

If a relevant check cannot be run, report the reason clearly.
