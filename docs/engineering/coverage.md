# Coverage ledger

What actually gets blocked, on which platform, by which layer — and what does not.

Every component in this repository is scoped narrowly and honestly, and each narrowing is recorded
in its own plan or status row. Nobody computes the union. **The union is the product.** This file is
the union.

## How to read it

| Status | Meaning |
|---|---|
| **Covered** | A real instance was blocked end to end and observed. The Evidence column names how. |
| **Partial** | Covered under stated conditions only; the conditions are in Notes. |
| **Uncovered** | Nothing in the repository addresses this. Not a bug — a stated gap. |
| **Unverified** | Code exists and is expected to cover it; no one has observed it doing so. |
| **Not built** | The component that would cover it does not exist. |

The `Status` cell holds exactly one of these five values — never a qualifier stapled onto one
(`Covered (recorded, not prevented)`, `Uncovered by design`). Qualifiers, caveats, and "why" belong
in `Evidence / Notes`. This matters specifically for `Covered`: several rows below are covered in
the sense of *observed to record the event*, not observed to *prevent* it — that distinction lives
in Notes, not in a second status vocabulary, so the five values stay comparable across rows.

`Where` names the exact branch the evidence lives on, not just "branch" — a branch can move or be
rebased, and a reader needs to find the commit, not just know one exists somewhere. Work described
as done in `CLAUDE.md` but living on an unmerged branch has not shipped, and this column is why the
distinction is tracked here rather than in the status table.

## Rules

1. A PR that changes what any layer covers updates this file in the same PR.
2. **Unverified may not be promoted to Covered by an argument.** Only by an observation, named.
3. A row may not be deleted because it is uncomfortable. Move it to Uncovered and say why.
4. New rows come from asking "what would a user reasonably expect this to catch?", not from the
   module list.
5. `Status` is one of the five values above, always. `Where` is `master` or an exact branch name,
   never the bare word "branch".
6. A row cites a scenario as `scenario:<id>`, with the host class and the date, and the scenario must name
   the row in its `proves` list. Only the owner promotes a row to `Covered`
   ([e2e-scenario-contract.md](../decisions/e2e-scenario-contract.md)).

---

## macOS

