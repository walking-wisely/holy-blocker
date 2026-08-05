# macOS Daemon — backlog

Work that is known, scoped, and deliberately not done yet. Items leave this file by being struck
through with the evidence that closed them, or by moving into [plan.md](plan.md) when they become
the next step rather than a deferred one.

The threat model is the one in [content-interception.md](../../decisions/content-interception.md):
the adversary is the machine's own user, and unlike Android they have a terminal. The bar is
therefore **no bypass route that is both easy and silent**, not prevention.

## Open

### 1. `PermissionGate` has never seen a real *revocation*

**The grant half is closed.** In the first live e2e pass the agent held real Screen Recording and
Accessibility grants under `launchd`, keyed to the `Holy Blocker Dev` certificate, and reported them
itself (`ax grant: granted`, `assess()` → `weakened`, the remaining weakness being
`protectedUserIsAdministrator` — this machine's known configuration). The grants survived four
bundle replacements, which is exactly what the stable identity exists for.

What remains untested is the part that matters most: `poll()` reporting `permissionLost` when a held
capability is actually taken away. It has still only ever run against a fake probe. Now that a real
grant exists this is minutes of work and needs no new code — `tccutil reset ScreenCapture
com.holyblocker.daemon` was confirmed during the pass to resolve and run unprivileged, so both
revocation routes can be exercised against a live agent and the log line will show the transition.

One thing stood between here and the grant half, and it is closed:

- ~~**A stable signing identity.** The bundle signs ad-hoc by default, and an ad-hoc identity is
  derived from the `cdhash`, so a grant made against it dies on the next `swift build`. This is a
  decision — Developer ID, or a self-signed code-signing certificate reused consistently — and
  creating one writes to a keychain. `scripts/bundle.sh` already honours
  `HOLY_BLOCKER_SIGNING_IDENTITY`; nothing else is needed in code.~~ **Done.** A self-signed
  `Holy Blocker Dev` identity lives in the development machine's login keychain;
  `scripts/create-dev-signing-identity.sh` recreates or rotates it. See
  [signing-identity.md](signing-identity.md).

**What closes it:** with the agent running and granted, revoke through *both* System Settings and
`tccutil reset ScreenCapture com.holyblocker.daemon`, and confirm each is reported. The whole claim
of watching outcomes rather than routes is that one poll catches both, and that claim is still
untested.

### 2. Mission Control composites live window previews above the overlay

**Found in the first live e2e pass; filed rather than fixed, by decision.** A four-finger swipe up
puts Mission Control in front, and it is composited by the Dock above ordinary window levels —
including `.screenSaver`, which is the highest level `OverlayPlan` has. Its previews are live, so
the content is legible in the thumbnails with the interstitial pushed underneath. App Exposé, the
Command-Tab switcher's window previews, and Stage Manager's strip are the same family.

Nothing in the daemon knows the gesture exists: `EventHooks` (module 11) is not built, so there is
no monitor to notice it either.

Partly mitigated as of the live pass: a block now hides the offending application
(`WindowSuppression`), and a hidden application has no windows for Mission Control to preview. That
covers content the daemon has *seen* — it does nothing for content it never scanned, which is
backlog item 3 below.

Two directions when this is picked up, both explicitly not chosen yet:

- **Detect and report**, treating an activation while blocking content is on screen as a tamper
  event, consistent with the "watch outcomes, not routes" rule the permission gate already follows.
  Needs a measurement first: whether `NSWorkspace.activeSpaceDidChangeNotification` or a
  `CGWindowList` level scan actually fires for Mission Control, neither of which is documented for
  this purpose.
- **Disable the gesture**, via the `mcx-expose-disabled` managed preference in `com.apple.dock`.
  Prevention rather than detection, but it is a system-wide change to someone's machine and would
  need the same snapshot/restore discipline as `ProxyConfiguration`.

**What closes it:** re-measure what Mission Control still exposes *after* per-window scanning and
suppression are in place, then decide. It may turn out there is nothing left to reveal.

### 3. Only the focused window is ever scanned, so on-screen content escapes by losing focus

**Found in the first live e2e pass, and it is the largest coverage gap on this platform.**
Confirmed on macOS 26.5: with blocking text in a TextEdit window the interstitial goes up; click
another application — or just click the desktop, which unfocuses everything at once — and the
verdict flips to `allow` and **the overlay tears down while the text is still fully visible on
screen**. That is worse than a miss — it reads to the user as "cleared".

**Partly mitigated, and the remaining half is the dangerous one.** A block now hides the offending
application rather than only covering it (`WindowSuppression`), so content that has been *seen*
cannot be revealed by unfocusing, by clicking the desktop, or through Mission Control — there is no
window left. What is untouched is content that is never scanned in the first place: a second window
beside the focused one, a video playing next to a chat, anything on a second display. That content
never produces a verdict at all, so nothing is ever covered *or* hidden.

The cause is structural rather than a bug. `SystemAXProbe.focusedRoot()` reads
`NSWorkspace.frontmostApplication` and then that application's `kAXFocusedWindowAttribute`; nothing
else on screen is walked. Side-by-side windows, a video beside a chat, and anything on a second
display are all invisible to the text path. Note the exact mirror of the Android split-screen
finding in the root `AGENTS.md`: there, *every* watched window is evaluated, precisely because an
unfocused pane emits no accessibility events at all.

Two directions, and they are not alternatives:

- **Walk every on-screen window, not just the focused one.** `AXWindows` on each
  `NSWorkspace.runningApplications` entry with `.regular` activation policy, worst-verdict-wins.
  Cost is the open question: `SystemAXProbe` budgets 0.5s per walk and that budget is per
  *application*, so the 1s scan cadence cannot absorb a dozen of them. Needs a shared per-tick
  budget, skipping hidden/minimised applications, and probably a cheap change check before
  re-walking an application whose windows have not moved.
- **OCR the captured frame**, which is focus-independent by construction and already has a cadence
  slot in `ScanLoop` reserved for it. **There is no OCR module in this plan at all** — the Layer 2
  module list goes capture → scanner → overlay with nothing between, even though `ScanLoop` splits
  an image cadence from an "OCR" one and the Windows daemon plan has the module. That is the real
  long-term answer for unfocused content and it is currently unplanned work.

**What closes it:** the AX half is a bounded change and worth doing first, since it needs no new
module and no model. Closing it *fully* needs the OCR module to exist, which should be specified
before the image classifier is bound to a frame.

### 4. Can a *standard* user reset a Screen Recording grant?

**This blocks any tamper-resistance claim, and it is minutes of work on the right machine.**

Measured so far: `tccutil reset ScreenCapture <bundle-id>` runs **without `sudo`** and fails only
at bundle-ID resolution (`OSStatus -10814`). But it was run from an **admin** account, because the
development machine's only user is in the `admin` group. Screen Recording lives in the system-wide
TCC store rather than the per-user one, so it plausibly fails for a standard user — and
"plausibly" is carrying the entire account model.

**What closes it:** a real standard-user account on a Mac, and the same command run from it. If a
standard user *can* run it, the standard-user configuration is worth much less than the model
assumes and the plan needs revisiting rather than patching.

### 5. Safari's page body has not been confirmed through `AccessibilityText`

Module 12 is verified against Chrome 151 — the full page content of a local test page came back,
including image `alt` text. Safari's *window* reads fine too (45 nodes of real content), but its
**page body** was never confirmed: every attempt to put the test page in front of Safari lost focus
back to the terminal, and reading Safari by bundle ID instead returned an empty `AXWindows` because
its windows were on another Space.

This is a coverage question, not a code one — Chrome already proves the web-content path works, and
Safari needs no opt-in of any kind.

**What closes it:** open a page with known text in Safari, leave it frontmost, and run
`holy-blocker-macd ax-text 5` from a shell. Confirm the page's body text appears, not just the
title and toolbar. Minutes of work, and it needs a human only because focus does.

### 6. A lexicon phrase can match across two unrelated AX elements

`AccessibilityText` joins elements with a newline, and that separator is not a boundary — no
character could be. `packages/text-policy`'s `collapse_whitespace` rewrites a newline to a space and
its `compact` pipeline strips it, so a sidebar label ending in one word and an unrelated heading
beginning with the next read to the scorer as the phrase. `AccessibilityScanner` scores the walk as
one blob and inherits this.

The fix is to evaluate elements separately and take the worst verdict, which means widening
`AccessibilityText.extract` to return the lines rather than one joined `String`. It was **not** done
in session 4 because it trades one error direction for another: text legitimately wrapped across two
AX elements — which is ordinary in a paragraph, a table cell, or a chat bubble — would stop matching,
turning a false-positive class into a false-negative one. Which is worse depends on how real
applications actually split their text, and nothing in this repo measures that.

**What closes it:** a corpus of real AX walks (the `ax-text` verb already produces them) scored both
ways, showing which direction costs more. Until then the per-element split is a guess with a
plausible story, which is exactly what the joined blob already is.

### 7. `ProxyConfiguration.restore()` is not atomic per service

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
