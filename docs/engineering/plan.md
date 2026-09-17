# Engineering harness — plan

The repository's own tooling. This plan covers the ledger that makes "what is done,
and what is next" a machine-checkable fact instead of a hand-edited table, the rule
that ties the step loop to it, and the cleanup that follows from both.

The step ledger itself is `tools/plan/ledger.py`; the step loop that reads it is
`.claude/skills/step-loop/SKILL.md`. Step 1 extended the ledger's coverage from the
four packages that had a manifest to every component with a `plan.md`. Steps 2–4 are
the follow-ups it made visible: enforcement, the CLAUDE.md status migration, and the
CLAUDE.md/AGENTS.md divergence.

## Assumption audit

Run before implementation, per the repository's assumption-audit rule.

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| The four existing manifests were built on a shared "module N" heading convention that the remaining plans also follow | `for p in docs/components/*/plan.md; do grep -ci 'module [0-9]' "$p"; done` | all 17 remaining plans report **0** matches; the four manifest-backed plans carry module headings, the rest do not | **FALSE** — the remaining plans expose steps as `## Implementation order` numbered lists with `**Done.**` / `~~struck~~` markers, not module headings. Extraction must key off each plan's own structure |
| Every component that has a `plan.md` can carry a step marker comment without a heading rewrite | `grep -c '^## \|^### ' docs/components/*/plan.md` | every remaining plan has at least 3 `##` sections, including an `## Implementation order` list (android-service excepted: a superseded 51-line stub with no implementation order) | **HOLDS**, except `android-service`, which is handled as a single explicit supersession step |
| `tools.plan.ledger.validate` accepts a generated manifest iff every step has exactly one matching marker and ids/status/evidence are well-formed | `python -m tools.plan.ledger validate docs/components/domain-blocklist docs/components/text-policy docs/components/net-shield docs/components/mac-daemon` | exit 0, no problems | **HOLDS** |
| A `done` status can be anchored to `master` | `git cat-file -e master:packages/image-sandbox` and per-path `git log master -- <path>` | paths resolve on `master`; per-path log is available | **HOLDS** — `done` means "implementation exists on master"; work living only on a branch is `pending` with the branch named in `evidence` |
| `gh` is authenticated, so the loop's gate 6 can open a PR | `gh auth status` | logged in to github.com as `ip43-dutov-ivan`, `repo` scope present — but that account has **READ** on `walking-wisely/holy-blocker`, so it cannot create a PR | **FALSE for this machine's `gh` account** — push works over SSH as `walking-wisely`; PR creation is blocked until `gh` is re-authed as `walking-wisely`. Not a code problem and not fixable in-repo |

Two false claims, neither changing the module's purpose. The heading claim changed the
extraction convention; the auth claim is environmental and is recorded so a future
session does not read the loop's gate 6 as broken.

## Step 1 — ledger coverage **Done.**

<!-- step: engineering.ledger-coverage -->

Give every component with a `plan.md` a `steps.toml` and one step marker comment
per step in the plan, so `ledger next` answers for the whole repository and
not four packages. Seven components exist in some form and carry done steps
(`desktop`, `image-sandbox`, `machine-learning`, `mitm-proxy`, `mobile`,
`win-daemon`, and `win-network`, whose scaffold and first modules are on `master`
even though two of its steps are still stubs); five are planned but not created
(`backend`, `classifier-head`, `frontend`, `video-watchdog`, `voice-gate`);
`android-service` is a superseded stub whose single step records that it was
replaced by `apps/mobile`.

A manifest is thin. Status derivation and evidence anchoring are per the ledger's
own contract: `done` means the implementation exists on `master`, `evidence` is one
line naming where it landed, and live observation belongs to
[`../engineering/coverage.md`](../engineering/coverage.md), never here.

A repo-wide test (`tools/plan/tests/test_manifests.py`) discovers every
`docs/components/*/plan.md` and fails if a component has no manifest or if any
manifest does not validate against its plan, so future plans cannot silently opt out.

## Step 2 — loop enforcement

<!-- step: engineering.loop-enforcement -->

The loop exists and is voluntary. Nothing makes a session enter it, and nothing
checks that a PR came through its gates. This step makes entry the default and makes
the two gates that leave evidence (`assumption-audit` before, `adversarial-review`
after) machine-checkable, so "the loop ran" is a fact in the PR rather than a memory
in a transcript.

Deliverables, in order. Each is a repo-local change.

### 2.1 A trigger rule

Add a short, tree-shaped rule to the working-rhythm section of `CLAUDE.md`, and one
sentence to `.claude/skills/step-loop/SKILL.md` pointing at it:

> If the work matches a step marker comment in a `docs/components/**/steps.toml`,
> do not implement it directly — run `step-loop`. If no step matches but the change
> moves a `docs/engineering/coverage.md` row, add the step first, then run the loop.
> Everything else is loop-lite or exempt.

State the mandatory tiers verbatim, so a session classifies without arguing:

| Tier | What | Gates | Merge |
|---|---|---|---|
| Full loop | any manifest step; any file a coverage.md row cites; security triggers (preload/IPC, TLS/CA, TUN, named pipes, capture/OCR, permissions) | all 5 | auto only if `acceptance = "code"` |
| Loop, PR-only | `acceptance = "observation"` | all 5 | stop at PR; a human confirms against coverage.md |
| Loop-lite | docs-only, dependency bumps, mechanical refactor, frontend rendering | marker + coverage check + the package's own check | normal |
| Exempt | typo fixes; throwaway spikes that never leave the worktree | none | none |

