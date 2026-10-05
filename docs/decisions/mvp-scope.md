# ADR: MVP scope and the milestone label

| Field | Value |
|---|---|
| **Status** | Accepted, amended 2026-10-05 with the owner's answers; one open question remains |
| **Date** | 2026-10-05 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

Tier: product decision, owner-decided. It touches the tamper model, so the red-team is tier 2
(three lenses) under [decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md); the
findings are recorded below.

---

## Context

The repository has no definition of "first shippable". Ordering is computed from `depends_on` and
an optional `group` ([ledger-order-and-derived-status.md](ledger-order-and-derived-status.md)), so
`ledger next` and `todos` offer whatever is ready, not whatever the product needs first. Measured
2026-10-05 from [coverage.md](../engineering/coverage.md): its cross-cutting row "Can a user install
this today and have anything blocked end to end?" reads **No**, while work with no bearing on a first
build (the Windows daemon, SNI/IP filtering, the image classifier) sits in the same queue as work
that does.

## Decision

1. **The MVP is four requirements, on Android and macOS only.**

   | # | Requirement | Meaning |
   |---|---|---|
   | 1 | Block adult content | Text on screen, and domains through DNS (Android) or the HTTPS proxy (macOS). Images are not classified |
   | 2 | Block user-chosen apps | A person names an application and it is blocked regardless of content |
   | 3 | Not deletable | Uninstall and disable are refused while protection is armed; what cannot be prevented is recorded in the tamper log. This is refuse-and-log, with no voice gate and no partner notification |
   | 4 | Developer escape hatch | A separate development build that can turn protection off |

   Any release wording says **text and domains only**. The mission names imagery served from a clean
   CDN as the case a domain block cannot see ([mission.md](../mission.md)); the MVP does not answer
   it, and a "Yes" on the coverage row is not a claim of full protection. Requirement 2 is in and
   imagery is out because per-app blocking is deterministic and observable, while the image path has
   no calibrated threshold ([classifier-operating-point.md](classifier-operating-point.md)).
   Requirement 3 relies on solo mode ([accountability.md](accountability.md)) as the shipped
   configuration: there is no external witness at the moment of removal, and that is accepted for
   the MVP. It guards against casual and impulsive removal, not against someone who can build the
   repository.

