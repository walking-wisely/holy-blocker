# Engineering harness — plan

The repository's own tooling. This plan covers the ledger that makes "what is done,
and what is next" a machine-checkable fact instead of a hand-edited table, plus the
rule that ties the step loop to it.

The step ledger itself is `tools/plan/ledger.py`; the step loop that reads it is
`.claude/skills/step-loop/SKILL.md`. This plan is about extending the ledger's
coverage from the four packages that have a manifest today to every component with
a `plan.md`, and about removing the hand-maintained status table it makes redundant.

## Assumption audit

Run before implementation, per the repository's assumption-audit rule.

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| The four existing manifests were built on a shared "module N" heading convention that the remaining plans also follow | `for p in docs/components/*/plan.md; do grep -ci 'module [0-9]' "$p"; done` | all 17 remaining plans report **0** matches; the four manifest-backed plans carry module headings, the rest do not | **FALSE** — the remaining plans expose steps as `## Implementation order` numbered lists with `**Done.**` / `~~struck~~` markers, not module headings. Extraction must key off each plan's own structure |
| Every component that has a `plan.md` can carry a step marker comment without a heading rewrite | `grep -c '^## \|^### ' docs/components/*/plan.md` | every remaining plan has at least 3 `##` sections, including an `## Implementation order` list (android-service excepted: a superseded 51-line stub with no implementation order) | **HOLDS**, except `android-service`, which is handled as a single explicit supersession step |
| `tools.plan.ledger.validate` accepts a generated manifest iff every step has exactly one matching marker and ids/status/evidence are well-formed | `python -m tools.plan.ledger validate docs/components/domain-blocklist docs/components/text-policy docs/components/net-shield docs/components/mac-daemon` | exit 0, no problems | **HOLDS** |
| A `done` status can be anchored to `master` | `git cat-file -e master:packages/image-sandbox` and per-path `git log master -- <path>` | paths resolve on `master`; per-path log is available | **HOLDS** — `done` means "implementation exists on master"; work living only on a branch is `pending` with the branch named in `evidence` |
| `gh` is authenticated, so the loop's gate 6 can open a PR | `gh auth status` | logged in to github.com, `repo` scope present | **HOLDS** |

No claim was unverifiable here. The one false claim changed the extraction convention,
not the module's purpose: the remaining plans still yield steps, just from their
implementation-order lists rather than a heading family.

## Step 1 — ledger coverage

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

## Step 2 — CLAUDE.md status is derived

<!-- step: engineering.claude-status-derived -->

Once every component has a manifest, the hand-edited "Current State" table in
`CLAUDE.md` is a second, staler source of truth. Replace the rows it duplicates with
a pointer to the derived views (`python -m tools.plan.ledger next` /
`render`), after moving each row's measured narrative into the package's own
`plan.md`, where the ledger's own contract says gotchas belong. The
`## Documentation` rule that instructs agents to hand-edit the status table is
removed at the same time; plan.md and `steps.toml` remain the two things a landed
step updates.

## What this does not cover

- Automatic merge of `observation` steps. The loop never merges those; this plan is
  `code`-acceptance only.
- A schedule or hook that forces the loop to run. This plan makes the ledger
  universal and removes the competing status table; it does not make entry into the
  loop mandatory. That is unbuilt.
- Migrating narrative for components whose status row is a single line (the planned
  ones). They have little to move.
