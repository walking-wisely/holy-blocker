# macOS Daemon — backlog

Work that is known, scoped, and deliberately not done yet. Items leave this file by being struck
through with the evidence that closed them, or by moving into [plan.md](plan.md) when they become
the next step rather than a deferred one.

The threat model is the one in [content-interception.md](../../decisions/content-interception.md):
the adversary is the machine's own user, and unlike Android they have a terminal. The bar is
therefore **no bypass route that is both easy and silent**, not prevention.

## Open

### 1. `PermissionGate` has never seen a real grant or a real revocation

**Half-unblocked by module 0.** The pure half is built and tested, the four environment signals are
confirmed live, and the bundle and LaunchAgent now exist — the agent bootstraps into `gui/501`,
runs, and takes a permission baseline under launchd rather than under a terminal, which removes the
responsible-process problem. What remains untested is the part that matters most: `poll()` reporting
`permissionLost` when a held capability is actually taken away. It has only ever run against a fake
probe, because nothing has ever *held* a real grant.

Two things stand between here and that:

- **A stable signing identity.** The bundle signs ad-hoc by default, and an ad-hoc identity is
  derived from the `cdhash`, so a grant made against it dies on the next `swift build`. This is a
  decision — Developer ID, or a self-signed code-signing certificate reused consistently — and
  creating one writes to a keychain. `scripts/bundle.sh` already honours
  `HOLY_BLOCKER_SIGNING_IDENTITY`; nothing else is needed in code.
- **A human.** A Screen Recording grant requires clicking the prompt or the System Settings
  toggle. No amount of daemon code can produce one.

**What closes it:** sign with a stable identity, install the agent, grant Screen Recording, then
revoke it through *both* System Settings and `tccutil reset ScreenCapture com.holyblocker.daemon`.
The whole claim of watching outcomes rather than routes is that one poll catches both, and that
claim is still untested.

### 2. Can a *standard* user reset a Screen Recording grant?

**This blocks any tamper-resistance claim, and it is minutes of work on the right machine.**

Measured so far: `tccutil reset ScreenCapture <bundle-id>` runs **without `sudo`** and fails only
at bundle-ID resolution (`OSStatus -10814`). But it was run from an **admin** account, because the
development machine's only user is in the `admin` group. Screen Recording lives in the system-wide
TCC store rather than the per-user one, so it plausibly fails for a standard user — and
"plausibly" is carrying the entire account model.

**What closes it:** a real standard-user account on a Mac, and the same command run from it. If a
standard user *can* run it, the standard-user configuration is worth much less than the model
assumes and the plan needs revisiting rather than patching.

### 3. `ProxyConfiguration.restore()` is not atomic per service

Carried over from Layer 1. Each service is restored with two `networksetup` calls, so an
interruption between them can leave a proxy enabled with an empty server — a state that breaks
browsing rather than merely failing to filter it. The persisted snapshot self-heals it on the next
`proxy-restore`, so nothing is permanently lost, but the window exists.

Low priority: the interruption has to land between two adjacent subprocess calls, and the recovery
is automatic and already implemented.

## Tracked elsewhere, listed here so it is not lost

- **What to do when the protected user is a local admin.** A product decision, not an engineering
  one, and it blocks writing onboarding. `PermissionGate.assess()` already makes the two
  configurations impossible to confuse — a self-admin user reports `weakened`, never `protected` —
  which is the floor. The decision itself belongs in
  [content-interception.md](../../decisions/content-interception.md), where it is listed as an open
  question.
- **`mitm-proxy`'s plain-HTTP path takes no `ScanHooks`**, so URL, body and image scanning apply
  only to CONNECT-tunnelled traffic. That is a `packages/mitm-proxy` gap; it is noted here only
  because the macOS proxy configuration routes both.