2. **Out of the MVP**, by name: explicit imagery (the classifier head, the LiteRT backbone, the
   macOS image scanner's first real frame), SNI/IP filtering, DoH and DoT coverage, Windows, iOS,
   the desktop control panel, usage nudges, the voice gate and partner notification, a liveness
   signal ("know that protection is actually running"), and a false-positive recourse beyond what
   exists today. The MVP runs protection mode `full` only; a false positive has no in-product
   remedy yet, and that cost is accepted and recorded here rather than left unmentioned. They keep their steps and their place in
   [outcomes.md](../product/outcomes.md); they are not MVP work.
3. **A step carries `milestone = "mvp"`** in its step file when the MVP needs it. The value set is
   closed (`mvp` only) and validated. Ready milestone steps sort before every other ready step,
   within a component by `ledger next` and across components in `todos`, which also takes
   `--milestone mvp`. Bug steps sort before milestone steps, so a defect or a security fix is never
   queued behind new MVP work. The label is added in the PR that creates or amends a step file and
   is not otherwise gated; `loop check`'s note below is an aid, not a control. Dependencies still come first: a milestone step may not wait on an
   unfinished step outside the milestone, and `ledger validate` fails when it does. This amends
   the ADR above, where `group` stayed presentation and no label influenced order; the amendment
   is bounded to this one label and `depends_on` remains the only thing that blocks.
4. **`loop check` notes, never fails.** When a milestone step is still pending and a PR names a
   `feature` step that is not in the milestone, `loop check` prints a `note:` line. Bug steps are
   never noted. A manifest that cannot be read produces its own `note:` instead of being skipped
   silently.
5. **Done means the ledger says so.** The MVP is complete when the cross-cutting coverage row
   "Can a user install this today and have anything blocked end to end?" reads **Yes** on both
   platforms, observed under [e2e-scenario-contract.md](e2e-scenario-contract.md). A step being
   `done` does not move that row. On macOS the Yes is conditional on a standard account and says
   so; it is never given for an admin account. Only the owner promotes the row, and only the owner
   demotes it when a later change regresses what it rests on; no tool reads `coverage.md`.
6. **The development build is a separate artifact, not a runtime flag.**
   - It has its own application id (Android) and bundle id (macOS), so it never shares an install,
     a TCC grant, a Device Admin enrolment or a keychain item with the release build.
   - The bypass is compiled in only by the development configuration. The release configuration
     contains neither the code nor the setting that reaches it.
   - CI asserts the absence in two ways: release artifacts are scanned for the development build's
     identifier and the bypass entry point, and a behavioural check confirms the release build
     refuses the development path's off command. The release artifact's signing certificate must
     differ from the development build's. A string scan alone proves only that one name is absent,
     so the check is a step (`engineering.dev-build-release-guard`), and the coverage row stays
     Unverified until it runs. Scan output names the identifier, never artifact contents.
   - The development build is never published, signed with the release identity, or given the
     release build's uninstall protection. It is never installed on a device the person it
     protects uses.
   - The bypass acts on the development build's own process only. It never touches the release
     build's VPN or DNS slot, Device Admin enrolment, accessibility grant or daemon. The release
     build records the presence of a development-build sibling as a content-free tamper event.
   - The bypass is reachable only by a local, on-device action. The development build has no
     network listener, remote configuration, exported component, deep link or push command, and it
     logs nothing the release build does not.
   - Protection mode `off` is unchanged: it stays a user-facing mode behind the override gate in
     [protection-modes.md](protection-modes.md). The development bypass is not that mode and does
     not weaken it.

## Owner answers, 2026-10-05

1. **macOS runs in a standard account.** Changing a LaunchDaemon needs sudo, which is accepted as the
   removal barrier. The agent is the open part: it runs as the protected user to hold the
   Accessibility grant, so whether that user can stop it is observed by
   `mac-daemon.standard-user-tamper` before any wording ships. A protected user who is a local admin
   stays out of scope, and a "Yes" is never given for an admin account.
2. **Turning protection down is the same on every platform.** Request, cooldown, confirm, as
   `apps/mobile` does today; `mac-daemon.protection-schedule` brings macOS to parity. Requirement 3
   therefore reads: removal is refused or delayed, and what cannot be prevented is logged. The
   development build remains the only instant off.
3. **Custom-app blocking has per-app modes and a searchable list panel on both platforms.** Block
   terminates the app where the platform allows it, hide covers the app with the overlay, unblocked
   does nothing. Android cannot terminate another app under plain Device Admin (unverified), so
   there block sends the user home and covers the app. The macOS panel is native in the agent, not
   the Electron control panel, which stays out of the MVP. Lowering a mode follows the disarm
   cooldown. The remaining design goes through `plan-inception` in the two custom-app steps.
4. **The MVP is working builds on the owner's own devices, not store releases.** Release builds are
   signed with a local keystore (Android) and a local identity (macOS), both different from the
   development build's.
5. **The domain list is bundled in each build.** Updates come from a new build. Fetching updates from
   the project's own source is permitted and is a later step that passes through `plan-inception`
   first: it adds a capability and a trust boundary, and no host exists yet.

## Open question

**A protected user who is a local admin on macOS.** [content-interception.md](content-interception.md)
leaves this open and `assess()` reports `weakened` without refusing to run. Against an admin, a
standard-user `tccutil` result does not apply, so the MVP's "not deletable" claim on macOS stays
scoped to a standard account.

## Build order across components

`depends_on` binds within one component only, so these edges are recorded here and in step titles.

| Step | Needs first |
|---|---|
| `mobile.blocklist-provisioning` | `net-shield.ffi-artifact-load` |
| `mac-daemon.proxy-blocklist-wiring` | `mitm-proxy.domain-blocklist` |
| `mobile.e2e-android-scenario`, `mac-daemon.e2e-macos-scenario` | `engineering.e2e-contract`, `engineering.e2e-runner` |
| `mac-daemon.install` and the two macOS tamper observations | `engineering.privilege-dispatcher` |
| `mobile.custom-app-domains` | `net-shield.keyword-rule` |
| `mac-daemon.custom-app-domains` | `mitm-proxy.keyword-rule`, which follows `mitm-proxy.domain-blocklist` |
| `engineering.dev-build-release-guard` | `mobile.dev-kill-switch`, `mobile.release-signing` |
| `engineering.dev-build-release-guard-macos` | `mac-daemon.dev-kill-switch`, `mac-daemon.release-identity` |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| A runtime developer flag in the release build | A flag in the shipped artifact is a flag the protected user can reach. Compile-time exclusion is the only form CI can prove absent |
| One MVP checklist file kept in sync by hand | A second source of truth beside the ledger; it goes stale the way `at-pr` did |
| Use `group` for the MVP | `group` is presentation only, per the ADR it extends; overloading it would make ordering depend on a label the validator treats as cosmetic |
| Block, rather than note, an off-milestone PR | A security fix or a defect found mid-MVP would be refused by CI; the owner does not review code, so a hard gate has no human to override it |
| A free-form `milestone` value | Typos silently drop a step out of the queue. A closed set fails at `validate` |

## Constraints on the custom-app steps

The app list a person chooses and any observed foreground package can reveal sexuality, religion or
health, and they are a usage history. They are not rows in the privacy inventory yet. Before the
first custom-app step merges: the list and any observation stay on the device; observations stay in
memory and are bounded; a blocked-app event records no package name in the tamper log, whose
vocabulary is content-free by test; and a row for these data classes is added to
`.claude/skills/privacy-review/references/data-classes.md`. The capture paths that already exist
(`ScreenCaptureService`, `ScreenCapture.swift`) hold frames the inventory treats as sensitive; the
MVP build states whether they are dormant, and the release wording is "images are not classified"
whichever it is. The on-device confidentiality claim on macOS carries the same standard-account
scope as requirement 3.

## What this does not cover

- It does not pick the design for custom-app blocking. The steps `mobile.custom-app-blocking` and
  `mac-daemon.custom-app-blocking` begin with `plan-inception`; per-app detection differs by
  platform (AccessibilityService foreground package, `NSWorkspace`) and neither exists today.
- It does not make the development build safe to lose. A development build left installed on a
  protected person's device is a bypass; the decision constrains what ships, not what is sideloaded.
- It does not define an MVP for Windows or iOS.
- It does not make "not deletable" hold against a build-capable attacker: the repository is public
  and the development configuration is a build variant anyone can compile and sideload. Only a
  device held by someone else, or a signature policy, limits that.
- `milestone` ordering is advisory across components: `todos` sorts, but nothing stops an agent
  from picking a non-milestone step by hand.

## Red-team findings

Three fresh-context runs, 2026-10-05, all by reading; none executed the CI job, which does not
exist yet. Folded findings are listed; the rest are recorded in "What this does not cover".

| # | Lens | Finding | Resolved by |
|---|---|---|---|
| 1 | Bypass | A string scan proves one name is absent, not the capability; no CI job is named (**changed the design**) | decision 6, behavioural check and a step |
| 2 | Bypass | A development build installed beside the release build is a silent bypass if it can act on shared state | decision 6, own-process-only rule and sibling-presence tamper event |
| 3 | Bypass | A public repository lets anyone build the development configuration | requirement 3 scoped to casual removal; "What this does not cover" |
| 4 | Bypass | "Never signed with the release identity" is unenforced; matching signatures allow update-over-install | decision 6, certificate difference asserted |
| 5 | Bypass | Milestone label reorders ahead of a security fix (**changed the code**) | decision 3, bug steps sort first |
| 6 | Bypass | A manifest that fails to parse silently drops out of the note (**changed the code**) | decision 4, unreadable manifest noted |
| 7 | Bypass | macOS requirement 3 is empty for a local admin, and a Yes would hide it | decision 5, conditional Yes |
| 8 | Bypass | Nothing demotes the done row | decision 5, owner demotes |
| 9 | Intent | "Block adult content" hides that images are excluded | decision 1, wording and rationale |
| 10 | Intent | Not-deletable ships without the deliberate pause the mission calls the feature | requirement 3, refuse-and-log stated |
| 11 | Intent | The partner flow is absent without saying solo mode is the shipped configuration | decision 1, solo mode named |
| 12 | Intent | App blocking is promoted over imagery with no stated reason | decision 1, rationale |
| 13 | Intent | No liveness signal and no false-positive recourse in the MVP | decision 2, listed as out with the cost accepted |
| 14 | Legal and privacy | App list and foreground package are an unnamed data class; blocked-app events could pollute the tamper log | "Constraints on the custom-app steps" |
| 15 | Legal and privacy | The development build has no stated data or command boundary | decision 6, local-only bypass and no extra logging |
| 16 | Legal and privacy | Existing capture code holds frames the inventory treats as sensitive | "Constraints on the custom-app steps" |
