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

## Steps 5–8 — bug tracking and mandatory review gates

These four steps close a gap surfaced in conversation, not in code: `steps.toml`
today only models forward-looking feature work (no way to file a bug distinct from
a planned step), and the two review skills that exist (`holy-blocker-security`,
and the not-yet-created `privacy-review`) are opt-in — triggered by keyword or path
match, never guaranteed to run. For a project whose entire surface *is* trust
boundaries (MITM proxy + local CA, VPN/TUN, tamper guards, OS permission grants,
screen/AX capture), a gate that only fires when a diff happens to match a trigger
phrase will miss exactly the boundary nobody thought to name.

**Execution note.** Steps 6 and 7 have no dependency on each other and can run in
parallel worktrees/subagents. Step 5 is independent of both. Step 8 depends on all
three and must run last — it wires the other three together and cannot itself be
started until they exist. Run `python -m tools.plan.ledger next docs/engineering`
to find the next unblocked step; do not start a step whose `depends_on` has not
landed.

### Step 5 — bug kind and regression links in the ledger schema

<!-- step: engineering.bug-kind-ledger -->

`tools/plan/ledger.py`'s `Step` has no way to represent "a bug was found" as
distinct from "a feature is planned." This step adds that, test-first.

Deliverables, in order:

1. **Tests first**, in `tools/plan/tests/test_ledger.py`:
   - a `Step` built with no `kind` argument defaults to `kind == "feature"`.
   - `ledger.validate` accepts a step with `kind = "bug"`.
   - `ledger.validate` rejects `kind = "vulnerability"` (or any value outside
     `{"feature", "bug"}`) with a problem string containing `"invalid kind"`.
   - a step with `regressed_step = "p.a"` where `"p.a"` exists in the same manifest
     passes validation.
   - a step with `regressed_step = "p.ghost"` where no such id exists in the
     manifest fails validation with a problem string containing
     `"unknown regressed_step"`.
   - a step with `kind = "bug"` and no `regressed_step` is valid — a bug found
     during manual or exploratory testing is not always tied to one prior step.
   - `render()`'s output contains a `Kind` column, and a `kind = "bug"` row is
     visually distinguishable from a `kind = "feature"` row in the rendered text
     (e.g. the literal string `"bug"` appears in that row).

2. **Implementation** in `tools/plan/ledger.py`:
   - Add `VALID_KIND = ("feature", "bug")` and `DEFAULT_KIND = "feature"` next to
     the existing `VALID_STATUS` / `VALID_ACCEPTANCE` constants, with the same
     one-line-per-value doc-comment style already used there.
   - Add `kind: str = DEFAULT_KIND` and `regressed_step: str = ""` fields to the
     `Step` dataclass.
   - `load_manifest`: read `raw.get("kind", DEFAULT_KIND)` and
     `raw.get("regressed_step", "")`.
   - `validate`: append a check that `step.kind in VALID_KIND` (same shape as the
     existing status/acceptance checks), and — mirroring the existing "unknown
     dependency" check for `depends_on` — a check that `step.regressed_step`, if
     non-empty, is one of `ids`.
   - `render`: add a `Kind` column to the table header and one cell per row.

3. **`tools/plan/todos.py`**: read the file before editing it — its current output
   shape is not specified here. Add a count or a distinct marker for `kind = "bug"`
   steps in its report, with a matching test, so a fleet-wide scan surfaces open
   bugs alongside pending feature steps rather than mixing them silently.

Acceptance: `code`. Verify: `python -m unittest discover -s tools/plan/tests -t .`.

#### Assumption audit

