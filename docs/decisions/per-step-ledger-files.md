# ADR: One file per step in the ledger

| Field | Value |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-10-02 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | Decisions 3 and 4 and the "derive `done`" rejection, by [ledger-order-and-derived-status.md](ledger-order-and-derived-status.md) |

Tier: architecture, decided by the owner. It meets none of the escalation triggers in
[decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md) (no ML, no
cross-platform contract, no trust boundary, reversible by a later PR), so the red-team is
tier 0 and the attack on it is recorded below instead.

---

## Context

The repository runs many parallel worktrees, and each step PR edits shared state files. Two
measurements, taken on 2026-10-02:

| Measurement | Result |
|---|---|
| Two branches each flip a different `pending` step in `docs/components/mac-daemon/steps.toml` | merges cleanly |
| Two branches each append a new `[[step]]` to the end of the same `steps.toml` | **conflicts** |
| Files most often touched by merged non-merge commits since 2026-09-01 | `docs/engineering/steps.toml` 13, `docs/engineering/plan.md` 13, `CLAUDE.md` 9, `docs/components/machine-learning/plan.md` 6 |

So the conflict is not "every merge, every step". It is: **appends to a shared file**
(a new step, a new plan section), **two edits to the same step**, and **state that is written
twice** — a step's status lives in `steps.toml` and again as a `**Done.**` strike-through in
`plan.md`, so one logical change touches two hot files.

The same drift shows in the ledger itself: `docs/engineering/steps.toml` on master carries
three steps at `at-pr` whose PRs have merged. A status that a human must remember to flip
after merge is stale by construction.

## Decision

1. **One file per step.** A component's steps live at
   `docs/components/<component>/steps/<step-id>.toml`, one step per file, flat keys, the file
   stem equal to the step `id`. A new step is a new file, so two branches that add steps
   never touch the same path.
2. **Package metadata in `package.toml`.** The code roots (`paths`) move out of the step list
   into `docs/components/<component>/package.toml`. It changes when a component gains a code
   root, not when a step lands.
3. **Order comes from `plan.md`.** `ledger next` offers steps in the order their
   `<!-- step: id -->` markers appear in `plan.md`; steps with no marker sort last by id.
   There is no `order` field to renumber.
4. **Status is written once.** The step file's `status` is the only record of whether a step
   is done. The `**Done.**` strike-through in `plan.md` is no longer required; existing ones
   stay as history and nothing reads them.
5. **No derived table is committed.** `ledger render` prints to stdout. A test fails if a
   rendered ledger table appears in a tracked file.
6. **Legacy `steps.toml` is read, flagged, and migrated.** For a transition the loaders still
   read a legacy `steps.toml`; `validate` reports it as a problem on the head;
   `python -m tools.plan.ledger migrate <dir>` converts one. Branches opened before the
   change convert with that command. A later step deletes the legacy reader.

## Why this scales

Git merges by path first and by line hunk second. Per-step files make the unit of change a
path, so concurrent work on different steps is conflict-free by construction instead of by
luck of line spacing. The remaining shared writes are the ones that are genuinely one fact
edited twice (the same step) or prose (`plan.md` sections), and the second is only touched
when a plan is authored, not when a step lands.

## Rejected alternatives

| Alternative | Why not |
|---|---|
| GitHub Issues / Projects as the source of truth | Not versioned with the code (a branch cannot say "done here, pending on master"), needs network and `gh` auth for `ledger next` and `todos`, and cannot be validated against `plan.md`, `coverage.md` and manifest `paths` in CI. A one-way mirror to a Project board is compatible with this decision and is left for later |
| Derive `done` from merged PRs that name the step id | Removes the status field entirely, but makes `ledger next` need `gh` or full git history, breaks offline use and worktree-local views, and cannot represent `at-pr` or `observation` acceptance. Revisit if status drift persists after this change |
| Keep one `steps.toml`, add `merge=union` in `.gitattributes` | Union merge concatenates both sides, which duplicates a step edited on both and silently accepts two conflicting statuses for the same id |
| An `order` field in each step file | Two steps inserted at the same position renumber each other, recreating the conflict in a smaller file |
| One file per component for status plus a separate definitions file | Status for a component is still one file every step PR edits |

## What this does not cover

- `plan.md` is still a shared file. Two branches that each add a section at the end of the
  same plan still conflict. That is a plan-authoring conflict, rarer than a landing, and is
  not removed here.
- `CLAUDE.md`, `docs/status/<package>.md` and `docs/engineering/coverage.md` stay shared and
  hand-edited. `CLAUDE.md`'s status table is addressed by `engineering.claude-status-derived`.
- Branches opened before the change conflict once (modify/delete on `steps.toml`) when they
  merge. The `migrate` command resolves it mechanically.
- A step file renamed or deleted on one branch while edited on another is a conflict, and
  correctly so.
