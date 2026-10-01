# macOS Daemon — Implementation Plan

The per-platform capability analysis and the tamper-resistance model live in
[content-interception.md](../../decisions/content-interception.md) (§ "macOS — what carries over
and what changes"). The daemon responsibilities and scan cadence shared with the Windows daemon
are defined in [edge-daemons.md](../../architecture/edge-daemons.md). This document is the build
plan for `native-modules/mac-daemon/`: what modules to add, in what order, and what each one is
responsible for.

## Related flows

- [../flows/block.md](../../product/flows/block.md) — what happens when a scan returns Block
- [../flows/warn-interstitial.md](../../product/flows/warn-interstitial.md) — click-to-reveal cover on Warn verdict
- [../flows/protection-mode-change.md](../../product/flows/protection-mode-change.md) — how ProtectionMode propagates at runtime
- [../flows/partner-setup.md](../../product/flows/partner-setup.md) — the accountability-partner setup this platform's tamper model depends on

## Current state

`native-modules/mac-daemon/` is scaffolded as a SwiftPM package and Layer 1 is partially built:

- `Package.swift` — library target `MacDaemon`, executable `holy-blocker-macd`, test target
  `MacDaemonTests`. **Done.**
- `PrivilegedCommand.swift` — `CommandRunner` protocol, `SystemCommandRunner`, `SystemTool` paths.
  **Done.**
- `FakeCommandRunner.swift` — recording test double. **Done.**
- `NetworkServices.swift` — pure parsers for service lists, proxy settings, bypass domains.
  **Done.**
- `CATrust.swift` — System-keychain trust state, install, uninstall. **Done**, verified live on
  macOS 26.5: install → `installedAndTrusted`, confirmed independently by
  `security verify-cert` ("certificate verification successful"); second install raises no
  further admin prompt; uninstall leaves neither trust setting nor certificate.
- `ProxyConfiguration.swift` — snapshot / apply / restore, plus `DefaultBypass`. **Done**,
  verified live: a full apply → restore cycle across all four real network services returns the
  machine byte-identically to its prior state.
- `ProxySupervisor.swift` — `ProxySupervisorMachine` (the pure ordering state machine),
  `RestartBackoff`, and the executor that drives it through injected edges. **Done**, verified
  live: launch → health check → apply → SIGTERM → restore returns all four services
  byte-identically, and the child is reaped.
- `ProxyProcess.swift` — `MitmProxyProcess`, `TCPListenerProbe`, and the `SystemProxySettings`
  adapter over `ProxyConfiguration`. **Done.**
- `FirefoxTrust.swift` — `ImportEnterpriseRoots` policy in the `org.mozilla.firefox` preference
  domain, with a `CFPreferences`-backed store. **Written and unit-tested, but not working**: the
  policy lands correctly and `sudo firefox-trust` reports `enabled`, yet Firefox 152 still says the
  Enterprise Policies service is inactive. Firefox is therefore still uncovered — see the
  "unresolved" note in Layer 1 module 6.
- `holy-blocker-macd` — CLI verbs (`services`, `ca-status`/`ca-install`/`ca-uninstall`,
  `proxy-status`/`proxy-apply`/`proxy-restore`, `firefox-status`/`firefox-trust`/`firefox-untrust`,
  and `run`) so each module can be exercised against the real system. **Done.**

82 tests pass via `scripts/test.sh`.

**Layer 2 is now fully specified** (modules 0 and 7–14 below), superseding the sketch that stood
there. No Layer 2 code is written yet. Specifying it surfaced three things worth knowing before
implementation starts: the sketch was missing both a scan scheduler and an IPC module, the signed
bundle is a step-zero prerequisite rather than a later concern, and `PermissionGate` has to come
first rather than last. One product decision — what to do when the protected user is a local admin —
is called out in module 7 and needs sign-off before onboarding is written.

**Layer 1 is verified end to end, including a real browser.** With the supervisor running, a
`URLSession` fetch of `http://example.com` reached the proxy — `mitm_proxy::forward: forwarding
method=GET host=example.com port=80` — and returned 200. With the root CA installed and trusted via
`CATrust`, **Firefox 152 renders `https://example.com` through the proxy**, and `openssl s_client
-proxy 127.0.0.1:8080` reports `Verify return code: 0 (ok)` against our CA for a leaf issued
`CN=Holy Blocker Local CA` / `CN=example.com`. Layer 2 may begin.

Getting there required fixing a defect in `packages/mitm-proxy`: generated leaf certificates carried
`rcgen`'s default 1975→4096 validity and no extended key usage, so Firefox rejected the handshake
with `BadCertificate` regardless of root trust. See the mitm-proxy plan for the fix and, more
usefully, for why a chain-verification test would *not* have caught it.

**Firefox pins its own services.** Two background connections were still rejected with
`BadCertificate` after the fix, while `example.com` succeeded. These are almost certainly Firefox's
pinned Mozilla endpoints (telemetry / remote settings), which is the certificate-pinning limitation
already recorded under "What Layer 1 does not cover" — pinned clients fail closed rather than being
inspected. It could not be confirmed from the logs because **the proxy does not record the SNI when
a browser-side handshake fails**, which is a small observability gap worth closing before the next
coverage investigation.

### Two bugs the unit tests could not have caught

Both were found only by running against the real tools, and both are worth remembering before
writing fixtures for any further `security(1)` or `networksetup(8)` parsing:

1. **An invented fixture encoded a format that does not exist.** `state()` required a
   `Result = trustRoot` line in `dump-trust-settings` output. Real output has no such line: a
   trusted root is rendered with `Number of trust settings : 0`, an empty settings array meaning
   "trusted for all purposes" — all 157 built-in system roots appear exactly that way. The parser
   therefore reported `installedUntrusted` immediately after a successful install, making
   `install()` re-run `add-trusted-cert` on every daemon start and re-prompt for admin credentials
   each time. The unit test asserting idempotence passed against the fictional format.
2. **`restore()` left the proxy host behind.** `-setwebproxystate off` disables a proxy but keeps
   the server and port, so every service was left holding `127.0.0.1:8080`. Re-enabling the proxy
   for any unrelated reason would then route traffic at a dead local port. `-setwebproxy <service>
   "" 0` does blank both fields — undocumented but accepted — and must be issued *before* the
   state-off call, because it also turns the proxy on.

Capture fixtures from real tool output. Where that is impractical (the `deny` trust result is the
one remaining case), say so at the fixture.

### Do not verify proxy coverage with `curl`

macOS ships curl built against **SecureTransport** (`curl -V` reports it), and that build **ignores
`--cacert`** and does not consult the macOS system proxy settings. So `curl --cacert ca.crt
https://example.com` returns 200 whether or not the traffic was intercepted, and returns 200 again
with the flag removed. It looks like a passing end-to-end test and proves nothing in either
direction.

Use a CFNetwork client instead — a `URLSession` fetch honours the per-service proxy settings this
package writes, which is exactly the coverage question being asked. For the TLS half, `openssl
s_client -proxy 127.0.0.1:8080 -connect host:443 -servername host` prints the issuer actually
served, which is unambiguous. Both are recorded in the current-state section above.

What already exists and is reused unchanged:

- `packages/mitm-proxy/` — Rust. Plain HTTP forwarding, TLS state and per-SNI cert generation,
  CONNECT handler, and the HTTP/1.1 tunnel loop with scan hooks. Portable; runs on macOS today.
- `packages/text-policy/` — Rust. The classification engine the proxy already calls.

Unlike iOS, macOS is a **full-tier platform**: both interception layers run and our own
classification engines participate. The only capability lost relative to Windows is the deferred
process-injection optimization, which is not in the core model.

## Scope split — what belongs in this package

This package is the **macOS platform adapter**. Policy and classification stay in the Rust
packages; the daemon owns Apple APIs, permission handling, and process lifecycle.

| Concern | Where it lives |
|---|---|
| Verdicts, scoring, lexicon | `packages/text-policy` (Rust) — unchanged |
| TLS termination, URL/body scanning | `packages/mitm-proxy` (Rust) — unchanged |
| CA trust, system proxy config, process supervision | **this package**, Layer 1 |
| Screen capture, overlay, AX/event hooks | **this package**, Layer 2 |

Language is **Swift**, built with SwiftPM. Keep Apple API glue thin and put decision logic in
pure, testable types — the same rule the Windows daemon follows for Win32 glue.

## Build environment constraint

Layer 1 was built on **Swift 6.3 with Command Line Tools only — no full Xcode**, which was
sufficient for all of it (a plain SwiftPM executable, no app bundle, no entitlements).

**That constraint has since lifted.** As of macOS 26.5.2, `xcode-select -p` on the development
machine reports `/Applications/Xcode.app` (Swift 6.3.3), and `ScreenCaptureKit`,
`ApplicationServices`, `IOKit.hid` and `AppKit` were all confirmed to import, link and run against
this toolchain. The `scripts/test.sh` workaround below is now a no-op here, but keep it — it tests
the active toolchain, not the presence of Xcode, and Layer 1 must stay buildable under Command Line
Tools.

Layer 2 still needs a signed `.app` bundle for stable TCC permission grants, since TCC identifies
clients by code signature and unsigned command-line binaries get inconsistent, easily-invalidated
grants. **That bundle is the *first* Layer 2 step, not a later one** — see Layer 2 module 0. An
earlier draft of this paragraph deferred it to "step 5", which was wrong: every permission grant
made before the bundle exists is invalidated by the next rebuild, so building capture first means
granting Screen Recording to an identity that is about to change.

### Running the tests — use `scripts/test.sh`, not `swift test`

Two toolchain quirks, both already worked around, both worth knowing before touching the manifest:

1. **`swift test` alone cannot find swift-testing under Command Line Tools.** `Testing.framework`
   ships at `/Library/Developer/CommandLineTools/Library/Developer/Frameworks`, which SwiftPM does
   not search, so the test target fails to compile `import Testing`; adding only `-F` gets it
   compiling but it then fails to `dlopen` both `Testing.framework` and `lib_TestingInterop.dylib`.
   `scripts/test.sh` supplies the search path and both rpaths, and is a no-op when the active
   toolchain is a full Xcode.
2. **Those flags must not move into `Package.swift`.** A test target carrying `unsafeFlags` makes
   SwiftPM **silently stop discovering tests** — `swift test` builds, prints nothing, exits 0, and
   `swift test list` reports nothing. There is no warning. A green-looking run that executed zero
   tests is the worst possible failure mode for a test-first package, so the flags stay in the
   script.

Note also that `xcode-select -p` may point at the Command Line Tools even when `Xcode.app` is
installed; the script tests the active toolchain, not the presence of Xcode.

3. **The script also builds the UniFFI bindings**, which are generated and gitignored, so a bare
   `swift test` on a fresh checkout fails at package-layout resolution before it compiles anything.
   Set `HOLY_BLOCKER_SKIP_FFI=1` to skip that step when iterating on Swift alone. See module 16.

---

# Layer 1 — network path

The proxy itself is done. What is missing is everything that makes macOS traffic actually arrive at
it, and everything that makes the daemon a well-behaved system citizen.

## Modules to add

### 1. `PrivilegedCommand` — the process-execution edge <!-- step: mac-daemon.privileged-command -->

Specification: [modules/01-privileged-command.md](modules/01-privileged-command.md).

### 2. `CATrust` — root CA installation in the System keychain <!-- step: mac-daemon.catrust -->

Specification: [modules/02-catrust.md](modules/02-catrust.md).

### 3. `NetworkServices` — enumerating and parsing network services <!-- step: mac-daemon.network-services -->

Specification: [modules/03-network-services.md](modules/03-network-services.md).

### 4. `ProxyConfiguration` — pointing macOS at the proxy, and putting it back <!-- step: mac-daemon.proxy-configuration -->

Specification: [modules/04-proxy-configuration.md](modules/04-proxy-configuration.md).

### 5. `ProxySupervisor` — running and monitoring the Rust proxy <!-- step: mac-daemon.proxy-supervisor -->

Specification: [modules/05-proxy-supervisor.md](modules/05-proxy-supervisor.md).

### 6. Firefox NSS trust — `FirefoxTrust.swift` <!-- step: mac-daemon.firefox-trust -->

Specification: [modules/06-firefox-trust.md](modules/06-firefox-trust.md).

## What Layer 1 does *not* cover on macOS — honest limits

Specification: [layer-1-limits.md](layer-1-limits.md).

## Layer 1 implementation order

1. ~~Scaffold the SwiftPM package: `Package.swift` with one executable target and one test target.
   Confirm `swift build` and `swift test` both run under Command Line Tools with no Xcode.~~
   **Done.** Test invocation is `scripts/test.sh` — see the build-environment section above. <!-- step: mac-daemon.package-scaffold -->
2. ~~`PrivilegedCommand.swift` — the `CommandRunner` protocol, the real runner, and the fake.
   Nothing else can be tested until this exists.~~ **Done.**
3. ~~`NetworkServices.swift` — pure parsers. Tests first; these need no fake runner at all.~~
   **Done.** Fixtures are verbatim `networksetup` output captured on macOS 26.5.
4. ~~`CATrust.swift` — argv construction and state parsing under test, then the real install path,
   verified once by hand against the System keychain.~~ **Done.** All fixtures confirmed against
   the real `security(1)`; install/idempotence/uninstall verified live, with `verify-cert` as an
   independent check that the CA was actually trusted.
5. ~~`ProxyConfiguration.swift` — snapshot/apply/restore under test, then a manual end to end
   check that `restore()` genuinely returns the machine to its prior state.~~ **Done** —
   apply → restore verified byte-identical across all four real network services.~~ Traffic
   through the proxy is now confirmed too — see the end-to-end note in the current-state section.
6. ~~`ProxySupervisor.swift` — the ordering state machine under test, then real process
   spawning.~~ **Done.** The state machine, `RestartBackoff`, and the executor are unit-tested;
   the `run` verb was exercised live against a real `mitm-proxy` child, including SIGTERM →
   restore → reap.
7. ~~Firefox NSS trust (module 6 above).~~ **Retired as a coverage step.** Firefox trusts
   System-keychain roots by default (`security.enterprise_roots.enabled` defaults to `true`), so
   `CATrust` alone covers it and no policy is required. `FirefoxTrust.swift` is kept for the
   tamper-model case only; read module 6 before touching it, because Firefox silently ignores
   `ImportEnterpriseRoots` when it is the only policy present.

---

# Layer 2 — render path

Conceptually identical to the Windows daemon — capture → text + image models → mode-driven response
— but every mechanism is an Apple API behind a TCC permission, and that changes the shape of the
work: on Windows the hard part is the capture, here the hard part is *holding* a permission the OS
deliberately keeps under user control.

**Layer 1 has landed**, so the sketch that stood here is now replaced by a full specification.

## What changed between the sketch and this specification

Four things, recorded because they alter the plan rather than merely detail it:

1. **Full Xcode is now installed** — `xcode-select -p` reports `/Applications/Xcode.app` on the
   development machine (macOS 26.5.2, Swift 6.3.3). The build-environment section above was written
   under Command Line Tools only. `ScreenCaptureKit`, `ApplicationServices`, `IOKit.hid` and
   `AppKit` all import and link, verified by compiling and running a probe against this toolchain.
   The bundle-signing prerequisite Layer 2 needs is therefore no longer blocked on a toolchain
   install.
2. **The sketch was missing two modules.** It listed capture, overlay, hooks, AX text, fullscreen
   and permissions — but nothing that *schedules* scans (the Windows daemon's `scan_loop`) and
   nothing that carries verdicts to the Electron app (the Windows daemon's `ipc`). Both are load
   bearing and both are specified below. Without the scheduler there is no debounce, no cadence
   split between the image and text models, and nowhere for `ProtectionMode` to be applied; without
   the IPC module the daemon can detect but cannot respond.
3. **`PermissionGate` moves from last to first.** It was numbered 12 because it reads like
   onboarding polish. It is not: no other Layer 2 module can be *run*, let alone verified, until
   the process holds the permission it needs, and the permission is attached to a code signature
   that must exist before the first grant. Building capture first means granting Screen Recording
   to a binary whose identity is about to change.
4. **Permission loss is a tamper event, not an error path.** This is the lesson the Android work
   already paid for — see the `GuardStatus` foreground-service note in the root `AGENTS.md`: the
   thing that matters is the still-alive process that can record a disable the guard itself cannot
   report. Screen Recording is revocable by anyone who can authenticate as an admin, and Apple
   reserves that right permanently (no MDM can remove it). So the daemon must treat "I no longer
   have capture" as a reportable state transition, not as a reason to log and exit.

## Module 0 — the signed bundle, and the TCC identity trap <!-- step: mac-daemon.signed-bundle -->

Specification: [modules/00-signed-bundle.md](modules/00-signed-bundle.md).

## Modules to add

### 7. `PermissionGate` — TCC state, and the account model that locks it <!-- step: mac-daemon.permission-gate -->

Specification: [modules/07-permission-gate.md](modules/07-permission-gate.md).

### 8. `ScreenCapture` — `ScreenCaptureKit` <!-- step: mac-daemon.screen-capture -->

Specification: [modules/08-screen-capture.md](modules/08-screen-capture.md).

### 9. `Scanner` and `ScanLoop` — the interface and the scheduler <!-- step: mac-daemon.scanner-scan-loop -->

Specification: [modules/09-scanner.md](modules/09-scanner.md).

### 10. `Overlay` — borderless `NSWindow` <!-- step: mac-daemon.overlay -->

Specification: [modules/10-overlay.md](modules/10-overlay.md).

### 11. `EventHooks` — window lifecycle and scroll <!-- step: mac-daemon.event-hooks -->

Specification: [modules/11-event-hooks.md](modules/11-event-hooks.md).

### 12. `AccessibilityText` — AX text extraction. **Done.** <!-- step: mac-daemon.accessibility-text -->

Specification: [modules/12-accessibility-text.md](modules/12-accessibility-text.md).

### 13. `FullscreenControl` — AX `AXFullScreen` <!-- step: mac-daemon.fullscreen-control -->

Specification: [modules/13-fullscreen-control.md](modules/13-fullscreen-control.md).

### 14. `DaemonIPC` — carrying verdicts to the desktop app <!-- step: mac-daemon.daemon-ipc -->

Specification: [modules/14-daemon-ipc.md](modules/14-daemon-ipc.md).

### 15. `SettingsGuard` — notice the settings pane, and record it <!-- step: mac-daemon.settings-guard -->

Specification: [modules/15-settings-guard.md](modules/15-settings-guard.md).

### 16. `TextPolicyFFI` — the Rust policy engine over UniFFI. **Done.** <!-- step: mac-daemon.text-policy-ffi -->

Specification: [modules/16-text-policy-ffi.md](modules/16-text-policy-ffi.md).

### 18. `ImageScanner` — the ONNX classifier over UniFFI. **Done.**

Specification: [modules/18-image-scanner.md](modules/18-image-scanner.md).

## What Layer 2 does *not* cover on macOS — honest limits

Specification: [layer-2-limits.md](layer-2-limits.md).

## Layer 2 implementation order

0. ~~**Module 0 — the signed bundle and the `launchd` split.** Nothing below can be honestly
   verified before this exists, because every grant made to an unsigned binary is invalidated by
   the next build, and every grant made from a terminal belongs to the terminal.~~ **Done**, with
   one decision outstanding: the bundle signs ad-hoc until a stable certificate exists, and both
   `bundle` and `agent` say so at runtime. See the module 0 section above for the five findings,
   the first of which contradicts the instruction the section was written with.
1. **`PermissionGate`** (module 7) — ~~pure snapshot logic and transitions under test, then the
   three preflight calls~~ **Done**, ahead of module 0 because none of it needs a grant — then a
   real grant against the signed bundle. Verify revocation is *detected*, not just that the grant
   works. **That last part is still outstanding and is blocked on module 0**: revocation can only
   be observed against a stable signing identity, and today `permissions` run from a shell reports
   the *terminal's* grants. The pure half is 38 tests (`PermissionGateTests.swift`), and all four
   zero-permission environment signals were confirmed live through the `permissions` verb.
2. **`ScreenCapture`** (module 8) — ~~get one `.complete` frame out of `SCStream` and write it to a
   PNG by hand before building anything on top of it. Confirm the row-padding copy against a known
   test pattern rather than by eye; a sheared frame looks fine at a glance.~~ **Done**, except the
   live PNG-by-hand check, which is blocked on the same real grant as `PermissionGate` — see the
   module 8 section above.
3. **`Scanner` + `ScanLoop`** (module 9) with `NullScanner` — ~~the debounce, cadence and mode logic
   are the most testable code in Layer 2 and need no permissions at all. This step can proceed in
   parallel with steps 1–2 by anyone blocked on grants.~~ **Done.** `ScanVerdict` carries the
   `regions: [ImageDetection]` field per
   [content-classification.md](../../architecture/content-classification.md#image-localization), but
   nothing populates it yet — `NullScanner` and every test double return an empty list, per that
   section's deferral of real image localization. 62 new tests (`ScannerTests.swift`,
   `ScanLoopTests.swift`). **The first real `Scanner` now exists too** —
   `AccessibilityScanner.swift`, built in session 4 of the e2e split below, which settles module
   9's mode→action "open" note: the daemon consumes `text-policy-ffi` over UniFFI rather than
   adding a third implementation. Note the split that leaves behind: `ScanLoop`'s own
   `applyProtectionMode` is still plain Swift and stays that way, because a `ProtectionMode`
   downgrade is a *product* setting this daemon owns, not the scoring rule the Rust crate owns.
   25 more tests (`AccessibilityScannerTests.swift`).
4. ~~**`Overlay`** (module 10)~~ — **Done**, both halves. The pure part decides one placement per
   screen from a verdict-shaped intent, with `.screenSaver`-equivalent level and
   `.canJoinAllSpaces`/`.fullScreenAuxiliary`-equivalent collection behavior recorded as data.
   Region-level cover is deliberately not implemented — every placement still covers the full screen
   frame, per the `Overlay` section's note above. The AppKit half is `OverlayWindow.swift`:
   `AppLifecycle.configureAccessoryApp()` (`NSApplication.shared` +
   `setActivationPolicy(.accessory)`, without running the loop, so the caller keeps the thread),
   `ScreenConfiguration.current()`/`init?(screen:)`, the two mappings onto `NSWindow.Level` and
   `NSWindow.CollectionBehavior`, a `@MainActor OverlayController` owning `[screenID: NSWindow]` and
   observing `didChangeScreenParametersNotification`, and an `OverlayContentView` backdrop. An
   `overlay [seconds] [passive]` verb drives it from the CLI; the process runs, covers the one
   attached display, and terminates cleanly. **Still outstanding — the *visual* is unverified**:
   confirming the overlay is actually drawn, sits above native-fullscreen video, and reconciles
   across a display connect/disconnect needs a human looking at a screen (and, for a screenshot, a
   Screen Recording grant a terminal-run binary does not have). Three implementation notes:
   - **The reconcile is a third pure function, not AppKit glue.** `OverlayReconciliation.diff`
     (`Overlay.swift`) splits the live window set against a fresh plan into create/update/close.
     Reusing an existing window instead of rebuilding it is not an optimization — the scan loop
     reconciles several times a second, and recreating an unchanged window would flicker the
     overlay continuously. Pulling this out is what keeps `OverlayWindow.swift` branch-free and so
     legitimately exempt from tests. 11 new tests.
   - **The backdrop reads no pixels at all.** `NSVisualEffectView` blurs the composited desktop
     *behind* the window, so the native interstitial matches the proxy's injected
     `backdrop-filter: blur(24px)` over `rgba(0,0,0,0.55)` without a captured frame existing to be
     mishandled — stronger than the module's "the frame is a texture" rule.
   - **A `Timer` scheduled after `NSApplication.run()` never fires**, and neither does one created
     off the main run loop — the same trap as an `AXObserver` with no run-loop source. The `overlay`
     verb schedules its teardown timer before surrendering the thread.
   The verse and the two-button deliberate choice from
   [warn-interstitial.md](../../product/flows/warn-interstitial.md) are **not** built: they need the
   verse table and a dismissal path back to the scan loop, neither of which exists on this platform.
   The content view draws a placeholder label where that card goes. 27 tests total
   (`OverlayTests.swift`).
5. **`DaemonIPC`** (module 14) — the pure part is **done**: length-prefixed JSON framing, an
   incremental decoder (`.needsMoreData` / `.message` / `.oversized` / `.invalid`, never throwing on
   a partial read), and validated `scan_event`/`config_update` Codable types, with `regions`
   (`ImageDetection`, always an array, never omitted) on `scan_event` per the module 14 section above.
   34 new tests (`DaemonIPCTests.swift`). Still outstanding: the actual Unix domain socket
   transport, and wiring into `apps/desktop/src/main/daemon-ipc.ts`, whose current shape
   (`verdict`/`windowTitle`/`at`, camelCase) does not yet match this wire protocol. Also flagged but
   not resolved: `ts` is encoded here as a Double (Unix epoch seconds), while the Windows daemon plan
   documents an ISO-8601 string — a cross-platform choice to make once, deliberately, not by default.
6. **`EventHooks`** (module 11) — measure which permission the scroll monitor actually requires
   before writing onboarding copy.
7. ~~**`AccessibilityText`** (module 12) — validate against a Chromium-based app early, since that is
   where the `AXManualAccessibility` trap bites and where the coverage question is decided.~~
   **Done**, and validating early was worth it: the trap is real but its documented fix is not, and
   Chrome 151 refuses the attribute the plan was written around. See module 12's section above for
   the measurements and for the one outstanding check (Safari's page body). 22 new tests
   (`AccessibilityTextTests.swift`).
8. **`FullscreenControl`** (module 13) — last, and only if module 10 proves insufficient in practice.
9. **`SettingsGuard`** (module 15) — can be built at any point after step 1, since it needs no
   permissions; sequenced last because it is defence in depth over `PermissionGate`, not a
   substitute for it.
10. ~~**`ImageScanner`** (module 18) — the ONNX classifier over UniFFI, and the first `Scanner` that
    looks at a pixel.~~ **Done.** See module 18's section above for the six carried findings, the
    load-bearing one being that inference cannot run on the main run loop. It also forces the
    `ScanLoop` two-scanner split, since one `Scanner` served both cadences before it. 22 new tests
    (`ImageScannerTests.swift`), plus 2 in `AppBundleTests.swift` for the sealed model resource.
    **Outstanding: the classifier has never seen a real captured frame** — that needs the agent
    under `launchd` with a Screen Recording grant, which is a human at a screen rather than code.

### The first live e2e pass — text-only, split into six sessions

Everything above through step 5 is pure logic with no permissions and nothing visible on screen.
The next milestone is the first thing a human can actually watch happen: a real overlay appearing
over real on-screen text, scored by the real Rust policy engine, under a real (non-terminal) TCC
grant. This is too much for one session — it fans out into three independent pieces, a merge
point, and an integration pass, mirroring how modules 9/10/14 were each their own branch and
`merge:` commit. Scope for this pass is text only: no image classifier, no `DaemonIPC` socket
transport or `apps/desktop` wiring, no `EventHooks`/`FullscreenControl`/`SettingsGuard`, no
region-level overlay cover.

The mode→action "open" question in module 9 above is resolved for this pass: consume
`packages/text-policy-ffi`'s `PolicyEngine`/`evaluate` over UniFFI from Swift rather than add a
third independent reimplementation of policy logic. `packages/text-policy-ffi/src/lib.rs` already
exports exactly what's needed — a `uniffi::Object` `PolicyEngine`
(`with_builtin_dictionary()`/`with_thresholds(block:warn:)`) with
`evaluate(text: String, source: SourceKind) -> Verdict { action: Action, score: u32 }`, and
`SourceKind::AccessibilityTree` already exists — no Rust changes needed.

Sessions, in dependency order (1–3 can run in parallel worktrees; 4 needs 1+2 merged; 5 needs 3+4
merged; 6 is human-only, no code):

1. ~~**`infra/text-policy-ffi-swift-bindings`** — a new **module 16**. `scripts/build-ffi.sh`
   (modeled on `apps/mobile/scripts/build-ffi.sh`, one crate, `--language swift` instead of
   kotlin, run from the crate dir since `uniffi-bindgen` resolves `cargo metadata` off cwd, not
   `--manifest-path`) builds `libtext_policy_ffi.dylib` for `aarch64-apple-darwin` and generates
   Swift bindings into `Sources/TextPolicyFFI/generated/` — gitignored, never committed, same
   treatment as `apps/mobile`'s `app/src/generated/kotlin`. Two new SwiftPM targets
   (`CTextPolicyFFI` wrapping the generated header, `TextPolicyFFI` wrapping the generated
   `.swift` file) that `MacDaemon` depends on. `scripts/test.sh` must call `build-ffi.sh` itself
   and export `DYLD_LIBRARY_PATH` for the loose dylib. **The real gap this session closes**:
   `AppBundle.assemble` (`AppBundle.swift:79`) today only copies the single executable into
   `Contents/MacOS/` — it has no concept of embedding a dylib. Add a `Frameworks/` directory to
   `BundleLayout`, copy `libtext_policy_ffi.dylib` there, and **sign the dylib before the bundle**
   in `CodeSigning.sign(bundle:identity:)` (an unsigned/differently-signed loose dylib under a
   signed bundle can be refused at `dlopen` time) — test-first via the existing
   `AppBundleTests.swift`/`FakeCommandRunner` fake-filesystem pattern.~~ **Done** — see module 16
   above for the five findings, two of which contradict this bullet: the C target could not be
   named `CTextPolicyFFI`, and `DYLD_LIBRARY_PATH` does not survive `swift test` under SIP.
2. ~~**`feat/mac-daemon-accessibility-text`** — module 12, `AccessibilityText.swift`. Pure,
   test-first: `AXElementProbing` protocol + `FakeAXElementProbe` (same shape as
   `CommandRunner`/`FakeCommandRunner`), `AccessibilityText.extractText(from:probe:limits:)`
   bounded by depth/node-count (`AXWalkLimits`, defaults 40/2000), adjacent-only dedup, and a
   documented separator choice. Tests cover an empty tree, a cyclic fake graph (real AX trees can
   legitimately cycle), the depth and node-count bounds, and whitespace filtering. The real edge
   (`RealAXElementProbe`) is thin AppKit/AX glue, exempted from test-first same as
   `SCShareableContentCapture`: `AXUIElementSetMessagingTimeout` (~300ms) plus a coarser ~500ms
   wall-clock budget on the whole walk, off the scan-timer thread; `kAXManualAccessibility` set to
   `true` on the frontmost app's application element before reading its focused window (required
   for Chrome/Electron, which expose nothing without it); any AX error mid-walk stops that subtree
   rather than throwing. Verify live against Safari and a real Chromium app, with and without the
   manual-accessibility set.~~ **Done**, and verifying live is what earned this session's findings:
   **the `kAXManualAccessibility` instruction in this bullet is wrong** — Chrome 151 refuses the
   attribute outright, and what builds its tree is being an AX client at all. See module 12 above
   for the measurements. The edge shipped as `SystemAXProbe` rather than `RealAXElementProbe`, and
   the walk takes an `AXNodeID` handle rather than a raw element so it stays free of AX types.
   22 tests.
3. ~~**`feat/mac-daemon-overlay-appkit`** — module 10's second half, new file
   `OverlayWindow.swift` (existing `Overlay.swift` stays AppKit-free per its own header comment).
   `OverlayController` owns the live `[screenID: NSWindow]` set, observes
   `NSApplication.didChangeScreenParametersNotification`, and reconciles from
   `OverlayPlan.plan(intent:screens:)`. Mechanical mappings onto real AppKit:
   `OverlayWindowLevel.screenSaver → NSWindow.Level.screenSaver`, both
   `.canJoinAllSpaces`/`.fullScreenAuxiliary` set together (never just one — missing either
   silently fails over native-fullscreen video, per the module 10 section above),
   `ignoresMouseEvents` direct. `AppLifecycle.configureAccessoryApp()` calls
   `NSApplication.shared`/`setActivationPolicy(.accessory)`. No new tests — thin glue over the
   already-tested pure `OverlayPlan`, same exemption as the rest of this split; no existing
   `NSScreen` fake exists in this codebase and building one just for this session isn't worth
   it.~~ **Done**, with one correction to this bullet: **it does need new tests**, because the
   reconcile is not glue. Splitting the live window set against a fresh plan into
   create/update/close is ordinary logic, it lives in the AppKit-free file as
   `OverlayReconciliation.diff(existing:planned:)`, and it earns 11 tests — reusing a window
   instead of rebuilding it is load-bearing rather than tidy, since the controller runs on every
   scan tick and recreating unchanged windows would flicker the overlay several times a second.
   Extracting it is also what makes the "no `NSScreen` fake" exemption honest: what remains in
   `OverlayWindow.swift` has no branches to test. See step 4 of the implementation order above for
   the rest of the findings. The overlay's *visual* is unverified and is one of the things session
   6 should look at.
4. ~~**`feat/mac-daemon-real-scanner`** (needs 1+2 merged) — module 9 gets its first real
   `Scanner`, new file `AccessibilityScanner.swift`. **Interface resolution**:
   `Scanner.scan(_ frame: CapturedFrame)` is frame-driven, AX text isn't —
   `AccessibilityScanner` ignores its `frame` parameter and does its own independent AX read,
   which needs zero changes to `Scanner.swift`/`ScanLoop.swift`/their 62 existing tests and slots
   into `ScanLoop`'s OCR-cadence slot (1–2s), the right tier for this kind of scan. Because
   `ScanLoop` calls one injected `Scanner` on *both* its ~500ms image cadence and its ~1–2s OCR
   cadence, `AccessibilityScanner` must internally rate-limit its own AX walk so the faster cadence
   doesn't trigger a full walk every time — test-first. `PolicyEngineHandle` is the UniFFI seam
   (`RealPolicyEngine` wraps the generated `PolicyEngine`), kept fake-able so
   `Scanner.swift`/`ScanLoop.swift` stay UniFFI-agnostic. The Rust `Action` enum has 5 cases
   (Block/Blur/Warn/Log/Allow), `ScanAction` has 3 (allow/warn/block) — map `Block → .block`,
   `Warn → .warn`, `Allow`/`Log → .allow`, **`Blur → .warn`** (no dedicated blur visual state
   exists yet), with its own named test since it's a real information-loss decision. Test-first:
   the 5→3 mapping, the rate-limit gate, the empty-tree/no-root → `.allow` fallback, and the
   `u32` 0–100 → `Double` 0.0–1.0 score normalization, all against fakes.~~ **Done**, and this
   bullet held up — every interface prediction in it was right, `Scanner.swift`/`ScanLoop.swift`
   and their 62 tests are untouched, and the `PolicyEngineHandle` seam keeps them UniFFI-agnostic.
   25 new tests (`AccessibilityScannerTests.swift`), 308 in the suite. Four things worth carrying
   forward, none of which the bullet anticipated:

   - **The rate-limit gate must repeat its last verdict, not allow.** The bullet specified the
     gate but not what a gated tick *returns*, and `.allow` is the wrong answer in a way that only
     shows up on screen: `ScanLoop.lastVerdict` and the overlay above it are driven by what the
     scanner returns, so allowing between walks would tear the interstitial down and rebuild it
     several times a second over content that never stopped being blocked. The cached verdict is
     load-bearing state, not an optimization.
   - **A walk that found nothing still closes the gate.** Stamping the gate only on a successful
     walk would leave a browser with no focused window — the *expected* case, per module 12's
     finding that a first walk against a browser is legitimately thin — hammered on every 500 ms
     image tick, which is precisely the load the gate exists to remove.
   - **The seam carries the generated `Verdict`/`SourceKind` rather than Swift mirrors of them.**
     `text-policy-ffi`'s API is a cross-platform contract shared with Android; a mirrored enum here
     would be a second place to update and a second chance for this daemon to disagree with the
     policy it enforces. `Scanner.swift`/`ScanLoop.swift` still never see a UniFFI type.
   - **The cross-element phrase problem from module 12 is decided, and deferred deliberately.**
     `AccessibilityText`'s header flags that no separator can stop a lexicon phrase matching across
     two unrelated elements, and hands the decision to this session. It is not fixed here:
     `AccessibilityText.extract` returns one joined `String`, so evaluating elements separately
     means widening its API *and* accepting a new false-negative class — a phrase legitimately
     wrapped across two AX elements would stop matching. That is a trade between two error
     directions and deserves a measurement rather than a guess, which nothing in this repo can
     supply yet. Recorded in [backlog.md](backlog.md). <!-- step: mac-daemon.accessibility-scanner -->
5. ~~**`feat/mac-daemon-agent-render-loop`** (needs 3+4 merged) — rewires `runAgent()` in
   `Sources/holy-blocker-macd/main.swift`: replaces the `Thread.sleep(30s)` poll loop with
   `AppLifecycle.configureAccessoryApp()` + a real `NSApplication` run loop. A main-thread
   repeating `Timer` (~0.25s, created before `app.run()` — an off-thread timer never added to the
   main run loop silently never fires, the same trap as `AXObserver`) ticks
   `scanLoop.tick(now:)` and reconciles `overlay.apply(intent:)`; a second timer keeps polling
   `gate.poll()` for tamper events; a third watches `stopRequested` and calls `app.terminate(nil)`
   since `app.run()` now owns the thread. This timer is an explicitly acknowledged stand-in for
   module 11 (`EventHooks`), not a replacement for it. **Known coupling, flagged not fixed**:
   `ScanLoop.tick` (`ScanLoop.swift:179`) gates on `!frame.isEmpty`, so without a Screen Recording
   grant the AX-text scanner never runs at all even though it doesn't touch frame pixels — since
   Screen Recording is being granted anyway for session 6, `ScanLoop` is left untouched rather than
   special-cased for a frame-less scanner; revisit once a real image scanner exists.~~ **Done**,
   with the three timers built as specified and one addition the bullet didn't anticipate. The
   mapping from `ScanVerdict.action` to `OverlayIntent` — the exact seam
   `OverlayIntent.OverlayVerdictAction`'s doc comment named this session as the place to resolve —
   is a new pure function, `overlayIntent(forVerdict:)`, test-first with 5 tests including one that
   pins reading `.action` rather than `.rawAction` (a warn-mode downgrade must reach the overlay as
   a warn, not the raw block underneath it). **The three timers could not capture `scanLoop`/`gate`
   directly**: `Timer.scheduledTimer`'s block is `@Sendable`, and both are plain (correctly
   non-`Sendable`) library classes, so Swift 6 flags every capture as a potential data race even
   though all access is main-thread-only. Splitting the async prep (the two real `await`s) from a
   second, non-`async` function that does the Timer wiring did not fix it either — the same error
   persisted with `gate`/`capture` as plain parameters. What actually resolved it was giving the
   timers one `@MainActor` class, `AgentRenderLoop`, to capture instead of two plain ones — the
    same reason `OverlayController` itself is `@MainActor` and not a plain class. 313 tests. <!-- step: mac-daemon.agent-render-loop -->
6. ~~**Live verification** (needs 5 merged, human-in-the-loop, no code): rebuild bindings and bundle,
   reload the LaunchAgent, grant Accessibility + Screen Recording for real via System Settings
   against `HolyBlockerDaemon.app` specifically (never a shell binary — the responsible-process
   rule), confirm `holy-blocker-macd permissions` reports both granted, bring text the shipped
   starter dictionary scores `Block` on (e.g. "explicit act", already used in
   `text-policy-ffi`'s own test fixtures) frontmost and confirm an interstitial appears and
   swallows a click within ~1–2s, confirm it tears down over clean text, check native-fullscreen
Space interaction, and multi-display connect/disconnect if a second display is available.~~
    **Done for the core claim, and it was not "no code" — see below.** The pipeline ran end to end
    on macOS 26.5: real `SCStream` frame → real AX walk of the frontmost window → `text-policy` over
    UniFFI scoring `Block` at 0.80 → a real `NSWindow` on screen, and it tears down when the text
    goes away. The remaining checks (native-fullscreen Space, multi-display) are still outstanding,
    as is the biggest thing this pass found — see [backlog.md](backlog.md). <!-- step: mac-daemon.live-e2e -->

#### What the first live pass actually found

The session was specified as human-in-the-loop with no code. That was wrong in both directions: two
of the three blockers were code defects, and neither was reachable by any test in this repo.

1. **The capture stream delivered YUV, not BGRA, and every frame was dropped.**
   `SCStreamConfiguration.pixelFormat` was never set, and the default on macOS 26.5 is biplanar
   `420v` — 2 planes, and `CVPixelBufferGetBytesPerRow` returns the *Y plane's* stride (1536 for a
   1512-wide frame) against the 6048 a BGRA row needs. `PixelBufferCopy.depad` refused all 608 of
   them, which is exactly right, and `ScanLoop` then bailed at its `!frame.isEmpty` gate forever.
   **The symptom is indistinguishable from a missing Screen Recording grant** — a permanently empty
   frame — which is why this cost the session rather than a minute. Fixed by asking for
   `kCVPixelFormatType_32BGRA` explicitly; pinned by `planarStrideIsRejected` in
   `ScreenCaptureTests`. Never trust a default pixel format.
2. **Module 8's point-vs-pixel fix did not work, because `SCDisplay.width` is itself in points.**
   The comment claimed pixel dimensions; the stream ran at 1512×982 on a 3024×1964 display, i.e.
   quarter resolution — the trap the plan recorded, avoided in prose and not in fact. Fixed with
   `CGDisplayCopyDisplayMode(display.displayID).pixelWidth/.pixelHeight`. Note the live frame's
   stride is 12160 against a tight 12096, so the row padding module 8 warned about is real and
   `depad` is load-bearing on every frame.
3. **A TCC grant added by hand through the Settings pane's `+` button does not necessarily match
   the running process.** Accessibility read as ON in System Settings while the daemon's own
   `AXIsProcessTrusted()` returned false, indefinitely. Replacing the bundle on disk (which this
   session did five times) leaves the row in place but stales the recorded signature. Toggling it
   off and on does **not** repair it; `tccutil reset Accessibility com.holyblocker.daemon` followed
   by the process requesting access itself does. **Onboarding must therefore call
   `requestAccess(to:)` from the daemon and never instruct a user to add the app by hand** — Screen
   Recording never had this problem precisely because it was registered by the process that asked
   for it. `runAgent` now requests Accessibility once per launch when it is not held.
4. **The render loop was unobservable, and that is a defect of its own.** With the overlay as its
   only output, a failure anywhere in capture → AX → policy → window looks identical from outside.
   `AgentRenderLoop` now prints one line per *state change* (plus a 10s heartbeat) carrying frame
   geometry, delivery tallies with per-cause drop counts, the live Accessibility state, AX text
   **length only — never its content**, verdict, intent and whether the overlay is up. Every finding
   above came from that line; none was diagnosable without it.
5. **`tccutil reset` resolved and ran unprivileged** against `com.holyblocker.daemon` once the app
   was in `/Applications`. The earlier `OSStatus -10814` was only a missing bundle to resolve, not a
   privilege check — see [backlog.md](backlog.md) item 2, whose *standard-user* half is still open.

#### The response is no longer only an overlay — module 17, `WindowSuppression`

The live pass killed the assumption underneath module 10. **Covering the screen is not covering
content**: the overlay is a picture drawn on top of windows the window server is still compositing,
and anything that re-arranges them goes around it. Two routes were found within a minute of the
first successful block, both by the user rather than by design review:

- **Click the desktop.** Every window unfocuses at once, the frontmost-window scan finds nothing,
  the verdict flips to `allow`, and the cover tears down over content that never moved.
- **Four-finger swipe up.** Mission Control is composited by the Dock above `.screenSaver` — the
  highest level `OverlayPlan` has — and its previews are live. Filed as backlog item 2.

So a block now draws the interstitial **and** removes the offending application from the screen.
`WindowSuppression.swift` is the pure decision (`SuppressionDecision.command`) plus an
`ApplicationHiding` edge over `NSRunningApplication.hide()`, which needs **no TCC grant** — it is
application-level window management, not accessibility control of another process. Verified live:
blocking text in TextEdit produces `hiding: com.apple.TextEdit` and the application goes to
`visible: false`.

Four decisions worth keeping:

- **Hide, never close.** Closing a window discards unsaved work, and a blocker that loses a
  half-written document gets uninstalled. Hiding removes it from the screen *and* from Mission
  Control's previews, and is one Dock click from recovery.
- **Block only.** A warn is an interstitial the user is meant to be able to think past; taking their
  window away is not a weaker response than asking them a question.
- **A protected set that must never be hidden** — ourselves (hiding this process takes the overlay
  with it), Finder (takes the desktop), Dock/SystemUIServer/loginwindow. They are the shell, not
  content.
- **The verdict has to carry its target.** `AXElementProbing` gained `lastWalkedApplication` and
  `AccessibilityScanner` a `lastVerdictApplication`, retained across rate-limited ticks. Re-reading
  `NSWorkspace.frontmostApplication` at response time is a race the scan cadence loses, and the cost
  of losing it is hiding an innocent window.
- **A refused hide must not stamp the cooldown**, or one refusal buys the application five quiet
  seconds on screen — the exact window the response exists to close.

This does **not** close the coverage gap, and the plan should not pretend it does: it acts on
content the daemon has *seen*. Content that is never scanned — a second window beside the focused
one, another display — produces no verdict at all and is neither covered nor hidden. That is backlog
item 3, and it is still the largest gap on this platform.

What did **not** need fixing: `AccessibilityText`, `AccessibilityScanner`, `ScanLoop`,
`OverlayPlan`/`OverlayController`, the UniFFI seam, and the signing identity all behaved exactly as
specified on their first live run. Grants survived four bundle replacements, which is the whole
point of the stable certificate.

### Outstanding verification — one item blocks a tamper-resistance claim

**Can a *standard* user reset a Screen Recording grant with `tccutil`?** Measured so far: `tccutil
reset ScreenCapture <bundle-id>` runs without `sudo` and fails only at bundle-ID resolution
(`OSStatus -10814`) — but it was run from an **admin** account, since the development machine's user
is in the `admin` group. Screen Recording lives in the system-wide TCC store rather than the per-user
one, so it plausibly fails for a standard user; "plausibly" is doing far too much work for the
assumption the entire account model rests on.

This needs a real standard-user account on a Mac and takes minutes to settle. **No tamper-resistance
claim should ship before it is run**, because if a standard user *can* run it, the standard-user
configuration is worth much less than the model above assumes and the plan needs revisiting.

Real classifiers replace `NullScanner` once the pipeline is proven end to end; `packages/image-sandbox`
is the image side and already runs under `mitm-proxy`, so the work is binding it to a frame rather
than building it.

---

## What this package does not cover

- **Classification** — text-policy, OCR, and image ML stay in their own packages. The daemon calls
  them; it does not implement them.
- **TLS interception** — `packages/mitm-proxy` owns it. The daemon only installs trust and routes
  traffic.
- **Safari extension** — Safari Web Extensions must ship inside a notarized App Store app. Safari
  coverage leans on the proxy alone for now; revisit only if the project ships a Mac App Store
  build.
- **Process injection** — impossible on macOS (SIP ignores `DYLD_INSERT_LIBRARIES` for Apple
  binaries; the hardened runtime blocks it for notarized third-party apps) and not in the core
  model anyway.
- **iOS** — a separate, lesser product. See the iOS section of
  [content-interception.md](../../decisions/content-interception.md).