### 2.2 `tools/plan/loop.py check`

A pure function plus CLI over the PR body. Signature is a design choice, not a
constraint; the contract is what must hold:

- Input: the PR body text, the changed-file list, and the repo root.
- If no changed file belongs to a manifest, exit 0 — the rule is not applicable.
  "Belongs to a manifest" needs a mapping the ledger does not have today: add an
  optional `paths = ["packages/domain-blocklist"]` field to the manifest schema in
  `tools/plan/ledger.py` and populate it for all 18 manifests in the same step. This
  is mechanical and is what lets the check answer "does this PR touch governed code?"
  without guessing from directory names.
- Otherwise, fail (exit 1) unless the body: (a) names at least one step id present
  in some manifest, and (b) contains a top-level `## Assumption audit` section and a
  top-level `## Adversarial review` section. Each section may be a one-line
  "N/A — <reason>", but the heading must be present. A missing heading is the
  failure mode this prevents.
- Unit tests first, in `tools/plan/tests/test_loop.py`: no manifest touched → passes;
  manifest touched and body empty → fails on both counts; body names an unknown step
  id → fails; body has the id but one heading missing → fails; body complete → passes.

### 2.3 `tools/plan/loop.py open [step-id]`

Render a PR body from a manifest entry — id, title, acceptance, verify, an empty
audit table, an empty review section — to stdout. This is the mechanism that puts
the artifacts in the PR instead of a transcript; it is what `gh pr create --body-file`
should be fed. Optional in the sense that `check` does not depend on it, but without
it the rule is a tax rather than a convenience.

### 2.4 CI

Extend the `plan-ledger` job. On `pull_request` events, write
`${{ github.event.pull_request.body }}` to a file, compute the changed files from the
event's base/head SHAs, and run the check. Add the manifest-bearing code roots to the
`plan` path filter (they now come from each manifest's `paths`), or the check never
runs for a code-only PR.

Acceptance: `code`. Verify: `python -m unittest discover -s tools/plan/tests -t .`.

## Step 3 — CLAUDE.md status is derived

<!-- step: engineering.claude-status-derived -->

`CLAUDE.md`'s "## Current State" table is a second, hand-edited source of truth for
package status, stale by construction. Step 1 made every component's status derivable
from `ledger next` / `render`; this step removes the table.

The rows carry measured narrative that is **not** status and must not be deleted
blindly: at least `"Authority Key Identifier"` (mitm-proxy) and the preprocess
off-by-one (image-sandbox) exist in no `docs/components/**/plan.md`. Procedure, per
row:

1. `grep` every distinctive phrase from the row across `docs/components/**/plan.md`
   and the package's `backlog.md`. For each fact with no hit, append it to that
   package's `plan.md` at the relevant module/section, verbatim and with its
   measurement context. Do not paraphrase — these are measured constants.
2. Once the row's facts live in the plan, replace the row with a pointer:
   `` | `apps/desktop` | see `python -m tools.plan.ledger next docs/components/desktop` | ``.
3. After all rows, replace the table with a short "Status is derived" note naming
   `ledger next` and `render`, and linking `../engineering/coverage.md`.
4. Update the `## Documentation` rule that mandates hand-editing the status table.
   Keep: a landed step updates its `plan.md` and its `steps.toml`. Remove the
   status-table instruction.

Guard, added in this step: extend `tools/plan/tests/test_manifests.py` (or a sibling)
to assert `CLAUDE.md` no longer contains the hand-edited table header and does name
`tools.plan.ledger`.

Acceptance: `code`. Verify: `python -m unittest discover -s tools/plan/tests -t .`.
The loss risk is real: the reviewer must diff each deleted row against the plans it
was migrated into — a deleted measured fact is invisible to a green test.

## Step 4 — one guidance file, not two

<!-- step: engineering.agents-md-dedupe -->

`CLAUDE.md` (77 KB) and `AGENTS.md` (7.8 KB) have diverged. `AGENTS.md` is a stale
subset that still says `packages/net-shield` and `packages/image-sandbox` are
"planned but not yet created"; `CLAUDE.md` carries the current state. Agents are
handed one or both depending on the client, so the two answer the same questions
differently — the exact drift this plan exists to remove, one level up.

Deliverables:

1. Decide the single canonical file. Recommendation: keep `CLAUDE.md` (the one that
   gets maintained) and reduce `AGENTS.md` to a one-screen pointer at it plus the
   build/test commands, so a client that only reads `AGENTS.md` still finds its way.
2. Reconcile the stale rows the pointer removes rather than keeping two copies.
3. Record the decision in `docs/decisions/` if it is not obvious, per the repo's
   decision-doc convention.

Acceptance: `code`. Verify: a test asserting `AGENTS.md` contains no "planned but not
yet created" list that contradicts a manifest, and that it names `CLAUDE.md`.

## What this does not cover

- Automatic merge of `observation` steps. The loop never merges those.
- A scheduled runner. Enforcement here is per-PR; nothing runs the loop unprompted.
- Reaping the ~30 pre-existing worktrees whose branches are already merged. The
  reaper exists (`python -m tools.plan.worktrees reap`); running it is an operation,
  not a step, and is deliberately left to a human.
- Migrating narrative for the components whose status row is a single line (the
  planned ones). They have little to move.