| Route | Layer that should cover it | Status | Where | Evidence / Notes |
|---|---|---|---|---|
| Explicit text in the frontmost window | mac-daemon `AccessibilityScanner` | **Covered** | `master` | Observed on the pre-merge branch `feat/mac-daemon-live-e2e-pass`, which no longer exists on `origin`; `master` carries the BGRA capture fix and `WindowSuppression`, and the pass has not been repeated on the merged tree. Live e2e pass: real AX walk → `text-policy` scoring Block at 0.80 → overlay on screen, torn down when the text goes |
| Explicit text in a window that is **not** frontmost | — | **Uncovered** | — | `SystemAXProbe.focusedRoot()` reads `NSWorkspace.frontmostApplication` + `kAXFocusedWindowAttribute` and walks nothing else. Click away and the overlay tears down while the content is still on screen |
| Explicit imagery anywhere on screen | mac-daemon `ImageScanner` | **Unverified** | `master` | Code is on `master`, and the agent runs it only when the model is bundled and both thresholds are set, so it is off by default. Focus-independent by construction. The classifier has never seen a real captured frame — `SCStream` and `ImageGuard` have only met through a synthetic buffer |
| Explicit text rendered as pixels (image with text, canvas, video subtitle) | an OCR module | **Not built** | — | `ScanLoop` reserves an OCR cadence slot; there is no OCR module anywhere in the Layer 2 plan |
| Blocked app kept visible after a block | `WindowSuppression` | **Covered** | `master` | `NSRunningApplication.hide()`; block only, never warn; protected set never hidden |
| Content visible through Mission Control | — | **Uncovered** | — | Mission Control renders live previews above `.screenSaver`. Filed in the mac-daemon backlog, not fixed |
| Content in native fullscreen | overlay | **Unverified** | `master` | Never observed over native-fullscreen video |
| Content on a second display | overlay | **Unverified** | `master` | Reconcile across display connect/disconnect never observed |
| DRM/HDCP-protected video | — | **Uncovered** | — | Captures as black by design (platform limit, not a defect) |
| HTTPS page content via the proxy | Layer 1 `mitm-proxy` + `CATrust` | **Partial** | `master` | Firefox 152 renders an HTTPS page through the proxy with the CA installed. Condition: pinned clients (Firefox's own Mozilla endpoints) fail closed, as expected, and are not covered by this row |
| Plain **HTTP** request content | `mitm-proxy` | **Uncovered** | `master` | `forward::forward_http` takes no `ScanHooks` at all — URL, body and image scanning apply only to CONNECT-tunnelled traffic. Host refusal from the domain list does apply to plain HTTP (see "Listed domain through the HTTPS proxy") |
| OpenSSL-based clients through the proxy | `mitm-proxy` | **Uncovered** | `master` | Leaf certs carry no Authority Key Identifier; Python/Node/Linux curl reject them |
| Blocking a user-chosen app | `ScreenGuardService` + `CustomAppEnforcement` | **Partial** | `feat/mobile-custom-app-enforcement` | Android only. Observed on an API 36 emulator by `apps/mobile/scripts/smoke-test-custom-app.sh`: a listed app is sent home, a cover window appears and then lifts once the app has gone, an unlisted app is untouched, and nothing is enforced while protection is disarmed. Send-home backgrounds the app; its process keeps running, so "covered", not "stopped". The cover arrives about when the app leaves: its content was foreground for roughly a second first. The list is seeded through `run-as` because no panel exists yet (`custom-app-panel`). Not observed: an app already foreground or in split-screen when it is added (rescan on list change is unit-built, not observed), real-device or vendor-skin behaviour, clones and work-profile copies (not covered by design). macOS not built. Design: [custom-app-blocking.md](../decisions/custom-app-blocking.md). Steps: `custom-app-panel`, `custom-app-domains` (name-matched domains) |
| Daemon survives logout and reboot, and a standard user cannot remove it | — | **Not built** | — | No launchd job installs the daemon, and nothing refuses its removal. Steps: `mac-daemon.install`, `mac-daemon.persistence`, `mac-daemon.standard-user-tamper`, `mac-daemon.standard-user-removal` |
| Revoking Screen Recording or Accessibility mid-session | `PermissionGate` | **Unverified** | `master` | A real grant is now observed; **a real revocation has never been** |
| `tccutil reset` from a **standard** (non-admin) account | `PermissionGate` polling | **Unverified** | `master` | Measured only from an admin account. This single measurement blocks any tamper-resistance claim |
| Listed domain through the HTTPS proxy | `mitm-proxy` | **Partial** | `feat/mitm-proxy-domain-blocklist` | Built and unit/integration-tested, not observed in a browser: a listed host is refused on the CONNECT authority, the TLS SNI and the inner `Host` header, and on plain HTTP; the list is the signed artifact loaded with `--blocklist-dir` and `--blocklist-key`. Not covered: IP literals are never matched (a CONNECT to a listed site's IP with an unlisted SNI passes if its `Host` header is also unlisted); encrypted ClientHello shows only the public name; with no `--blocklist-dir` nothing is filtered and nothing is logged; a rolled-back or `previous/`-slot list is not reported; the refusal ignores `ProtectionMode`. Step: `mac-daemon.proxy-blocklist-wiring` |
| Bundled domain list reaches the supervised proxy | mac-daemon `ProxyBlocklist` + `MitmProxyProcess` | **Covered** | `feat/mac-daemon-proxy-blocklist` | `bundle` seals a signed two-slot list and its public keys under `Contents/Resources/blocklist`; `run` passes `--blocklist-dir` and one `--blocklist-key` per key. `bundle` refuses to assemble without a list, and `run` refuses to route traffic when the bundle has no usable list; `HOLY_BLOCKER_ALLOW_NO_BLOCKLIST=1` overrides both for development only. Not covered: a deleted list in an already-installed bundle is a refusal to start, not a recorded tamper event (no tamper log). Observed: a fixture list staged by `bundle` refuses a listed host with 403 to `URLSession` through the proxy and forwards an unlisted one. Also observed through the system proxy set by the root LaunchDaemon: 403 for a listed host over plain HTTP, 200 for an unlisted one. Not distinguished: over HTTPS a listed host and an unresolvable one both surface as a dropped connection, so the HTTPS refusal rests on the proxy's own tests. The list is a fixture signed by a throwaway key |
| A macOS tamper log | mac-daemon `TamperLog` + `TamperLogStore` | **Partial** | `feat/mac-tamper-log` | Fixed content-free code vocabulary (asserted by test), bounded at 2,000 entries (up to 2,200 between trims) with a `log_trimmed` marker, torn final line tolerated and not allowed to swallow the next record, file mode 0600 opened `O_NOFOLLOW` in a directory the daemon's user owns and others cannot write, details limited to a closed type, session-start classification (clean restart, reboot, unclean stop) from uptime. `PermissionGate` transitions map to codes with user names dropped. Not covered: nothing yet calls `record` — the daemon does not write session boundaries or feed `poll()` into the store, and the state directory is not created by an installer. Not observed on an installed daemon. A deleted or unreadable log reads as a first run (`mac-daemon.tamper-log-hardening`). Steps: `mac-daemon.tamper-log` (this), `mac-daemon.settings-guard`, `mac-daemon.install` |
| Turning protection down on macOS | mac-daemon `ProtectionSchedule` + `ProtectionRecord` | **Partial** | `feat/mac-protection-schedule` | Pure request, cooldown, confirm, expiry and re-arm, held to the same `test-vectors/protection-schedule.tsv` as Android (both suites run it). Timing is `CLOCK_MONOTONIC_RAW`, observed by comparing readings against boot time to count through sleep where `ProcessInfo.systemUptime` does not (a real sleep cycle was not run); timestamps are tied to `kern.bootsessionuuid`, so a request or disarm from another boot is void (the replay once the new clock catches up is closed); an overflowing timestamp reads as armed. Transitions return the tamper code to record. Not covered: nothing persists a `ProtectionRecord` or calls it, no UI to request or confirm, and nothing consults `guardActive` (`mac-daemon.settings-guard`); a real reboot and a real sleep were not observed; a missing record evaluates to off, so the store must tell never-armed from unreadable and fail closed. Android has the reboot replay (`mobile.protection-reboot-replay`). Steps: `mac-daemon.protection-schedule` (this), `mac-daemon.settings-guard` |
| A protected user who is a local admin | — | **Uncovered** | — | By design, not by gap: open product decision in `docs/decisions/content-interception.md`; `assess()` reports `weakened` and does not refuse to run |

## Android

| Route | Layer that should cover it | Status | Where | Evidence / Notes |
|---|---|---|---|---|
| Explicit text on screen | `ScreenGuardService` (AccessibilityService) | **Covered** | `master` | Verified on an android-36 emulator, including split screen |
| Explicit imagery on screen | MediaProjection capture → classifier | **Not built** | `master` | Capture and gating are done; the sink is a counter. The classifier needs the LiteRT backbone + `packages/classifier-head`, neither of which exists |
| Plaintext DNS to a blocked domain | `NetworkGuardService` | **Covered** | `master` | `scripts/smoke-test-vpn.sh` on an android-36 emulator |
| **DoH** (Chrome's default where the resolver supports it) | — | **Uncovered** | — | Never reaches port 53; the TUN's single `/32` route means the 443 flow never reaches us either |
| **DoT** via Android's Private DNS toggle | — | **Uncovered** | — | System-wide, one Settings toggle, no signal |
| Hardcoded resolver, DNS on a non-standard port, direct-IP connection | — | **Uncovered** | — | Same class. Forced by the `/32` route, which is itself forced by the platform |
| A forged DNS answer from an off-path attacker | `NetworkGuardService.ask()`, `DnsAnswer` | **Unverified** | `fix/mobile-dns-forged-answer` | The forwarding socket is connected to the resolver and an answer is kept only if its transaction ID and question match the query (`DnsAnswer`, unit-tested). No forged answer has been sent at a device, so this is not observed. Resolver port and source address are the only entropy a connected socket adds; an on-path attacker is out of scope |
| Domain list on a fresh install | `BlocklistStore` | **Covered** | `feat/mobile-blocklist-provisioning` | The APK bundles a signed two-slot artifact and its trusted public keys (Gradle `-PblocklistArtifactDir`/`-PblocklistTrustedKeyDir`); the app installs it under `filesDir` and loads it with `DnsGuard.withArtifact`. Observed on an android-36 arm64 emulator by `smoke-test-vpn.sh` with no `adb push`: a listed name is refused, a permitted one resolves. The list in that run is a fixture signed by a throwaway key; the repository ships no real list or release key |
| SNI / IP-level filtering | `net-shield` | **Not built** | — | Not built on Android specifically — needs a userspace TCP stack; widening the route first black-holes the device |
| Blocking a user-chosen app | — | **Not built** | — | The list, its removal cooldown and the protected-set check exist as pure code and a `filesDir` store (`CustomApps.kt`, `CustomAppStore.kt`; unit-tested, no caller yet), so nothing is enforced. Neither `ScreenGuardService` nor `SettingsGuard` checks which app owns a window; they watch content and settings screens. Design: [custom-app-blocking.md](../decisions/custom-app-blocking.md). Steps: `custom-app-core`, `custom-app-enforcement`, `custom-app-panel`, `custom-app-domains` (name-matched domains) |
| Uninstall or Device Admin removal while armed | Device Admin + `SettingsGuard` | **Covered** | `master` | Verified on an android-36 emulator |
| Disabling the guard via `adb`, guest account, or an unrecognised OEM screen | `GuardStatusService` + `TamperLog` | **Covered** | `master` | Covered means *observed to record the event*, not to prevent it — records what it cannot prevent, by design |
| Revoking the VPN from Settings | — | **Uncovered** | `master` | Recorded (`NETWORK_GUARD_REVOKED`), not prevented. The VPN pane is deliberately not in `SettingsProfiles` — identifiers are dumped from a device, never inferred |
| Swipe-kill from Recents on One UI / HyperOS | — | **Unverified** | — | Deferred, blocked on real hardware. Cannot be reproduced on an emulator, so no mitigation can be verified |
| A missing or unverifiable bundled list | `BlocklistStore`, `TamperEvent.BLOCKLIST_MISSING`/`BLOCKLIST_REJECTED` | **Covered** | `feat/mobile-blocklist-provisioning` | The guard still starts on the placeholder rules and records `list_missing` or `list_rejected`. `list_rejected` observed once by hand on the emulator (installed slot, APK with no trusted key); a truncated installed `current` slot is re-copied on the next start (observed by hand: 0 to 123 bytes, list loaded); `list_missing` rests on the Rust loader tests and the event-mapping unit test, not a device run. A validly signed list with zero entries is not detected on-device; the publish gates are what stop one. The loader falls back to `previous/` without saying so when `current/` fails verification, so that case is unrecorded: step `mobile.blocklist-provisioning-hardening` |

## MVP

[mvp-scope.md](../decisions/mvp-scope.md) defines the MVP as four requirements on Android and macOS.
This section adds no routes of its own: each requirement is decided by rows above, and this table
names which.

| Requirement | Decided by | Status | Notes |
|---|---|---|---|
| Block adult content | macOS and Android rows for text, DNS and the HTTPS proxy; imagery is out of scope | **Unverified** | Android text and DNS are Covered on `master`, but a fresh install has no domain list. macOS text is Covered on the frontmost window only and the proxy has no domain list |
| Block user-chosen apps | the two "Blocking a user-chosen app" rows | **Not built** | No per-app deny check exists on either platform |
| Not deletable | Android Device Admin and `SettingsGuard` rows, macOS permission, standard-user and persistence rows | **Unverified** | Android is Covered while armed. macOS has no install, tamper log or protection-down path, and the standard-user measurement is missing |
| Developer escape hatch | — | **Not built** | A build property, not a blocking route; steps `mobile.dev-flavor`, `mobile.dev-kill-switch`, `mobile.dev-sibling-event`, `mobile.release-signing`, `mac-daemon.dev-build`, `mac-daemon.dev-kill-switch`, `mac-daemon.dev-sibling-event`, `mac-daemon.release-identity`, `engineering.dev-build-release-guard`, `engineering.dev-build-release-guard-macos` |

## Windows

| Route | Layer | Status | Where | Notes |
|---|---|---|---|---|
| Anything | `win-daemon`, `win-network` | **Not built** | `master` | WinEvent hooks and a message loop only; no capture, OCR, or IPC. `net-shield`'s Wintun path exists and is not driven by a service |

## iOS

| Route | Layer | Status | Where | Notes |
|---|---|---|---|---|
| Anything | — | **Not built** | — | Investigated only. Local VPN is the sole route without Family Controls; deferred pending hardware |

## Cross-cutting

| Question | Status | Notes |
|---|---|---|
| Can a user install this today and have anything blocked end to end? | **No** | Android text is observed on `master` and macOS text was observed before merge; no scenario under [e2e-scenario-contract.md](../decisions/e2e-scenario-contract.md) has recorded either, and macOS has no install path. Android is closer |
| Is the MVP in [mvp-scope.md](../decisions/mvp-scope.md) met on both platforms? | **No** | See the MVP section above; it is met when the first row of this table reads Yes on both platforms and every MVP row is Covered |
| Does any layer prove it is still alive, rather than merely not complaining? | **No** | Three separate fail-open-and-silent incidents so far, each fixed as an instance. No component asserts the permitted direction at runtime |
| Is there one place a user-facing statement of coverage could be generated from? | **This file** | It is new and incomplete. Rows marked Unverified outnumber Covered |
| DNS blocking via the signed blocklist artifact | `net-shield` `DnsShield` + `BlocklistArtifact` | **Unverified** | `master` | The precedence table, mmap-backed artifact (`load` verifies signature + digest over the two-slot layout), and bounded-budget worker are implemented and tested against a two-slot artifact the *test helper* wrote. Module 7 `cli` is now done and does produce a real artifact (a fixture-mode dry run passes every gate; a real publish writes the slot layout; a second run rotates and re-gates against the loaded previous manifest — all verified live at runtime), and `net-shield-ffi` tests now feed the producer's `build` output through `BlocklistArtifact::load` via `DnsGuard.withArtifact` (the tests write the slot files and the bincode manifest themselves, so the CLI's published slot files are still unread, and no device has run it) — the load path and the producer have implementations that agree on the byte layout by construction, but the two ends have not met in one run of the CLI and the loader — and no device has observed it blocking |
| Keyword rule: names carrying a confirmed app token | `net-shield` `KeywordRules` + `DnsGuard.setKeywords` | **Unverified** | `master` | Unit-tested for label equality, containment at any depth, the public-suffix boundary, the 5-character minimum, the stoplist and precedence below the allowlist and explicit rules. The proxy consumes the same rule through `ScanHooks::set_keywords` (CONNECT authority, SNI, plain HTTP host and inner `Host` header; tokens from `--keyword-file`), covered by unit and loopback tests. Nothing feeds either path real tokens yet and no device has observed a name refused. Misses a service whose domains do not carry its name and can block an unrelated site that does (`docs/decisions/custom-app-blocking.md`, decision 12) |
| SNI/IP-level blocking via the artifact | `net-shield` `NetShield` (Wintun) | **Not built** | `master` | The FST is wired into the DNS path only; `NetShield::process_packet` still consults `DomainFilter` alone. Recorded narrowing, not an oversight — the plan's module-6 budget section targets `DnsShield` |
