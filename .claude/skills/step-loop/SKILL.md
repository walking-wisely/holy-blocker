---
name: step-loop
description: >
  Run one ledger step end to end: pick the next pending step, audit its external
  claims, implement test-first in a worktree, verify, adversarially review in a
  fresh context, update docs, and take it to a PR or a merge according to the
  step's acceptance kind. Use when asked to "run the loop", "do the next step",
  "work the plan", "take a step to PR", or to continue a package's plan without
  re-deriving what is next. This is the repository's execution pipeline; the
  per-package plan.md files are prose it reads, not the state it runs on.
---

# Step loop

One unit of work, five gates, in order. The loop exists because each gate was
already a skill or a convention that a session had to remember, and remembering
is what drops when context resets.

State is **derived, never stored**: a step's status is `steps.toml`, and whether
its branch landed is git and `gh` — never a file the loop writes. A written
status goes stale and forks per branch.

Which work enters this loop at all is decided by `CLAUDE.md`, "Routing — which loop applies".

## 0. Pick the step

```
python -m tools.plan.ledger validate docs/components/<package>
python -m tools.plan.ledger next     docs/components/<package> ...
```

Take the reported step. If `next` reports none, stop — do not invent work.
Note the step's `acceptance` and `verify` from the manifest; both drive gates
below. If the user named a step, use it and check it is the actionable one.

## 1. Audit the external claims — before any code

Run the `assumption-audit` skill over the step's plan section. Enumerate the
three to seven claims about the outside world the step would collapse without,
attach one falsifying command to each, and run them.

**Stop and report** if a load-bearing claim is false: the module's purpose, not
its details, is at stake. Unverifiable-here is a legitimate outcome and must be
recorded as such, never promoted to verified.

## 2. Worktree and branch

```
git worktree add .claude/worktrees/<slug> -b <prefix>/<slug> master
```

Base is `master` unless the step genuinely builds on an unmerged branch; if that
is ambiguous, ask. Run every command from the worktree. Announce the branch —
it is the loop's handle for every gate after this.

## 2a. Context-budget gate — before any implementation

```
python -m tools.plan.context
```

Exit 0 means under 100,000 tokens: continue. Exit 1 means over: **stop, do not implement**,
and write a handoff brief instead, then tell the owner to continue in a fresh session:

- step id
- branch
- worktree path
- next action (the first gate not yet done, in one line)

Print the brief to the terminal or write it to the session scratchpad. Never commit it, and
never include transcript content.

Skip the gate only when the step carries `difficulty = "hard"` in `steps.toml` (`ledger next`
prints it). Exit 2 means no transcript could be read; say so and continue.

## 3. Implement, test-first

Write focused unit tests for new logic before the implementation, per the
repository's Test-First Rule. Then run the step's `verify` command from the
manifest — the narrowest check that settles the step. If `verify` is empty, use
the package's check from AGENTS.md "Verification Expectations" and add one to
the manifest in the same PR.

## 4. Review — fresh context, not the author

Three reviews run over the branch's full commit range, each **from a subagent with a
fresh context**. The implementer reviewing its own diff is confirmation bias with a
checklist; a review's value depends on it not sharing the author's model of the bug.

1. `adversarial-review`.
2. `holy-blocker-security` in Authoring mode.
3. `privacy-review` in Authoring mode.

Reviews 2 and 3 run **unconditionally, every time this gate runs** — not only when the diff
matches a trigger phrase in a skill's `references/review-triggers.md`. Trigger matching is how
each skill picks which boundary sections to load, never whether it runs.

Apply the two rules: reproduce or label, and cite or omit. Then triage every finding by
`docs/engineering/review-triage.md` §3:

| Tier | Route |
|---|---|
| Blocking | One fix attempt (`MAX_FIX_ATTEMPTS = 1`), one commit per finding, then re-run the finding's own reproduction against the fix and re-run the review. Still open after that attempt: stop, do not try a third time, and escalate. Nothing with an open Blocking finding merges. |
| Non-blocking | File a `kind = "bug"` step in the package's `steps.toml`, with `regressed_step` set when it regresses a `done` step. Fix inline only if it is a same-file, same-test-suite change to the current diff. |
| Judgment call | Never resolve it. Escalate. |

When anything escalates, report using the escalation packet from `review-triage.md` §4,
quoted verbatim — compose nothing freehand, and never list the findings that were cleanly
fixed and reverified:

> N findings. M auto-fixed and reverified. K stopped at the fix-attempt cap.
> J are judgment calls. [K and J are listed below, one line each: what's
> contested, and why it can't be resolved without your call.]

Append a row to the living log in `review-triage.md` §5 for each escalation. Only a review
that finds the step's premise false sends the step back to gate 1.

## 5. Close the books

- Move any coverage row the change alters in `docs/engineering/coverage.md` in
  the same PR. `Unverified` becomes `Covered` by an observation, never by an
  argument.
- Mark the step done in `plan.md`, and set `status`/`evidence` in `steps.toml`
  to the commit or PR that landed it. `validate` must pass.

## 6. Finish and merge by acceptance kind

Render the PR body from the ledger and fill it in — CI fails a PR that touches governed
code without a step id and non-empty audit and review sections:

```
python -m tools.plan.loop open <step-id> > body.md
python -m tools.plan.loop check --body-file body.md --base <base> --head HEAD
```

**E2e gate, before any merge.** If `tools/plan/e2e` exists, run the local e2e scenarios that
cover the changed paths and record each scenario and the commit it passed at under the PR
body's `## Adversarial review` section. A failed preflight is reported as **environment not
ready**, never as a regression, and the step is treated as unverifiable on this host. A green
CI does not imply the e2e layer passed; the PR record is the evidence. If `tools/plan/e2e`
does not exist yet, say so in the PR body.

Commit, push, and open the PR in one pass, then merge by acceptance kind:

- **`acceptance = "code"`** — done-ness is a claim about the diff. On a clean
  fresh-context review, green CI and a passing e2e gate, squash-merge.
- **`acceptance = "observation"`** — done-ness is a claim about the world. If a scenario
  can observe that truth, run it, record it, and merge on a pass. If no scenario can, or the
  host cannot run it, stop at the PR for a human to confirm against `coverage.md` and
  `outcomes.md`.
- **`acceptance = "product"`** — a human verdict closes it. Stop at the PR.

Default is `observation`, so a step must opt in to automatic merge.

**When to stop for the owner.** The loop runs a feature's steps without the owner and stops
only for: a product or architecture escalation, a step it cannot verify on the available host,
or the feature being done. When the feature is done, hand over the single command that
launches its demo (`demos/<feature>/`, interactive mode) and nothing else; the owner gives
impressions, not code review, and those become steps. See
`docs/decisions/feature-demos-and-local-e2e.md`.

After a merge, reap the worktree:

```
python -m tools.plan.worktrees reap          # dry run
python -m tools.plan.worktrees reap --yes
```

## What the loop must not do

- Merge an `observation` step whose truth no scenario observed.
- Attempt a third fix for a Blocking finding.
- Skip the security or privacy review because the diff "obviously" misses their triggers.
- Write a status file. Derive it.
- Reap a worktree with local-only commits, a dirty tree, an open PR, or
  non-disposable ignored files. `--force` relaxes only the last of those.
- Treat a `gh` failure as "no PR" — `unknown` is its own state, and the reaper
  leaves it alone.
- Trust a merged PR on a reused branch name: the PR must still target the base
  and its head must equal the local tip.
