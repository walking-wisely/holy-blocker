# Review triage policy

After a review finds something, what happens? This page answers that. `step-loop`
gate 5 used to say only "if the review finds a load-bearing gap, go back to gate 1",
which leaves every non-blocking finding and every judgment call unspecified. This is
the specification those words pointed at: severity tiers, a fix-attempt cap, a routing
table, and the escalation packet the loop quotes instead of composing.

Severity is ranked by consequence, on the same axis `adversarial-review` already uses
("a guard that fails open silently outranks a crash" — `.claude/skills/adversarial-review/SKILL.md`):
what a finding does to the product if left unaddressed, never how much was written or
how many tests it took to find. A finding's tier determines the rest of this page.

## 1. Severity tiers

Mapped onto `adversarial-review`'s consequence axis. Three tiers, one concrete example
from this repo per tier.

| Tier | Meaning | Example from this repo |
|---|---|---|
| **Blocking** | The module's stated purpose is not actually true, a trust boundary breaks, a guard fails open. | A DNS shield that answers a blocked domain with the real address under some code path: `coverage.md`'s live defect "A forged DNS answer from an off-path attacker" — `NetworkGuardService.ask()` writes an unvalidated upstream answer into the TUN (`apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/NetworkGuardService.kt:299`), so a blocked name can be answered with the real address. A guard that fails open. |
| **Non-blocking** | A real bug, off the current diff's critical path. | An off-by-one in a log formatter: the window-rect line in `win-daemon`'s `Log()` (`native-modules/win-daemon/src/main.cpp:42`). A bounds misprint changes what the console shows and nothing about whether the daemon acts on the event. |
| **Judgment call** | The right answer depends on risk tolerance or product taste, not on what is true. | Whether a permission-check that finds a weakness should fail open or fail closed: `PermissionGate.assess()` returns `.weakened` and the daemon runs anyway (`native-modules/mac-daemon/Sources/MacDaemon/PermissionGate.swift:411`); `docs/decisions/content-interception.md` records it as an open product decision. |

## 2. Fix-attempt cap

Exactly one fix attempt is made per Blocking finding after it is first reported, and the
result is re-verified against the review's own reproduction command. If it does not
clear on that second attempt, stop — do not attempt a third time. This is a hard number,
not a guideline:

```
MAX_FIX_ATTEMPTS = 1
```

One attempt exists to answer the finding; the second run verifies the fix. Anything that
still does not clear is either a mis-framed finding or a problem the first attempt did
not understand — both are re-triage work (regrade, or escalate), not more fixing. The
loop's own bias is to make the review go away; the cap is the guard against that bias.

## 3. Routing table

| Tier | Route |
|---|---|
| **Blocking** | Fix within the cap, then re-review. The re-review is the same fresh-context review re-run against the fix. Nothing with an open Blocking finding merges. |
| **Non-blocking** | Filed as a `kind = "bug"` step in the package's `steps.toml`, with `regressed_step` set when the finding is a regression against an already-`done` step. Never fixed inline unless it is a same-file, same-test-suite change to the current diff. |
| **Judgment call** | Never resolved by the loop. Always escalated — the escalation packet below, verbatim. |

## 4. Escalation packet format

Quote this verbatim; compose nothing freehand. The number-carrying first line makes the
shape of the review legible at a glance:

> N findings. M auto-fixed and reverified. K stopped at the fix-attempt cap.
> J are judgment calls. [K and J are listed below, one line each: what's
> contested, and why it can't be resolved without your call.]

The packet never lists the M that were cleanly fixed and reverified.

## 5. Living log

Every step-loop run that escalates something appends a row here. The columns hold the
triage verdict and its hindsight verdict; this is how the auto-fix/escalate boundary
tightens from observed outcomes instead of staying a fixed guess.

| date | finding | routed-as | should-have-been | why |
|---|---|---|---|---|