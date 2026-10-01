# 7. `PermissionGate` — TCC state, and the account model that locks it

Part of the [macOS Daemon plan](../plan.md).

```
Sources/MacDaemon/PermissionGate.swift
```

**Built, except for the live grant/revocation check — see the notes at the end of this section.**

Detect and report which permissions are held, and detect the configuration that silently defeats
the tamper model.

```swift
enum PermissionState { case granted, denied, undetermined }

enum Capability { case screenRecording, accessibility, inputMonitoring }

struct PermissionSnapshot {
    var screenRecording: PermissionState
    var accessibility:   PermissionState
    var inputMonitoring: PermissionState

    // Environment signals — none of these needs a permission. See "the tamper surface" below.
    var protectedUserIsAdmin: Bool
    var sipEnabled:           Bool
    var localUsers:           [String]
    var adminGroupMembers:    [String]
    var bootTime:             Date
}
```

Responsibilities:

- Read state through the **preflight APIs**, which do not prompt:
  `CGPreflightScreenCaptureAccess()`, `AXIsProcessTrusted()`, and
  `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)`. All three were verified to compile and run
  against the current toolchain; on an ungranted machine they return `false`, `false`, and
  `kIOHIDAccessTypeUnknown` (raw value 2) respectively.
- **Do not read `TCC.db`.** It is SIP-protected, reading it requires Full Disk Access — a *stronger*
  permission than the ones being inspected, which is an absurd dependency — and its schema is
  private and has changed across releases. The preflight APIs are the supported surface.
- Prompt exactly once per capability, deliberately, from the onboarding path — never as a side
  effect of a scan. `CGRequestScreenCaptureAccess()` and
  `AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt: true])`. Note that Screen Recording's
  prompt cannot be re-shown once denied; the only recovery is deep-linking the user to
  System Settings (`x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture`).
- **Poll for revocation** and emit a state transition when a held permission is lost. This is the
  Android `GuardStatus` pattern, and it is the module's most important job — a revoked Screen
  Recording grant is indistinguishable from "nothing bad is on screen" unless it is detected
  explicitly.
- Detect whether the protected user is a local administrator, which is the open product question
  below.

The pure part — mapping a `PermissionSnapshot` to a "what is actually protected right now"
assessment and to state transitions worth reporting — is a plain function over the struct and gets
tests first. The three preflight calls are the edge and go behind a protocol, mirroring
`CommandRunner`.

## This module is the tamper surface — watch outcomes, not routes

The design goal under the admin model is **"no bypass route that is both easy and silent"** (see
[content-interception.md](../../../decisions/content-interception.md) for the route-by-route coverage
table). `PermissionGate` is where nearly all of that lands, and the reason is a single observation:
**revoking Screen Recording through System Settings and running `tccutil reset` produce the same
outcome, so one poll detects both.** Guarding a route covers one door; watching the outcome covers
every door that leads to it.

Four extra environment signals belong in the snapshot. All four were verified readable on macOS
26.5.2 with **no permissions granted at all** — no Screen Recording, no Accessibility:

| Signal | Source | Detects |
|---|---|---|
| SIP state | `csrutil status` | the precondition for every deep bypass (SIP-off makes `TCC.db` directly writable) |
| Local users | `dscl . -list /Users` | a second account created to escape the guard |
| Admin members | `dscl . -read /Groups/admin GroupMembership` | the protected user self-granting admin |
| Boot time | `sysctl kern.boottime` | correlating an unclean stop with a reboot |

The last one is the Android lesson transplanted: `TamperLog.classifyConnect` anchors on session
boundaries rather than reading a single line, and the same reasoning applies here — a daemon that
died and a machine that rebooted are different events and must not be conflated.

Transitions from this module feed a macOS tamper log modelled directly on `apps/mobile`'s
`policy/TamperLog.kt` (pure) + `TamperLogStore.kt` (the filesystem edge) split. That is a port of a
proven design, not a new one.

## Admin detection, and the product decision it forces

The tamper model in
[content-interception.md](../../../decisions/content-interception.md) requires the protected person to
be a **standard user** with an accountability partner holding the admin password. If the protected
user is themselves an admin, every Layer 2 permission is revocable by them in two clicks, and —
as recorded in Layer 1's limits — `networksetup` proxy changes need no root for an admin either, so
Layer 1 falls with it. The entire tamper model is then decorative.

Detection is a group-membership read: `dscl . -read /Groups/admin GroupMembership`, or `id -Gn` for
the current user. **On the development machine this check currently returns true** — `admin` appears
in `id -Gn`, and `GroupMembership: root dutov _mbsetupuser` — so the machine Layer 2 is being built
on is itself the unprotected configuration. That is worth knowing before any tamper-resistance claim
is made from local testing.

The product decision is what to *do* about it, and it is genuinely open. Three options:

| Option | Consequence |
|---|---|
| Refuse to run | Honest, and useless for most Mac users — the majority are their own sole admin. Guarantees the product is uninstalled during onboarding. |
| Warn, run anyway, and **report the weakened state** | Onboarding names the limitation plainly; the daemon records it and, in partnered mode, surfaces it to the partner. |
| Proceed silently | Rejected outright. The product would be claiming protection it does not have. |

**Recommendation: the middle option.** It matches the accountability model in
[accountability.md](../../../decisions/accountability.md), where solo mode is a complete configuration
rather than a failure state — a self-admin user is in the same category, protected by friction and
their own intent rather than by a lock, and the tool should say so rather than either pretending
otherwise or refusing to help. The one thing that must not happen is the status UI showing the same
"protected" state in both configurations. This needs sign-off before onboarding is written; it is
listed as an open question in
[content-interception.md](../../../decisions/content-interception.md) and should be resolved there,
not here.

## What was built, and five things that only showed up in the writing

`PermissionGate.swift` ships `PermissionState` / `Capability` / `PermissionSnapshot` as specified,
plus `ProtectionAssessment` (the "what is actually protected" mapping), `TamperEvent` (the
transitions), `SystemEnvironment` (pure parsers for the four zero-permission signals),
`PermissionProbing` + `SystemPermissionProbe` + `FakePermissionProbe` (the preflight edge, mirroring
`CommandRunner`), and a `permissions` CLI verb. 38 tests.

1. **`CGPreflightScreenCaptureAccess` cannot distinguish denied from never-asked** — it returns a
   `Bool`, so two of the three `PermissionState` cases collapse. Not-granted is reported as
   `undetermined`, because the two states have different recovery paths and `undetermined` is the
   recoverable one; guessing `denied` would send onboarding to System Settings when a prompt would
   still work. Only `IOHIDCheckAccess` reports all three states.
2. **`csrutil status` has a third answer.** With individual protections toggled it prints
   `unknown (Custom Configuration)`, not `disabled`. That is parsed as *not* enabled — it is
   precisely the partially-disabled state the signal exists to catch — while genuinely
   unrecognised output returns `nil` and makes `snapshot()` throw rather than default.
3. **`dscl . -list /Users` alone is unusable** — it returns 133 rows on the development machine,
   all but one of them service accounts, so a bypass account created to escape the guard would be
   invisible in the noise. The listing is taken with `UniqueID` and filtered to UID ≥ 500 (Apple
   reserves everything below for the system; the first human account is 501), which reduces it to
   the one real name.
4. **`kAXTrustedCheckOptionPrompt` does not compile under Swift 6.** It is imported as a mutable
   global (`extern CFStringRef` in `HIServices/AXUIElement.h`), and reading it is a strict-
   concurrency error. The constant's literal value `"AXTrustedCheckOptionPrompt"` is used instead —
   it is the documented public name, not an implementation detail.
5. **`kern.boottime` moves without a reboot.** It is derived from the current clock, so an NTP
   correction shifts it by a second or two; a five-second tolerance keeps a clock adjustment from
   being logged as a restart. This is the Android `TamperLog.classifyConnect` lesson — a daemon
   that died and a machine that rebooted are different events — and `rebooted` is emitted first in
   the transition list so everything after it can be read against it.

**Still outstanding, and blocked on module 0:** a real grant, and a revocation observed through
`poll()`. Both need the stable signing identity module 0 produces. Running `permissions` from a
shell today reports the *terminal's* grants (the responsible-process rule), which the verb prints a
warning about; it is useful for the four environment signals, which were confirmed live, and for
nothing else. **The standard-user `tccutil` question in the verification section below is still
open** and is a separate matter from this module's code.

`assess()` implements the plan's recommended middle option — a self-admin user is reported as
`weakened`, never as `protected` — but it does **not** settle the product decision above. It makes
the two configurations distinguishable, which is the one outcome that must not be lost; what
onboarding *does* with that is still to be resolved in `content-interception.md`.

## Reference documents

- [Apple — `CGPreflightScreenCaptureAccess`](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()) — the non-prompting check and its companion `CGRequestScreenCaptureAccess`.
- [Apple — `AXIsProcessTrustedWithOptions`](https://developer.apple.com/documentation/applicationservices/1462089-axisprocesstrustedwithoptions) — the Accessibility check and the prompt option key.
- [Apple — `IOHIDCheckAccess`](https://developer.apple.com/documentation/iokit/3181425-iohidcheckaccess) — Input Monitoring state, and the `IOHIDAccessType` values.
- [Apple — Protecting user privacy (TCC)](https://support.apple.com/guide/security/controlling-app-access-to-files-sec51e0c5d5b/web) — which permissions are admin-modifiable and which are user-revocable.

---
