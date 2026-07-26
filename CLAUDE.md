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
| `apps/mobile` | Kotlin / Android | AccessibilityService text path MVP — policy core + ScanGate + overlay + onboarding done; text-policy wired in over UniFFI; SettingsGuard blocks the accessibility/App Info screens that would remove the guard, but **only while the user has armed the in-app protection mode** — `ProtectionSchedule`/`ProtectionStore` hold that state, and disarming is request → 15-min cooldown → confirm → 10-min window that re-arms itself; device admin active, so uninstall is refused until it is deactivated; all verified on an android-36 arm64 emulator. Plain Device Admin only — never design around Device Owner. The empty-harvest gap is closed: the device-admin list rows are `accessibilityDataSensitive` and needed `android:isAccessibilityTool="true"`, without which the guard is handed chrome and no rows — see `docs/components/mobile/backlog.md`. Split screen is handled: every watched window is evaluated, and `GLOBAL_ACTION_BACK` — which takes no window argument — is gated on the matched window holding input focus, covering instead when it does not; verified in real split screen on the emulator, and note an unfocused pane emits no accessibility events at all. The tamper log is done — `policy/TamperLog.kt` (pure) + `TamperLogStore.kt` (the `filesDir` edge) record guard-state transitions and removal attempts, and classify a session that ended without an unbind as an unclean stop; verified on an android-36 emulator against a force-stop, a clean off/on, and a fresh install. It records the guard, never the screen. Recents is **deferred, blocked on real One UI/HyperOS hardware** — the swipe-kill claim cannot be reproduced on an emulator, so no mitigation can be verified. The foreground service is done — `policy/GuardStatus.kt` (pure) + `GuardStatusService.kt` + `BootReceiver.kt`: an ongoing `specialUse` FGS that is the status surface **and** the still-alive process that records a disable the guard cannot report (`adb`, guest, an OEM disable screen we do not recognise), closing the other half of the backlog's undetectable-bypass item. It does **not** keep the guard alive — the system rebinds an enabled accessibility service by itself. `TamperLog.classifyConnect` now takes a tail of entries and anchors on the last session boundary, because the status service writes between sessions and a single-line read turned every clean off/on into an apparent kill. `BootReceiver` writes nothing: `ACTION_BOOT_COMPLETED` is delivered when the app leaves the force-stopped state, so a boot receiver must never be what identifies a boot. Verified on an android-36 emulator against an adb disable, a reboot, a force-stop, and an idle install. The VPN path is done and **verified on an android-36 arm64 emulator** via `scripts/smoke-test-vpn.sh` — `policy/NetworkGuard.kt` (pure) + `NetworkGuardService.kt` + `policy/Blocklist.kt`/`BlocklistStore.kt` (rules from `filesDir/blocklist.txt`) over `packages/net-shield-ffi`: a `VpnService` that answers blocked names with NXDOMAIN locally and forwards everything else to the underlying network's resolvers over a `protect()`ed socket. **It filters DNS only, and the TUN claims exactly one /32 route so nothing else can reach it** — this is forced by the platform, not a staging choice: an Android TUN cannot re-inject a packet the way Wintun can, so permitting a TCP flow needs a userspace TCP stack, and widening the route before that exists black-holes the device rather than filtering it. SNI/IP filtering is therefore a later step. Two traps recorded in the plan: once the VPN is up **our own TUN address is the system's advertised resolver**, so any "read the current DNS servers" path loops packets back into the TUN (guarded twice — `NET_CAPABILITY_NOT_VPN` on the `NetworkRequest`, and `NetworkGuard.upstreamResolvers` dropping our addresses); and NXDOMAIN is used rather than a sink address because one refusal covers A, AAAA and HTTPS/SVCB at once. There is deliberately **no fallback to a public resolver**. The VPN settings screen is **not** in `SettingsProfiles` — §7's rule is that every identifier is dumped from a device, never inferred — so revocation is recorded (`NETWORK_GUARD_REVOKED`) rather than blocked for now. Three defects the unit tests could not catch, all found on the first emulator run and all recorded in the plan: `ACCESS_NETWORK_STATE` was missing, so `registerNetworkCallback` threw *after* the TUN was established; `INTERNET` was missing, and **that failure looks exactly like the filter working** — blocked names refused, permitted ones silently unanswered — so always assert both directions; and **Android does not re-establish a `VpnService` after a reboot** (`setAlwaysOnVpnPackage` is owner-only), so `BootReceiver` restores it. Two smoke-test traps: netd answers from cache without asking the resolver and `ndc resolver flushnetdns` fails silently for a non-root shell, so the script reboots to get a cold cache; and assert on `ping`'s *positive* output, never an error string, or a broken guard reads as a pass. MediaProjection next |
| `packages/mitm-proxy` | Rust | Plain HTTP forwarding + TLS state/cert generation + CONNECT handler + HTTP/1.1 tunnel loop with phase 3/4/5 scan hooks done; text-policy wired into scan_url/scan_body; ProtectionMode next |
| `packages/net-shield` | Rust | radix domain/IP filter done; SNI parser done; tun adapter + PacketSink dispatch done; NetShield struct + run loop done (Windows Wintun path); smoke-test done — all 5 plan steps complete. **DNS path added for the Android VPN** — `dns.rs` (RFC 1035 question parse + NXDOMAIN synthesis; compression pointers in a QNAME are rejected, never followed), `udp.rs` (IPv4/UDP framing with RFC 1071 checksums; fragments refused), `dns_shield.rs` (`DnsShield::inspect` — one pure `Ignore`/`Blocked`/`Forward` decision per packet). 70 tests. Note the deliberate asymmetry with `NetShield`: `PacketSink::pass_packet` assumes Wintun-style re-injection, which **Android does not offer**, so the DNS path returns bytes for the caller to write instead of dispatching through a sink |
| `packages/net-shield-ffi` | Rust | UniFFI wrapper over net-shield's DNS path — `DnsGuard` + `inspect`/`wrapResponse`, `DnsDecision` as an enum-with-fields so the Kotlin binding is a sealed class. Built by the same `apps/mobile/scripts/build-ffi.sh`, which now loops over both FFI crates. Ships a **placeholder** rule set (one RFC 2606 reserved name), exactly as text-policy-ffi ships a placeholder dictionary; real lists go through `with_blocked_domains` at runtime |
| `native-modules/win-daemon` | C++20 | WinEvent hooks + message loop; no capture/OCR/IPC yet |
| `native-modules/mac-daemon` | Swift / SwiftPM | Layer 1 (network path) in progress. `CommandRunner` edge + `FakeCommandRunner`, `NetworkServices` parsers, `CATrust` (System-keychain root install/trust/remove), `ProxyConfiguration` (per-service snapshot → apply → restore, `DefaultBypass`), and the `holy-blocker-macd` CLI verbs — unit-tested **and verified live on macOS 26.5**: CA install is idempotent and independently confirmed trusted via `security verify-cert`, uninstall leaves no residue, and apply → restore returns all network services byte-identically. Note `networksetup` proxy changes need **no root** for an admin user, which makes the standard-user split a prerequisite rather than hardening. `ProxySupervisor` and Firefox NSS trust next; Layer 2 (ScreenCaptureKit capture, NSWindow overlay, AXObserver/CGEventTap) not started. **Run tests with `scripts/test.sh`, never bare `swift test`** — see `docs/components/mac-daemon/plan.md` |

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
