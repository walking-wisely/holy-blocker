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

## 3. Implement, test-first

Write focused unit tests for new logic before the implementation, per the
repository's Test-First Rule. Then run the step's `verify` command from the
manifest — the narrowest check that settles the step. If `verify` is empty, use
the package's check from AGENTS.md "Verification Expectations" and add one to
the manifest in the same PR.

## 4. Adversarial review — fresh context, not the author

Run the `adversarial-review` skill on the branch's full commit range **from a
subagent with a fresh context**. The implementer reviewing its own diff is
confirmation bias with a checklist; the review's value depends on it not sharing
the author's model of the bug.

Apply the two rules: reproduce or label, and cite or omit. Fixes are a separate
pass — one finding per commit, and re-run the finding's reproduction command
against the fix. If the review finds a load-bearing gap, go back to gate 1.

## 5. Close the books

- Move any coverage row the change alters in `docs/engineering/coverage.md` in
  the same PR. `Unverified` becomes `Covered` by an observation, never by an
  argument.
- Mark the step done in `plan.md`, and set `status`/`evidence` in `steps.toml`
  to the commit or PR that landed it. `validate` must pass.

## 6. Finish and merge by acceptance kind

Commit, push, and open the PR in one pass.

- **`acceptance = "code"`** — done-ness is a claim about the diff. On a clean
  fresh-context review and green CI, squash-merge.
- **`acceptance = "observation"`** — done-ness is a claim about the world. Stop
  at the PR. A human confirms the observation against `coverage.md` and
  `outcomes.md` and merges. The loop never merges these itself.

Default is `observation`, so a step must opt in to automatic merge.

After a merge, reap the worktree:

```
python -m tools.plan.worktrees reap          # dry run
python -m tools.plan.worktrees reap --yes
```

## What the loop must not do

- Merge an `observation` step.
- Write a status file. Derive it.
- Reap a worktree with local-only commits, a dirty tree, an open PR, or
  non-disposable ignored files. `--force` relaxes only the last of those.
- Treat a `gh` failure as "no PR" — `unknown` is its own state, and the reaper
  leaves it alone.
- Trust a merged PR on a reused branch name: the PR must still target the base
  and its head must equal the local tip.
