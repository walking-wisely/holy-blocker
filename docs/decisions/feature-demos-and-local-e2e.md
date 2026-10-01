# ADR: Feature demos and local e2e regression

| Field | Value |
|---|---|
| **Status** | Accepted — design only; nothing here is built |
| **Date** | 2026-10-01 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

---

## Context

The owner wants to feel the product when a **feature** is done, not at every step and not by
reading diffs. The pipeline should otherwise sustain itself. Two mechanisms were considered:
a hand-written smoke test per feature, and an e2e scenario that doubles as a demo.

A separate demo script rots: it is not run by anything, so it breaks silently on the first
refactor. The existing smoke scripts (`apps/mobile/scripts/smoke-test*.sh`) are the evidence
that this works when scripted, and the mac-daemon e2e sessions are the evidence of how much
a live pass finds that no unit test can reach.

## Decision

### 1. Test architecture is the primary gate

Each plan declares, per behaviour, which layer proves it:

| Layer | Runs | Proves |
|---|---|---|
| Unit | Everywhere, CI | Pure logic: policy, parsers, state machines |
| Integration | CI where the host allows | Package seams: FFI, IPC framing, ledger tooling |
| E2E scenario | **Locally**, on the owner's machine or an emulator | A real process on a real OS doing the real thing |

Anything an e2e scenario can observe never reaches the owner. `acceptance = "observation"`
is reserved for what no scenario can observe (see §5).

### 2. A demo is an e2e scenario with an interactive tail

One scenario file per feature seeds state, launches the app, and drives it to the
interesting screen.

- **Headless**, it drives there, asserts the state was reached, and exits. This is the
  regression run.
- **`--interactive`**, it drives to the same state and stops, leaving the app open. This is
  the demo.

There is no separate demo artifact to rot. A breaking change fails the scenario, so whoever
made it fixes the scenario in the same change.

Seeds go through the same code paths production uses, never by writing files or rows by hand,
so schema drift fails loudly. Fixtures are synthetic only (placeholder lexicon, RFC 2606
reserved names, a generated non-explicit image): the repo bars explicit samples.

A feature's plan ends in a `demo` step. Layout, once built:

```
demos/<feature>/
  scenario.<ext>     the scenario (headless + --interactive)
  seed/              synthetic fixtures
  README.md          a few lines: what to look at, what wrong looks like,
                     "last watched by a human at <commit>"
```

The README is deliberately short and carries a date-stamp, so a stale judgement is visible.

### 3. Local regression, not CI

CI stays at unit and integration. GitHub-hosted runners cannot supply a logged-in GUI
session, TCC grants, or a signed bundle, and an Android emulator is slow and flaky there.
Whether that is still true was **not checked**; the decision is made on cost, and is
revisited if CI runners with a GUI session become available.

Local e2e needs three things so that it does not depend on anyone remembering:

1. **Path-triggered gate.** A runner maps changed paths to the scenarios that cover them
   and runs those. `step-loop` calls it before merging a code step and records which
   scenarios ran, at which commit, in the PR body. A green CI does not imply the e2e layer
   passed; the PR record is the evidence.
2. **Preflight, distinct from failure.** Before running, the runner checks the environment
   (grants present, signing identity valid, emulator booted, privilege dispatcher
   installed — see [agent-privilege-boundary.md](agent-privilege-boundary.md)). A failed
   preflight reports **environment not ready**, never a regression. Without this, lost grants
   produce false failures and the owner learns to ignore red.
3. **Staleness report.** `e2e stale` lists scenarios whose covered paths changed since their
   last pass. `docs/engineering/coverage.md` rows cite the scenario and the commit it last
   passed at, so `Covered` always points at a dated observation.

### 4. Owner attention

The loop runs a feature's steps without the owner. It stops only for: a product or
architecture escalation ([decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md)),
a step it cannot verify on the available host, or the feature being done. At that point the
agent hands over one command that launches the demo. The owner gives impressions, not code
review, and the agent turns them into steps.

### 5. What cannot be automated

Even with the privilege dispatcher installed:

- **TCC grants** (Screen Recording, Accessibility, Input Monitoring). Root cannot grant them
  (`content-interception.md`, "No engine can grant a TCC permission"). One manual grant per
  signing identity; the stable `Holy Blocker Dev` identity keeps it valid across rebuilds.
- **System Settings approvals**: system extensions, login items, network extensions.
- **Android per-session consent**: the MediaProjection dialog is single-use; the VPN consent
  dialog needs a tap. Emulators can often be driven by `adb`; restricted real devices cannot.
- **Real hardware**: One UI and HyperOS Recents behaviour, DRM-black frames, FileVault
  pre-boot.
- **Feel**: flicker, timing, annoyance. A screenshot proves something is drawn, not that it
  is right.

These are one-time or rare, except the last, which is the point of the demo.

## Gotchas — macOS

Each is a measured finding recorded in `docs/components/mac-daemon/plan.md` or
`CLAUDE.md`; nothing here is new, and a scenario that ignores one will fail in a way that
does not point at the cause.