| Claim | Falsifier | Observed | Verdict |
|---|---|---|---|
| `render`'s current table shape (`\| Step \| Status \| Evidence \|`) is asserted only in `test_ledger.py`'s render test and defined only in `ledger.py:170` | `grep -rn "Step \| Status" tools/plan/` | matches only `test_ledger.py:64` and `ledger.py:170` | **HOLDS** — adding the `Kind` column updates exactly those two sites, no other consumer |
| Baseline test suite is green on this substrate commit, so later failures are attributable to this change | `python -m unittest discover -s tools/plan/tests -t .` | 85 tests, OK | **HOLDS** (run with `python3`; this machine has no `python` binary — verify-string discrepancy, non-load-bearing) |
| `ledger.Step(...)` is constructed with at most 4 positional args (id/title/status/evidence) plus keywords, so two trailing defaulted fields are backward compatible | `grep -n "Step(" tools/plan/tests` and `tools/plan/todos.py` | all constructions ≤ 4 positional args; `todos.py` uses keywords throughout | **HOLDS** |
| `todos.Todo(...)` is constructed only in `test_todos.py`'s `_todo` helper with keyword args; appending a defaulted `kind` field breaks nothing | `grep -rn "Todo(" tools/plan/tests/*.py` | single construction site, keyword args only | **HOLDS** |
| No existing `kind` or `regressed_step` key in any steps.toml or tool file, so the new optional keys cannot collide with or re-validate existing data | `grep -rn "regressed_step\|kind" docs/engineering/steps.toml tools/plan/*.py` | only the `engineering.bug-kind-ledger` step id/name match; no data keys | **HOLDS** |
| `gh` can open a PR against `walking-wisely/holy-blocker` | `gh auth status` | active account `walking-wisely`, `repo` scope | **HOLDS** — the plan's earlier FALSE (old READ-only account) is obsolete; the account was re-authed |

### Step 6 — review triage policy

<!-- step: engineering.review-triage-policy -->

Create `docs/engineering/review-triage.md`. This document answers the question
"after a review finds something, what happens?" — today `step-loop` gate 5 only
says "if the review finds a load-bearing gap, go back to gate 1," which leaves
every non-blocking finding and every judgment call unspecified.

Required sections, in this order:

1. **Severity tiers**, mapped onto `adversarial-review`'s existing consequence
   axis ("a guard that fails open silently outranks a crash"), with one concrete
   example from this repo per tier:
   - **Blocking** — the module's stated purpose is not actually true, a trust
     boundary breaks, a guard fails open. Example: a DNS shield that answers a
     blocked domain with the real address under some code path.
   - **Non-blocking** — a real bug, off the current diff's critical path.
     Example: an off-by-one in a log formatter that doesn't affect a decision.
   - **Judgment call** — the right answer depends on risk tolerance or product
     taste, not on what is true. Example: should a permission-check timeout fail
     open or fail closed.
2. **Fix-attempt cap.** Exactly one fix attempt is made per blocking finding after
   it is first reported, re-verified against the review's own reproduction
   command. If it does not clear on that second attempt, stop — do not attempt a
   third time. State this as a hard number, not a guideline: `MAX_FIX_ATTEMPTS = 1`.
3. **Routing table**: blocking → fix within the cap, then re-review; non-blocking
   → filed as a `kind = "bug"` step in the package's `steps.toml` (set
   `regressed_step` when the finding is a regression against an already-`done`
   step), never fixed inline unless it is a same-file, same-test-suite change to
   the current diff; judgment call → never resolved by the loop, always escalated.
