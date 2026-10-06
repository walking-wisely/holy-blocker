# Review triage policy

After a review finds something, what happens? This page answers that. `step-loop`
gate 5 used to say only "if the review finds a load-bearing gap, go back to gate 1",
which leaves every non-blocking finding and every judgment call unspecified. This is
the specification those words pointed at: severity tiers, a fix-attempt cap, a routing
table, and the escalation packet format (§4) that `step-loop` gate 4 quotes
so it never composes one freehand.

Severity is ranked by consequence, on the same axis `adversarial-review` already uses
("a guard that fails open silently outranks a crash" — `.claude/skills/adversarial-review/SKILL.md`):
what a finding does to the product if left unaddressed, never how much was written or
how many tests it took to find. A finding's tier determines the rest of this page.

## 1. Severity tiers

Mapped onto `adversarial-review`'s consequence axis. Three tiers, one concrete example
from this repo per tier.

| Tier | Meaning | Example from this repo |
|---|---|---|
| **Blocking** | The module's stated purpose is not actually true, a trust boundary breaks, a guard fails open. | A DNS shield that trusts an unvalidated upstream answer on a forwarded query: `coverage.md`'s live defect "A forged DNS answer from an off-path attacker" — `NetworkGuardService.ask()` accepts a response from an unconnected socket with no transaction-ID or question check and writes it into the TUN (`apps/mobile/app/src/main/kotlin/com/holyblocker/mobile/NetworkGuardService.kt:299`), so an off-path attacker's forged answer is believed and the client is sent where the attacker says. A guard that fails open silently. |
| **Non-blocking** | A real bug, off the current diff's critical path. | An off-by-one in a log formatter: the window-rect line in `win-daemon`'s `Log()` (`native-modules/win-daemon/src/main.cpp:42`) computes the size from `right - left`/`bottom - top` — if that were off by one it would change what the console prints and nothing about whether the daemon acts on the event. |
| **Judgment call** | The right answer depends on risk tolerance or product taste, not on what is true. | Whether a permission-check that finds a weakness should fail open or fail closed: `PermissionGate.assess()` returns `.weakened` and the daemon runs anyway (`native-modules/mac-daemon/Sources/MacDaemon/PermissionGate.swift:411`); `docs/decisions/content-interception.md` records it as an open product decision. |

## 2. Fix-attempt cap

Exactly one fix attempt is made per Blocking finding after it is first reported, and the
result is re-verified against the review's own reproduction command. If it does not
clear on that second attempt, stop — do not attempt a third time. This is a hard number,
not a guideline:

```
MAX_FIX_ATTEMPTS = 1
```

This document is the contract; `step-loop` gate 4 names the constant and refuses a third
attempt.

One attempt exists to answer the finding; the second run verifies the fix. Anything that
still does not clear is either a mis-framed finding or a problem the first attempt did
not understand — both are re-triage work (regrade, or escalate), not more fixing. The
loop's own bias is to make the review go away; the cap is the guard against that bias.

## 3. Routing table

| Tier | Route |
|---|---|
| **Blocking** | Fix within the cap, then re-review. The re-review is the same fresh-context review re-run against the fix. Nothing with an open Blocking finding merges. |
| **Non-blocking** | Filed as a `kind = "bug"` step in the package's `steps/` directory, with `regressed_step` set when the finding is a regression against an already-`done` step. Never fixed inline unless it is a same-file, same-test-suite change to the current diff. |
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
| 2026-10-02 | `edited` trigger re-runs all of CI on every PR title or body edit | escalated | pending | cost versus a separate light workflow is a product call |
| 2026-10-02 | `loop-contract` runs the checker from the PR head, so a PR can edit its own gate; CODEOWNERS coverage of `tools/plan/**`, `docs/**/steps.toml`, `.github/**` unconfirmed | escalated | pending | guardrail or enforcement is a risk-tolerance call |
| 2026-10-02 | steps 8 and 9 marked done on gates that are only prose in `SKILL.md`, with no e2e runner | escalated | pending | whether `acceptance = "code"` is the right claim for documented-only capability |
| 2026-10-02 | Dependabot PRs skip the contract check even on governed paths | escalated | pending | intentional skip versus coverage is a risk-tolerance call |
| 2026-10-02 | `loop check` does not run the stem-equals-id and legacy-beside-new checks that `ledger validate` runs, so a PR's gate and the ledger CI job can disagree | escalated | pending | whether the contract check should enforce layout itself or rely on `plan-ledger` is an enforcement-scope call |
| 2026-10-02 | `ledger migrate` puts every leading comment, including one written for the first step, into `package.toml` and drops comments between later steps | escalated | pending | no real manifest is affected; whether per-step comments are worth preserving is a convention call |
| 2026-10-05 | `mac-daemon.proxy-blocklist-wiring`: trust anchor (`keys/`) sits beside the list in the bundle, so the signature protects only if the installed bundle is root-owned; rollback to an older validly signed list is not shown to be refused by the proxy | escalated | pending | whether bundle ownership alone is the accepted baseline, and whether a monotonic version is required, are risk-tolerance calls |
| 2026-10-06 | privilege dispatcher: the signing pin does not separate the agent from root, because the signing key is usable by the same user that runs the agent | escalated | as routed | accepted: the dev machine is not a client machine and is treated as secure; the ADR records the limit |
| 2026-10-06 | privilege dispatcher: `install.sh` runs as root from a user-writable checkout, so files edited after review are what gets installed | escalated | as routed | accepted: the owner compares the printed hashes at install time |
| 2026-10-06 | privilege dispatcher: audit log records full home paths of the arguments, not only file names | escalated | as routed | accepted: full paths stay, the log is local, mode 0600 and never committed |
| 2026-10-06 | `net-shield.keyword-rule`: decision 12 puts the matcher in `net-shield` over UniFFI while decision 15 and its rejected-alternatives row say Kotlin and Swift ports against shared vectors | escalated | as routed | owner 2026-10-06: decision 12 stands, the matcher stays in `net-shield`; decision 15 amended |
| 2026-10-06 | `net-shield.keyword-rule`: the Exact/Contains confidence never leaves the core, so the panel cannot show what each token matches | escalated | as routed | owner 2026-10-06: expose it now; `match_keyword_token` in the FFI |
| 2026-10-06 | `net-shield.keyword-rule`: private suffixes such as `github.io` make a user's own subdomain the Exact label, so a token can block someone's Pages site | escalated | as routed | owner 2026-10-06: not accepted; a name under a private suffix never matches |
| 2026-10-06 | `net-shield.keyword-rule`: `KeywordRules` derives `Debug`, so a future `{:?}` would print confirmed app tokens | escalated | as routed | owner 2026-10-06: redact; `Debug` prints a token count only |
| 2026-10-06 | `mitm-proxy.keyword-rule`: keyword refusal ignores `ProtectionMode`, as the list check does, though keywords are the heuristic rule that can false-positive | escalated | as routed | open: owner to decide whether keywords follow the mode gate |