- **Never validate a TCC path from a shell.** A CLI launched from a terminal has its grant
  attributed to the *responsible process* (Terminal/iTerm), so it appears to work and then
  evaporates under `launchd`. Scenarios drive the signed bundle under the LaunchAgent.
- **Grants belong to a signing identity.** An unsigned or ad-hoc-signed rebuild invalidates
  them; the cdhash-derived signature changes every build. Use `HOLY_BLOCKER_SIGNING_IDENTITY`.
- **A hand-added TCC entry may not match the running process.** Settings can read ON while
  `AXIsProcessTrusted()` stays false. Preflight checks the process's own answer, and
  onboarding requests its own grants rather than telling a user to add the app.
- **"No frames" looks like "no grant".** `SCStreamConfiguration.pixelFormat` must be set
  explicitly or `ScanLoop` silently bails; a preflight that checks only the grant will blame
  the wrong thing. The render loop's one-line-per-state-change log is the diagnostic.
- **`SCStream` is change-driven.** A static image starves it; a scenario that displays a still
  frame must expect `.idle` deliveries and the retained last frame.
- **Replace the bundle, don't patch it.** A stale `_CodeSignature` makes `codesign` refuse.
- **Only the focused window is scanned.** A scenario that clicks away will see the overlay
  drop; that is a recorded coverage gap, not a flake.
- **Never verify coverage with macOS `curl`.** It is SecureTransport-built, ignores `--cacert`,
  and skips system proxy settings. Use `URLSession` or `openssl s_client -proxy`.
- **Run tests with `scripts/test.sh`, never bare `swift test`**, and a fresh checkout cannot
  build until `scripts/build-ffi.sh` has run.
- **Timers must be created before `NSApplication.run()`** and on the main run loop, or they
  never fire. Applies to any scenario hook that schedules work in the agent.
- **DRM/HDCP content captures as black** by design; do not use it as a fixture.
- **Native fullscreen, a second display, and Mission Control are unverified.** A scenario for
  them is new coverage, not regression.
- **The development machine is in the `admin` group** — the unprotected configuration — so a
  scenario passing here says nothing about a standard-user tamper model.
- **Covenant Eyes is installed** and occupies the network surface the proxy wants; a network
  scenario may be perturbed by it.

## Gotchas — Android

Recorded in `apps/mobile/` plans and `CLAUDE.md`; the target is the android-36 arm64 emulator.

- **Assert both directions.** A missing `INTERNET` permission looks exactly like the filter
  working: blocked names refused, permitted ones silently unanswered. Assert on a *positive*
  output (`ping` success), never an error string, or a broken guard reads as a pass.
- **netd answers from cache.** `ndc resolver flushnetdns` fails silently for a non-root shell,
  so the smoke script reboots to get a cold cache. A DNS scenario inherits that cost.
- **The VPN is not re-established after reboot** (`setAlwaysOnVpnPackage` is owner-only), so
  `BootReceiver` restores it. A reboot scenario must assert the restore, not assume it.
- **MediaProjection consent is per-session and single-use.** Capture cannot be restored after a
  reboot, and `adb` cannot drive the disarm-driven stop or revoke `PROJECT_MEDIA` mid-session;
  both are verified through other routes and recorded.
- **Start order for capture**: consent → `mediaProjection` foreground service →
  `getMediaProjection`, and `registerCallback` before `createVirtualDisplay` on Android 14+.
- **An unfocused split-screen pane emits no accessibility events at all.** A scenario cannot
  provoke a scan of it by waiting.
- **The device-admin list rows need `android:isAccessibilityTool="true"`**, or the guard is
  handed chrome and no rows.
- **Plain Device Admin only.** Never script around Device Owner.
- **Recents is blocked on real One UI/HyperOS hardware**; the swipe-kill cannot be reproduced
  on an emulator, so no scenario can verify a mitigation.
- **ONNX Runtime has no prebuilt `x86_64-linux-android` or `armv7` runtime**, so an image-path
  scenario on an x86 emulator image is not possible; use arm64.

## Other platforms

Windows and Linux are expected to be driven from UTM VMs on this machine later. Their
contracts are not yet written and nothing is decided for them. The mechanism that keeps this
honest is a `verify_host` field on steps, so a step that cannot be verified on the available
host reads as "cannot verify here" instead of passing silently. Adding that field is deferred
until the VM contracts exist.

## Rejected

- **A hand-maintained demo script per feature.** Rots, as above.
- **Owner sign-off at every observation step.** Reintroduces babysitting, and it is why
  finished work strands in unmerged branches.
- **Running e2e in CI now.** The setup cost exceeds the benefit while no runner has a GUI
  session or the grants.

## What this does not cover

- Building any of it. See `docs/engineering/plan.md` steps 9–11.
- Performance and load behaviour; scenarios assert reachability and correctness only.
- Visual regression. A pixel-comparison harness is a different problem and is not planned.
