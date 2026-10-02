# ADR: Computed step order and derived step status

| Field | Value |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-10-02 |
| **Owner** | Ivan Dutov |
| **Supersedes** | Decisions 3 and 4 and the "derive `done`" rejection in [per-step-ledger-files.md](per-step-ledger-files.md) |
| **Superseded by** | — |

Tier: architecture, decided by the owner. It meets none of the escalation triggers in
[decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md) (no ML, no
cross-platform contract, no trust boundary, reversible by a later PR), so the red-team is
tier 1 on one lens, and its findings are recorded below.

---

## Context

[per-step-ledger-files.md](per-step-ledger-files.md) made a step one file, so two branches adding
different steps no longer share a path. Two shared writes remain on every step's path:

- **Order lives in `plan.md`.** `ledger next` offers steps in marker order, and a step's prose
  is a `plan.md` section, so adding a step edits the same file as every other new step.
- **Status is written by the PR that lands the step.** A PR cannot know its own merge state,
  so the field is stale by construction. Measured 2026-10-02: four steps on master carry
  `status = "at-pr"` whose PRs (#103, #104, #105, #106) have merged.

The owner's operating model is auto-merge when CI is green, verification by an e2e run and a
demo, and no hand-tinkering with bookkeeping. A status a human or a PR must remember to flip
contradicts that.

Measured facts the decision rests on, taken 2026-10-02:

| Claim | Command | Result |
|---|---|---|
| A step already declares prerequisites | `grep -n depends_on tools/plan/ledger.py` | `depends_on` is in `STEP_KEYS`, with dangling-edge and cycle checks in `_dependency_problems` and `_cycles` |
| Squash merges keep every commit body | `gh api repos/{owner}/{repo} --jq .squash_merge_commit_message` | `COMMIT_MESSAGES` |
| Co-author lines follow the body | `git log master -1 --format=%b` | `Co-authored-by:` is appended after the body, so a step line is not the final trailer and must be matched as a line anywhere in the message |
| Every CI job has history | `grep -n fetch-depth .github/workflows/ci-plan.yml` | only some jobs set `fetch-depth: 0` |
| Repo auto-merge is on | `gh api repos/{owner}/{repo} --jq .allow_auto_merge` | `false` |

## Decision

1. **Order is computed, never stored.** `ledger next` and `ledger render` sort steps
   topologically over `depends_on`. Among steps that are ready together, the next is the
   smallest by `(group, id)`, with ungrouped steps after grouped ones. Two branches that add
   independent steps need no agreement on order, because there is none to agree on.
2. **`group` is an optional step key.** A label matching `[a-z0-9-]+` that clusters related
   steps in the rendered order, with ungrouped steps after every group. It is presentation, not
   gating: only `depends_on` blocks a step. `depends_on` stays inside one component; an edge
   naming another component's step is a validation error, because step ids are unique only per
   component.
3. **`plan.md` does not list steps.** It keeps the overview, rationale, audit tables and the
   "what this does not cover" section. A step's specification lives in
   `steps/<step-id>.md` beside its `.toml`, so adding a step adds files and edits no shared
   one. Markers in existing plans stay valid and are no longer required.
4. **Status is derived, not stored.** A step file holds its definition only (`id`, `title`,
   `acceptance`, `depends_on`, `verify`, `kind`, `group`, `difficulty`, `regressed_step`);
   it carries no `status` and no `evidence`. The state of a step is computed:

   | State | Computed from |
   |---|---|
   | `done` | a commit reachable from the base branch that carries the step line, is not reverted, and changed the step's own files or a path it governs; for `acceptance = "code"` nothing more |
   | `in-flight` | an open PR when `gh` is available, or a checked-out local worktree, whose branch carries the step line and is not already `done`. Replaces the stored `at-pr` and keeps its rule: an in-flight step is not offered again and does not satisfy a dependency |
   | `merged` | the same commit on the base for an `acceptance = "observation"` or `"product"` step with no passing e2e record. It does not satisfy a dependency until observed, and `ledger next` reports "blocked on unobserved <id>" instead of going quiet |
   | `pending` | none of the above |

   The rules that keep the derivation honest:

   - **The step line** is exactly `Step: <id>` at the start of a line of a commit message, parsed
     with an anchored pattern and compared by set membership, never by substring search. Indented
     and quoted lines do not count. It is a commit-message line, distinct from the
     ``Step: `id` — title`` line `loop check` prints in a PR body.
   - **A revert undoes `done`.** A commit whose body says `This reverts commit <sha>` removes the
     reverted commit from the step's landings, and a revert of that revert restores it.
   - **`done` wins over `in-flight`.** A squash-merged branch that is never deleted does not keep
     its step in flight. Bare branches and remote-tracking refs are not read, so an abandoned
     branch cannot hold a step forever; a worktree counts only while it is checked out.
   - **A step is claimed at its start.** `step-loop` makes the first commit in its worktree an
     empty commit carrying the step line, so a second agent running `ledger next` sees it
     in flight before any work exists. The squash drops nothing that matters; the line is
     repeated by the real commits.
   - **One step per PR.** `loop check` rejects a PR whose commits name more than one distinct
     step id, so a squash cannot mark several steps done.

   Observation is recorded the way [e2e-scenario-contract.md](e2e-scenario-contract.md)
   already specifies, as a pass note keyed on the covered-path tree hash. Until the runner that
   writes those notes exists, an observation step stays `merged` and the owner promotes it as
   today. This decision does not change what counts as `Covered`.
5. **Existing steps keep their history through a one-time `landed` key.** The 159 steps already
   done have no `Step:` line in their commits. Migration writes `landed = "<sha>"` once into
   each done step's file, found from the evidence it already holds. The reader verifies the sha
   is an ancestor of the base, and `loop check` rejects `landed` on any step file not already on
   the base, so it cannot be used to mark a new step done. The key is removed by the step that
   deletes the legacy reader's last consumer; new steps never carry it.
6. **`loop check` requires the line.** A PR whose diff touches a governed path must carry a
   `Step: <id>` line in a commit message naming a step that governs that path. With
   `COMMIT_MESSAGES` the line survives the squash.
7. **One validator.** `loop check` calls the function `ledger validate` uses for layout rules,
   so CI and a local run report the same problem.
8. **No derived table is committed.** `ledger render` prints to stdout, and a test fails when a
   rendered ledger table appears in a tracked file. The `**Done.**` strike-through is no longer
   required anywhere.

## Why this scales

A step PR now adds files and touches nothing else shared. Its status is not a fact it writes
but a fact git already holds once it merges, so there is no post-merge flip to forget and no
stale `at-pr`. Concurrent merges in any order produce the same ledger: the order is a function
of the edges and labels, not of arrival.

## Red-team findings

One fresh-context run on the bypass lens. Eight findings, all folded in; the ones that changed
the design are marked.

| # | Finding | Evidence | Resolved by |
|---|---|---|---|
| 1 | A reverted step still reads `done` (**changed the design**) | reproduced: `git revert` leaves the `Step:` line reachable | decision 4, revert rule |
| 2 | A `Step:` line quoted, indented or pasted in a prose body forges `done` (**changed the design**) | reproduced: quoted and indented lines appear in `%b` | decision 4, strict line parse plus the path check in `done` |
| 3 | A prefix id matches a loose grep | reproduced: `Step: foo` also matched `Step: foo-bar` | decision 4, anchored match and set membership |
| 4 | A stale branch holds a step in flight forever; a squash-merged branch stays in flight; two agents start one step before either commits; a shallow clone reads everything pending (**changed the design**) | reproduced for the branch and shallow cases; the double-start is by design, unverified | decision 4, `done` wins, worktree-only in-flight, claim commit; the shallow guard below |
| 5 | `landed` is a hand-editable done flag | design-level, unverified | decision 5, ancestor check and a `loop check` rejection |
| 6 | `group` sorts case-sensitively and an ungrouped step breaks the sort key; cross-component edges bind to the wrong id | reproduced for the sort; `ledger.py` builds its id map per manifest | decision 2, label pattern, sentinel and explicit rule |
| 7 | A squash names several steps; an observation step in a chain blocks everything silently | multi-line match reproduced; squash behaviour unverified | decision 4, one step per PR and the "blocked on unobserved" message |
| 8 | Between this decision and the migration the validator and the new rule disagree | design-level, unverified | `engineering.ledger-derived-status` lands reader, validator and migration in one PR and tests that `status` and `evidence` are rejected after the cut |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| Keep a stored `status` and add a bot that flips it on merge | Needs a bot token and push rights to master, adds a commit per merge that races other merges, and still leaves a stored fact that can disagree with git |
| Keep a stored `status` and reconcile it with a script | Two sources of truth that a script keeps in step; every failure of the script is a stale ledger again |
| Marker order in `plan.md` | One shared file every step-adding PR edits, and an order two concurrent branches must negotiate. Superseded; [per-step-ledger-files.md](per-step-ledger-files.md) decision 3 |
| A numeric `order` or filename prefix | Two inserts at the same position renumber each other |
| Derive `done` from merged PRs by number or from `gh` alone | Needs network and auth; the line in the commit message is local to git. `gh` is used only to see an open PR |
| GitHub Issues or Projects as the source of truth | Rejected in [per-step-ledger-files.md](per-step-ledger-files.md) and unchanged: not versioned with the code. A one-way mirror to a Project board stays compatible |

## What this does not cover

- `docs/status/<package>.md`, `docs/engineering/coverage.md` and `CLAUDE.md` stay shared,
  hand-edited files. `engineering.claude-status-derived` already plans the `CLAUDE.md` table;
  the other two are not addressed here and are the next conflict surface.
- A shallow clone cannot see old `Step:` lines. A job that needs derived status sets
  `fetch-depth: 0`, and `ledger` fails loudly on a shallow repository rather than reporting
  every step pending.
- A `Step:` line on a commit that does not implement the step marks it done. `loop check`
  ties the line to a governed path, which narrows that, and does not prove the work.
- Enabling repo auto-merge (`allow_auto_merge` is `false`) is a repository setting and is
  left to the owner; this decision does not depend on it.
- Observation steps are not promoted to `done` by this change.
