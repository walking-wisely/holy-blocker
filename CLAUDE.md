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
| `apps/mobile` | Kotlin / Android | AccessibilityService text path MVP — policy core + ScanGate + overlay + onboarding done; text-policy wired in over UniFFI; SettingsGuard blocks the accessibility/App Info screens that would remove the guard, but **only while the user has armed the in-app protection mode** — `ProtectionSchedule`/`ProtectionStore` hold that state, and disarming is request → 15-min cooldown → confirm → 10-min window that re-arms itself; device admin active, so uninstall is refused until it is deactivated; all verified on an android-36 arm64 emulator. Plain Device Admin only — never design around Device Owner. The empty-harvest gap is closed: the device-admin list rows are `accessibilityDataSensitive` and needed `android:isAccessibilityTool="true"`, without which the guard is handed chrome and no rows — see `docs/components/mobile/backlog.md`. Split screen is handled: every watched window is evaluated, and `GLOBAL_ACTION_BACK` — which takes no window argument — is gated on the matched window holding input focus, covering instead when it does not; verified in real split screen on the emulator, and note an unfocused pane emits no accessibility events at all. The tamper log is done — `policy/TamperLog.kt` (pure) + `TamperLogStore.kt` (the `filesDir` edge) record guard-state transitions and removal attempts, and classify a session that ended without an unbind as an unclean stop; verified on an android-36 emulator against a force-stop, a clean off/on, and a fresh install. It records the guard, never the screen. Recents is **deferred, blocked on real One UI/HyperOS hardware** — the swipe-kill claim cannot be reproduced on an emulator, so no mitigation can be verified. The foreground service is done — `policy/GuardStatus.kt` (pure) + `GuardStatusService.kt` + `BootReceiver.kt`: an ongoing `specialUse` FGS that is the status surface **and** the still-alive process that records a disable the guard cannot report (`adb`, guest, an OEM disable screen we do not recognise), closing the other half of the backlog's undetectable-bypass item. It does **not** keep the guard alive — the system rebinds an enabled accessibility service by itself. `TamperLog.classifyConnect` now takes a tail of entries and anchors on the last session boundary, because the status service writes between sessions and a single-line read turned every clean off/on into an apparent kill. `BootReceiver` writes nothing: `ACTION_BOOT_COMPLETED` is delivered when the app leaves the force-stopped state, so a boot receiver must never be what identifies a boot. Verified on an android-36 emulator against an adb disable, a reboot, a force-stop, and an idle install. VpnService next, then MediaProjection |
| `packages/mitm-proxy` | Rust | Plain HTTP forwarding + TLS state/cert generation + CONNECT handler + HTTP/1.1 tunnel loop with phase 3/4/5 scan hooks done; text-policy wired into scan_url/scan_body. Leaf certificates are now browser-acceptable — `src/tls.rs` sets `not_before`/`not_after` (now−1h → now+397d, under Apple's 398-day cap), `serverAuth` EKU, and `digitalSignature` key usage; previously they inherited rcgen's 1975→4096 default with no EKU and Firefox rejected every handshake with `BadCertificate`. **Verified live: Firefox 152 renders an HTTPS page through the proxy.** Note a chain-verification test would *not* have caught that bug (the bogus window contains the present, so webpki accepts) — the guard is the explicit validity-bound assertion in `leaf_cert_validity_is_bounded_and_current`. Open: no CLI args, so port and CA dir are hardcoded (`127.0.0.1:8080`, relative `data/ca`); handshake failures do not log the SNI, which makes coverage gaps hard to attribute. ProtectionMode next |
| `packages/net-shield` | Rust | radix domain/IP filter done; SNI parser done; tun adapter + PacketSink dispatch done; NetShield struct + run loop done (Windows Wintun path); smoke-test done — all 5 plan steps complete |
| `native-modules/win-daemon` | C++20 | WinEvent hooks + message loop; no capture/OCR/IPC yet |
| `native-modules/mac-daemon` | Swift / SwiftPM | **Layer 1 (network path) complete and verified end to end on macOS 26.5.** `CommandRunner` edge + `FakeCommandRunner`, `NetworkServices` parsers, `CATrust` (System-keychain root install/trust/remove), `ProxyConfiguration` (per-service snapshot → apply → restore, `DefaultBypass`), `ProxySupervisor` (pure ordering state machine + `RestartBackoff` + executor over `MitmProxyProcess`/`TCPListenerProbe`), `FirefoxTrust` (`ImportEnterpriseRoots` via the `org.mozilla.firefox` preference domain — **kept only for the tamper-model case, not needed for coverage, see below**), and the `holy-blocker-macd` CLI verbs including `run`. 82 tests. Live: CA install idempotent and independently confirmed via `security verify-cert`; a `URLSession` fetch routed through the supervised proxy (`forwarding method=GET host=example.com port=80`, 200) and `openssl s_client -proxy` was served `issuer=CN=Holy Blocker Local CA`; SIGTERM restores all four services byte-identically. Two traps recorded in the plan: **macOS `curl` is SecureTransport-built, ignores `--cacert`, and skips system proxy settings — never verify coverage with it**, and settings must be applied exactly once per run or a restart re-snapshots our own proxy as the "prior" state. **Firefox needed no policy at all**: `security.enterprise_roots.enabled` defaults to `true`, so `CATrust` covers it — and Firefox *deliberately* ignores `ImportEnterpriseRoots` when it is the only policy present (short-circuits to INACTIVE before applying), so any future use of `FirefoxTrust` must ship a companion policy. Read the shipped logic from `omni.ja` (a plain zip), not mozilla-central, which is versions ahead. Browser coverage is now proven: with the CA installed, **Firefox 152 renders an HTTPS page through the proxy** (this required fixing the leaf-certificate validity bug in `packages/mitm-proxy` — see that row). Firefox's own pinned Mozilla endpoints still fail closed, as expected for pinned clients. Also open: `mitm-proxy` has no CLI args so the port is fixed at 8080; `ProxyConfiguration.restore()` is non-atomic per service and an interruption can leave a proxy enabled with an empty server (the persisted snapshot self-heals it on the next `proxy-restore`). Layer 2 (ScreenCaptureKit capture, NSWindow overlay, AXObserver/CGEventTap) not started but unblocked on the routing side. **Run tests with `scripts/test.sh`, never bare `swift test`** — see `docs/components/mac-daemon/plan.md` |

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
- Windows daemon changes: build with CMake and run any added unit tests
- macOS daemon changes: `./scripts/test.sh` from `native-modules/mac-daemon`. Do **not** use bare
  `swift test` — under a Command Line Tools toolchain it cannot locate swift-testing, and moving
  the needed flags into `Package.swift` as `unsafeFlags` makes SwiftPM silently run zero tests
  while still exiting 0.

If a relevant check cannot be run, report the reason clearly.
