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

---

## macOS

| Route | Layer that should cover it | Status | Where | Evidence / Notes |
|---|---|---|---|---|
| Explicit text in the frontmost window | mac-daemon `AccessibilityScanner` | **Covered** | branch `feat/mac-daemon-live-e2e-pass` | Live e2e pass: real AX walk → `text-policy` scoring Block at 0.80 → overlay on screen, torn down when the text goes |
| Explicit text in a window that is **not** frontmost | — | **Uncovered** | — | `SystemAXProbe.focusedRoot()` reads `NSWorkspace.frontmostApplication` + `kAXFocusedWindowAttribute` and walks nothing else. Click away and the overlay tears down while the content is still on screen |
| Explicit imagery anywhere on screen | mac-daemon `ImageScanner` | **Unverified** | branch `feat/mac-daemon-image-scanner`, **not on master** | Focus-independent by construction. The classifier has never seen a real captured frame — `SCStream` and `ImageGuard` have only met through a synthetic buffer |
| Explicit text rendered as pixels (image with text, canvas, video subtitle) | an OCR module | **Not built** | — | `ScanLoop` reserves an OCR cadence slot; there is no OCR module anywhere in the Layer 2 plan |
| Blocked app kept visible after a block | `WindowSuppression` | **Covered** | `feat/mac-daemon-live-e2e-pass` | `NSRunningApplication.hide()`; block only, never warn; protected set never hidden |
| Content visible through Mission Control | — | **Uncovered** | — | Mission Control renders live previews above `.screenSaver`. Filed in the mac-daemon backlog, not fixed |
| Content in native fullscreen | overlay | **Unverified** | `feat/mac-daemon-live-e2e-pass` | Never observed over native-fullscreen video |
| Content on a second display | overlay | **Unverified** | `feat/mac-daemon-live-e2e-pass` | Reconcile across display connect/disconnect never observed |
| DRM/HDCP-protected video | — | **Uncovered** | — | Captures as black by design (platform limit, not a defect) |
| HTTPS page content via the proxy | Layer 1 `mitm-proxy` + `CATrust` | **Partial** | `master` | Firefox 152 renders an HTTPS page through the proxy with the CA installed. Condition: pinned clients (Firefox's own Mozilla endpoints) fail closed, as expected, and are not covered by this row |
| Plain **HTTP** request content | `mitm-proxy` | **Uncovered** | `master` | `forward::forward_http` takes no `ScanHooks` at all — URL, body and image scanning apply only to CONNECT-tunnelled traffic |
| OpenSSL-based clients through the proxy | `mitm-proxy` | **Uncovered** | `master` | Leaf certs carry no Authority Key Identifier; Python/Node/Linux curl reject them |
| Revoking Screen Recording or Accessibility mid-session | `PermissionGate` | **Unverified** | `master` | A real grant is now observed; **a real revocation has never been** |
| `tccutil reset` from a **standard** (non-admin) account | `PermissionGate` polling | **Unverified** | `master` | Measured only from an admin account. This single measurement blocks any tamper-resistance claim |
| A protected user who is a local admin | — | **Uncovered** | — | By design, not by gap: open product decision in `docs/decisions/content-interception.md`; `assess()` reports `weakened` and does not refuse to run |

## Android

| Route | Layer that should cover it | Status | Where | Evidence / Notes |
|---|---|---|---|---|
| Explicit text on screen | `ScreenGuardService` (AccessibilityService) | **Covered** | `master` | Verified on an android-36 emulator, including split screen |
| Explicit imagery on screen | MediaProjection capture → classifier | **Not built** | `feat/mobile-screen-capture` | Capture and gating are done; the sink is a counter. The classifier needs the LiteRT backbone + `packages/classifier-head`, neither of which exists |
| Plaintext DNS to a blocked domain | `NetworkGuardService` | **Covered** | `feat/mobile-vpn-service` | `scripts/smoke-test-vpn.sh` on an android-36 emulator |
| **DoH** (Chrome's default where the resolver supports it) | — | **Uncovered** | — | Never reaches port 53; the TUN's single `/32` route means the 443 flow never reaches us either |
| **DoT** via Android's Private DNS toggle | — | **Uncovered** | — | System-wide, one Settings toggle, no signal |
| Hardcoded resolver, DNS on a non-standard port, direct-IP connection | — | **Uncovered** | — | Same class. Forced by the `/32` route, which is itself forced by the platform |
| A forged DNS answer from an off-path attacker | `NetworkGuardService.ask()` | **Uncovered** | `feat/mobile-vpn-service` | Live defect, not a stated design gap: `ask()` uses an unconnected `DatagramSocket`, no transaction-ID or question check; the answer is wrapped and written into the TUN |
| SNI / IP-level filtering | `net-shield` | **Not built** | — | Not built on Android specifically — needs a userspace TCP stack; widening the route first black-holes the device |
| Uninstall or Device Admin removal while armed | Device Admin + `SettingsGuard` | **Covered** | `master` | Verified on an android-36 emulator |
| Disabling the guard via `adb`, guest account, or an unrecognised OEM screen | `GuardStatusService` + `TamperLog` | **Covered** | `master` | Covered means *observed to record the event*, not to prevent it — records what it cannot prevent, by design |
| Revoking the VPN from Settings | — | **Uncovered** | `feat/mobile-vpn-service` | Recorded (`NETWORK_GUARD_REVOKED`), not prevented. The VPN pane is deliberately not in `SettingsProfiles` — identifiers are dumped from a device, never inferred |
| Swipe-kill from Recents on One UI / HyperOS | — | **Unverified** | — | Deferred, blocked on real hardware. Cannot be reproduced on an emulator, so no mitigation can be verified |
| An empty, truncated, or deleted `blocklist.txt` | — | **Uncovered** | `feat/mobile-vpn-service` | An open guard with no signal |

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
| Can a user install this today and have anything blocked end to end? | **No** | The macOS text path is the closest and lives on an unmerged branch 20 commits behind `master` |
| Does any layer prove it is still alive, rather than merely not complaining? | **No** | Three separate fail-open-and-silent incidents so far, each fixed as an instance. No component asserts the permitted direction at runtime |
| Is there one place a user-facing statement of coverage could be generated from? | **This file** | It is new and incomplete. Rows marked Unverified outnumber Covered |
| DNS blocking via the signed blocklist artifact | `net-shield` `DnsShield` + `BlocklistArtifact` | **Unverified** | branch `feat/domain-blocklist-netshield` | The precedence table, mmap-backed artifact (`load` verifies signature + digest over the two-slot layout), and bounded-budget worker are implemented and tested against a two-slot artifact the *test helper* wrote. Module 7 `cli` is now done and does produce a real artifact (a fixture-mode dry run passes every gate; a real publish writes the slot layout; a second run rotates and re-gates against the loaded previous manifest — all verified live at runtime), but that producer's output has not yet been fed through `BlocklistArtifact::load` — the load path and the producer have implementations that agree on the byte layout by construction, but the two ends have not actually met in one run — and no device has observed it blocking |
| SNI/IP-level blocking via the artifact | `net-shield` `NetShield` (Wintun) | **Not built** | branch `feat/domain-blocklist-netshield` | The FST is wired into the DNS path only; `NetShield::process_packet` still consults `DomainFilter` alone. Recorded narrowing, not an oversight — the plan's module-6 budget section targets `DnsShield` |