4. **Escalation packet format**, verbatim, for the loop to quote rather than
   compose freehand:
   > N findings. M auto-fixed and reverified. K stopped at the fix-attempt cap.
   > J are judgment calls. [K and J are listed below, one line each: what's
   > contested, and why it can't be resolved without your call.]
   The packet never lists the M that were cleanly fixed and reverified.
5. **A living log**, starting as an empty table with columns
   `| date | finding | routed-as | should-have-been | why |`, appended to by every
   step-loop run that escalates something. This is how the auto-fix/escalate
   boundary tightens from observed outcomes instead of staying a fixed guess.

This document decides what surfaces to the product owner and what doesn't — that
is itself a contract-shaped product decision under step-loop's own gate 3.
**Do not treat this step as done on a green diff.** It must go through gate 3
(a one-page "these are the tiers, this is the cap, approve?") before merge.

Acceptance: `product`. The loop stops at the PR; a human closes it.

### Step 7 — privacy-review skill

<!-- step: engineering.privacy-review-skill -->

Create `.claude/skills/privacy-review/SKILL.md`, structured like
`.claude/skills/holy-blocker-security/SKILL.md` (read that file first — same
two-mode shape, same routing discipline):

- **Authoring mode** (scope: a diff) and **Audit mode** (scope: a package,
  findings written to `docs/engineering/privacy-backlog.md`, never fixed during
  an audit — same rule as the security skill's audit mode).
- An explicit scope-discipline paragraph up top: this skill is not legal advice
  and does not certify GDPR compliance. It answers one narrower, mechanical
  question — what personal or sensitive data does this diff create, store, or
  transmit, for how long, and does it ever leave the device (cross-checked
  against CLAUDE.md's no-cloud-calls rule).
- A `references/data-classes.md` file (parallel to the security skill's
  `references/review-triggers.md`) enumerating this project's actual sensitive
  data classes and where they already live — e.g. captured screen frames
  (`ScreenCapture`/`FrameSink` in mac-daemon, `ScreenCaptureService` in mobile),
  extracted AX/OCR text, tamper-log entries (`TamperLog.kt`/`TamperLogStore.kt`),
  classifier scores and verdicts, and which domains a device visited (implied by
  blocklist hits). For each: whether it is special-category data under GDPR
  Art. 9 (a sexual-content classification score about a specific person's screen
  is — this product's core function produces exactly that), its current
  retention (cite the file that sets it, or mark "unmeasured" as its own
  finding), and whether any code path transmits it off-device.
- **Step 1 (route)**: if a diff touches none of the enumerated data classes and
  introduces no new one, say so and stop — same discipline as the security
  skill's "nothing matches" rule.
- **Step 2 (apply)**: for each touched data class, check retention is finite and
  stated, check no new transmission path was added, and check any genuinely new
  kind of stored/logged data was added to `references/data-classes.md` as part
  of the same change, not left implicit.

Same reasoning as step 6: this skill defines what "privacy-sensitive" means for
the product, which is a product-owner call, not a model call.

Acceptance: `product`. The loop stops at the PR; a human closes it.

### Step 8 — wire mandatory gates into step-loop

<!-- step: engineering.mandatory-review-gates -->

Everything up to here is inert until `step-loop` actually calls it unconditionally.
This step edits `.claude/skills/step-loop/SKILL.md` only; steps 5–7 must exist
first, which is why they are listed as dependencies.

1. In the gate covering adversarial review, add: after `adversarial-review` runs,
   run `holy-blocker-security` in Authoring mode and `privacy-review` in
   Authoring mode, both against the branch's full diff, both from a fresh-context
   subagent — **unconditionally, every time this gate runs**, not gated on
   whether the diff happens to match a trigger phrase in
   `references/review-triggers.md`. Trigger-word matching stays as the *routing*
   mechanism inside each skill (which boundary sections to load), never as the
   decision of whether the skill runs at all.
2. Replace "if the review finds a load-bearing gap, go back to gate 1" with the
   routing table from `docs/engineering/review-triage.md` §3, and quote the
   escalation packet format (§4) verbatim rather than re-describing it.
3. Update the tier table from step 2.1 ("Full loop" row's trigger list) — the
   "security triggers" entry becomes "every full-loop step, unconditionally,"
   since this step removes the opt-in condition.
4. **Test** (structural only — this step edits prose, not logic): add
   `tools/plan/tests/test_step_loop_gates.py` asserting
   `.claude/skills/step-loop/SKILL.md` contains the strings
   `"holy-blocker-security"`, `"privacy-review"`, `"MAX_FIX_ATTEMPTS"`, and
   `"review-triage.md"`; and that `.claude/skills/privacy-review/SKILL.md` and
   `docs/engineering/review-triage.md` both exist on disk (this is what makes
   step 8 mechanically unable to land before 6 and 7 do, independent of the
   ledger's own `depends_on` check).

Acceptance: `code`. Verify: `python -m unittest discover -s tools/plan/tests -t .`.
The policy content itself was already approved in steps 6 and 7's own gate 3 — this
step is pure wiring and needs no separate product verdict.

## What this does not cover

- Automatic merge of `observation` steps. The loop never merges those.
- A scheduled runner. Enforcement here is per-PR; nothing runs the loop unprompted.
- Reaping the ~30 pre-existing worktrees whose branches are already merged. The
  reaper exists (`python -m tools.plan.worktrees reap`); running it is an operation,
  not a step, and is deliberately left to a human.
- Migrating narrative for the components whose status row is a single line (the
  planned ones). They have little to move.
- A GDPR compliance audit against a lawyer's checklist. `privacy-review` (step 7)
  is a mechanical data-inventory gate, not a certification.
