# ADR: Session model and harness

| Field | Value |
|---|---|
| **Status** | Proposed — awaiting owner approval; nothing here is built |
| **Date** | 2026-10-02 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

Tier: architecture. It adds a trust boundary (agent-editable permissions and hooks) and extends a
cross-platform contract ([e2e-scenario-contract.md](e2e-scenario-contract.md)), so the red-team ran at tier 2:
bypass, intent, and legal and privacy, one fresh-context reviewer each. Their findings are in "Red-team
findings" and changed the design in six places.

---

## Context

The owner wants each AI session to serve one purpose, state to live in the repo and not in a session's
context, and the owner to meet a feature once, at a demo, plus an early plan approval for new capabilities
and one-way doors. The owner does not review code
([decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md),
[feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md)).

The input to this decision was a list of sixteen conclusions from a design discussion, handed over as claims
to verify. Checked against master on 2026-10-02:

| Claim in the input | Command | Result |
|---|---|---|
| Transcripts are JSONL under `~/.claude/projects` and token totals can be derived | `tools/plan/context.py`; a scan of the largest transcript | **Holds, with a trap.** `context.py` already reads them, but one transcript has 134 assistant lines for 71 distinct message ids, so a naive sum of `usage` double counts. Subagents write to `<session>/subagents/*.jsonl`, outside the main file |
| Transcripts can be sampled without handling images | scan of 47 holy-blocker transcripts | **False.** 2 contain `"type":"image"` and 26 contain `base64`. A digest read by a model session sends whatever it holds to the model API |
| Owner gate state belongs "in the ledger" | [ledger-order-and-derived-status.md](ledger-order-and-derived-status.md), decision 4 | **Conflicts.** Status is derived, never stored. Gate state must be derived from git and notes |
| Confirmation is bound to a commit SHA | [e2e-scenario-contract.md](e2e-scenario-contract.md) §3 | **Conflicts.** A squash changes SHAs; the contract keys on the tree hash of the covered paths, which already keeps a confirmation across a tree-identical rebase |
| The brief is a committed artifact pinned to a base SHA | per-step ledger ADRs | **Self-defeating.** A committed SHA is stale the moment master moves. The brief is generated; only its inputs are committed |
| "Is the demo standard a feature of mitm-proxy or a cross-package contract?" is open | [e2e-scenario-contract.md](e2e-scenario-contract.md) | **Already decided**: a cross-platform contract, accepted 2026-10-02 |
| "Can a product step ever be auto-merged?" is open | `CLAUDE.md` tier table | **Already decided**: product steps stop at the PR |
| The unconditional security and privacy reviews run on docs-only changes | `step-loop` gate 4; `holy-blocker-security/references/review-triggers.md` | **Mostly false.** Docs-only work is loop-lite and never enters `step-loop`. Process-file steps do, and the skill already ends with "no security surface" when no row matches. The cost is a subagent spawn, not a wrong result. Unconditional was introduced because trigger matching let the agent decide a diff "obviously misses" |
| A launcher and tmux are available | `which tmux`; `ls scripts` | tmux is installed; no launcher exists. `claude` accepts an initial prompt, `--append-system-prompt`, `--settings` and `--permission-mode` |
| The weekly report is independent | `ledger.py`, ADR above | **False for step metrics.** `Step:` lines do not exist until `engineering.ledger-derived-status` lands, and 159 steps predate them. PR and worktree metrics do not depend on it |
| The mitm-proxy demo can prove steps 1–3 now | `ls demos tools/plan/e2e`; `docs/engineering/steps` | **Not yet.** There is no `demos/`, no runner, and `e2e-contract`, `e2e-runner`, `e2e-surfacing`, `ledger-derived-status` and `step-prose-files` are all pending |
| Master enforces any lock | `gh api repos/{owner}/{repo}/branches/master/protection` | **No.** HTTP 404, not protected. `.github/CODEOWNERS` has no entry for `tools/plan`, `.claude`, `demos` or `docs` |
| The repo has a stranded-work problem the design should address | `git worktree list \| wc -l` | **53 worktrees** |

## Decision

### 1. Four session types

| Session | Purpose | Ends with |
|---|---|---|
| plan | Decision record, step specs, the risk list, and for a demoable feature the case table | An open PR for owner approval |
| implement | One step, test-first | A PR, or a `blocked` or `failed` exit |
| review | The three reviews, from a cold context, over the branch's commits | Findings routed by `review-triage.md`, or a clean pass |
| demo | The owner meets the finished feature | The owner's verdict |

`loop route` keeps its job: given paths, say which loop and whether a step is required. It does not name a
session. The brief (decision 2) does.

### 2. The brief is generated, not committed

`python -m tools.plan.brief <step-id>` prints the step id, acceptance, verify command, governed paths, the
current base SHA, the audit commands from the step's spec, and the next session type. Pointers and commands,
never conclusions: the implementer re-runs every check. The tool exits non-zero on a broken input (missing
spec, unknown step, a dependency not done) and the phase stops. `brief --check <sha>` fails when the base has
moved since the brief was generated. Only the brief's inputs are committed: the step file, its
`steps/<id>.md` spec, and the case table.

### 3. Three exits, one budget, no empty commits

A phase ends `done`, `blocked` or `failed`.

- `done`: commits carrying a `Step: <id>` line, and a PR.
- `blocked` or `failed` with real commits: a `Phase-exit: blocked|failed <reason-code>` trailer on the last
  commit and a draft PR. `<reason-code>` is a closed vocabulary (stale-base, table-challenge, unverifiable-host,
  cap-hit, review-escalation, broken-tool). Free text stays in the terminal and the owner's session.
- `blocked` or `failed` with no code: the report is printed, nothing is stored, no branch is left behind.

One budget, the existing context gate (`tools/plan/context.py`, last-turn input tokens). Cumulative spend is
not capped: nothing needs it before the weekly report shows a cost problem, and a second budget is machinery
the agent that is being capped would also be the one counting. A retry cap is the existing `MAX_FIX_ATTEMPTS`.

Rejected: empty commits carrying a trailer (reproduced: `loop check` rejects an empty diff, so the PR is blocked
by CI), and free-text findings in a pushed commit body, which is a leak path the contract's closed payload list
forbids.

### 4. The owner gate: confirm once per feature, and say what is enforced

- A feature's demo is confirmed **once**, as a batch, per
  [e2e-scenario-contract.md](e2e-scenario-contract.md) §6. There is no per-step confirmation and no per-step
  `awaiting-owner` state. Product and observation steps stop at the PR exactly as today.
- The derived state is `awaiting-demo`: every step of a feature is `done` and the feature's scenario has no
  owner-confirmed note for the current covered-path tree hash. `todos` shows it and its age.
- The confirmation is a git note under `refs/notes/hb-e2e/<scenario-id>` on the tree hash, written by
  `python -m tools.plan.e2e confirm`, which refuses without a TTY. **This is a convenience, not a control.** An
  agent with a shell can forge a note with a pty or `git notes add`. The control the owner actually holds is the
  GitHub merge of a `product` or `observation` PR; nothing a note says lets such a PR merge itself. The residual
  stays recorded as the contract records it: evidence, not proof.
- Rejected: a SHA-bound confirmation (wrong key, above); a confirmation stored in the step file (a stored
  status, rejected by the derived-status ADR).

### 5. Which locks are real

The agent has shell, git, edit rights and, when auto-merge is used, authority to merge. A lock it can edit in
the same PR as the change it guards is advisory. Measured: master has no branch protection and CODEOWNERS covers
neither `tools/plan`, `.claude`, `demos` nor `docs`.

So the locks come in two strengths, and the plan names which is which:

| Lock | Mechanism | Strength |
|---|---|---|
| A step's `acceptance` may not change between base and head | `loop check` compares base and head step files | Advisory until protection |
| The demo case table may not change after the plan PR merged | `loop check` fails when the table differs from base on a PR that is not the plan's own; the table is written in the plan PR the owner approves | Advisory until protection |
| `loop check` and CODEOWNERS may not be weakened in the PR they gate | CI runs the checker from the base revision, and fails when CODEOWNERS loses a line | Real once the base checker is wired; the checker run from base cannot be edited by the head |
| Owner review on locked paths | CODEOWNERS entries for `/tools/plan/`, `/.claude/`, `/demos/`, `/docs/decisions/`, `/.github/` plus required code-owner review in branch protection | **Real only if the owner enables branch protection.** A repository setting, not a repo change; this decision cannot make it |

The case-table hash pin from the first draft is dropped. It amended an accepted contract and the base-revision
rule above gives the same guarantee for less.

### 6. The case table

`demos/<feature>/cases.toml`: inputs and expected verdicts, written by the plan session and approved with the
plan PR. Every blocked case has a control row, a clean input that must pass, which is the contract's
"assert both directions" rule. Assertions are on verdict fields and never on output wording, so wording
stays free. The implementer cannot edit the table; a disagreement is a `blocked` exit with reason
`table-challenge`, and a table-versus-code ambiguity goes to the owner. Fixtures are synthetic or benign only,
and the scenario's domains are RFC 2606 reserved names, per the contract §7.

### 7. Risk-first, held to its weight

A step spec carries a `## Risks` section: how this specific step could be wrong or harmful. The owner sees it in
the plan PR. Review lenses follow it, and the failure catalogue in `adversarial-review` stays as the floor,
because a risk list written by the model that wrote the plan shares its blind spots. A checker rejects an empty
or placeholder section; it cannot reject a boilerplate one, which is a recorded residual. It is a separate
section from `## Assumption audit`, which is per implementation PR, not per plan.

### 8. Review selection by touch is deferred, and fails closed when it comes

The unconditional security and privacy reviews stay. The measured cost is a subagent spawn per full-loop step,
not wrong results, and the unconditional rule exists to stop the agent judging its own diff out of scope. A
selector is revisited only if the weekly report shows the spawn cost mattering. If built it exempts only an
explicit never-security path allowlist (docs, tests without captured content, renderer styling), runs for every
unknown path, and evaluates the path-independent triggers (new process, new env-var read, new runtime
dependency) from the diff.

### 9. Measurement

- **Weekly report** (`engineering.weekly-report`): a local script printing PRs merged, time to merge (from `gh`),
  steps awaiting the demo and the oldest's age, bug steps with `regressed_step`, **worktrees open and the number
  with no PR** (the stranded-work number), and merged PRs with no `Step:` line. A metric with no source prints
  `no data`, never `0`; the last is listed so an agent cannot hide a step by not recording it. Nothing is
  committed. Step and demo metrics wait for `engineering.ledger-derived-status` and the first demo.
- **Transcript digest and session assessment** (`engineering.transcript-digest`,
  `engineering.session-assessment-skill`): kept in the plan, sequenced after the phase exits, and **the owner may
  drop them at approval**: the intent review found three sessions a week is too small a sample to carry a signal.
  If kept they must satisfy, as a precondition and unit test:
  - fail closed: any session whose raw bytes match an image or base64 marker, and any subagent file, is listed by
    id and counts only and is never sliced;
  - an allowlist of text-only fields (role, text, tool names), not a drop-list; no `cwd`, branch, attachment,
    snapshot or tool-input content;
  - usage deduplicated by message id and summed across subagent files;
  - the digest is written to the session scratchpad, mode 0600, and deleted after the assessment; a row for it is
    added to `privacy-review`'s `data-classes.md`;
  - the assessment session alone may read `~/.claude/projects`.

  Kill criterion for both: dropped if no process change came out of four weekly runs.

### 10. Deferred until a proving run

These wait for one real feature to go through the brief, the exits and the demo gate. Each has constraints the
red-team found, recorded here so they are not rediscovered:

- **Launcher** (tmux window in the worktree, interactive `claude`, phase prompt from the brief). Interactive,
  never headless, never `--dangerously-skip-permissions`. The owner starts a plan or a demo; implement then review
  chain without a keypress, because an owner trigger per phase is the babysitting
  [feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md) rejects. tmux `history-limit` small, cleared at
  session end, pane output never piped to a file.
- **Hooks and permission allowlist** (session-start brief injection, budget enforcement). A trust boundary: the
  template is written by the agent and **installed by the owner outside the repo**, as the privilege dispatcher
  is ([agent-privilege-boundary.md](agent-privilege-boundary.md) decision 7). The launcher ignores project-level
  hooks, `/.claude/` is under CODEOWNERS, and the implement and review sessions are denied keychain and
  `~/.claude/projects`. A pre-push `loop check` can check only the commit-message half of the contract; the PR
  body does not exist locally.
- **Browser launcher for the live check.** A TLS and CA step: full loop, with the security review. A throwaway
  profile in a `mktemp -d` directory with disk cache off and removed on exit and on a signal; a throwaway CA key
  mode 0600 in that directory, never in a worktree, scoped to that profile and never in the system trust store;
  benign inputs only and no listed domain ever fetched. Whether Chrome on macOS can trust a profile-local CA is
  unverified (it uses the Keychain); Firefox accepts a profile NSS database, and the repo has already verified
  Firefox 152 through the proxy.
- **The proving-ground feature.** Its own plan-inception, with the questions in "Escalated to the owner".
- **Retro.** After the proving run, record what the process cost and what it caught. Machinery that caught
  nothing is removed.

## Red-team findings

Three fresh-context reviewers. Findings that changed the design are marked.

| # | Lens | Finding | Evidence | Resolved by |
|---|---|---|---|---|
| 1 | bypass | CODEOWNERS is inert: no branch protection, no entries for the guarded paths (**changed the design**) | `gh api` 404; `.github/CODEOWNERS` | decision 5: locks labelled advisory or real; protection is an owner action |
| 2 | bypass | `loop check` is self-attesting; the PR that changes the check is checked by itself (**changed the design**) | `tools/plan/**` is in the same CI filter | decision 5: run the checker from the base revision |
| 3 | bypass | `acceptance` is agent-editable, so a `product` step can be downgraded to `code` (**changed the design**) | `ledger.py` step fields | decision 5: base-versus-head check |
| 4 | bypass | a TTY check does not stop a forged note | unverified | decision 4: stated as a convenience; the merge is the control |
| 5 | bypass | an unrecorded step never enters the derived state | unverified | decision 9: report lists merged PRs with no `Step:` line |
| 6 | bypass | an empty-commit exit is blocked by the empty-diff rule; free-text trailers are forgeable | `loop.py` empty-diff guard | decision 3 |
| 7 | bypass | project-level hooks load automatically | the repo ships `.claude/skills` | decision 10 |
| 8 | intent | per-step owner confirmation contradicts the one-demo model (**changed the design**) | feature-demos §4, contract §6 | decision 4 |
| 9 | intent | no merge-or-park rule and no stranded-work metric | 53 worktrees, reproduced | decisions 3 and 9 |
| 10 | intent | two budgets and draft PRs for blocked work add stranding (**changed the design**) | unverified | decision 3 |
| 11 | intent | assessment sample too small; demo and regression metrics have no data | unverified | decision 9: owner may drop; kill criterion |
| 12 | intent | the hash pin amends an accepted contract | contract decision 1 | decision 5: pin dropped |
| 13 | intent | no kill criteria | — | decision 10, retro |
| 14 | privacy | the digest cannot honour its image guarantee (**changed the design**) | 2 of 47 transcripts hold image blocks, 26 hold base64 | decision 9 preconditions |
| 15 | privacy | digest fields beyond text carry paths, branches, URLs | unverified | decision 9 allowlist |
| 16 | privacy | exit bodies and notes are pushed | contract §7 closed payload | decision 3 |
| 17 | privacy | profile cache and CA key handling unspecified | unverified | decision 10 |
| 18 | privacy | tmux scrollback; transcript read access by non-assessment sessions | unverified | decision 10 |

Disagreement recorded: the intent reviewer advised folding the risk list into `## Assumption audit`; kept
separate (decision 7) because the audit is per implementation PR and the risk list is read at plan approval.
The intent reviewer advised dropping the session assessment; kept as a droppable step because the owner asked for
it.

## Escalated to the owner

1. **Branch protection.** Enable it on master with required code-owner review, and widen CODEOWNERS to the paths in
   decision 5. Without it the first two locks are advisory. Recommended: yes.
2. **Keep or drop the transcript digest and session assessment.** Recommended: drop until the proving run shows a
   need; keep if you want the data from the first three steps.
3. **Which feature proves the process.** The input says mitm-proxy; the contract names the Android scenario as the
   first. mitm-proxy needs a macOS scenario with a CA and a browser, which the contract says is new coverage.
   Recommended: mitm-proxy, as its own plan-inception, with the case-table wording, the block-page text and whether
   the image case uses a stub decided there.
4. **Consent and disclosure for interception in the product.** A product question outside this change; handed to the
   proving-ground feature's inception. The demo on your own machine needs none.

Resolved by existing records, not re-asked: the demo standard is a cross-platform contract
([e2e-scenario-contract.md](e2e-scenario-contract.md)); a `product` step never auto-merges (`CLAUDE.md`).

## Rejected

- **Gate state stored in the ledger.** Contradicts [ledger-order-and-derived-status.md](ledger-order-and-derived-status.md).
- **SHA-bound confirmation.** A squash changes the SHA.
- **A committed brief with a base SHA.** Stale on the next master commit.
- **Per-step owner confirmation and an owner trigger per phase.** Reintroduces the babysitting that strands work.
- **A second, cumulative token budget now.** Counted by the agent it caps; no evidence it is needed.
- **Replacing the unconditional reviews with a selector now.** See decision 8.
- **Evals.** Dropped by the owner for compute.

## What this does not cover

- Forgery by an agent with a shell. Every note and trailer is evidence, not proof; the only controls are branch
  protection and the owner's merge.
- Building anything. See the "Session model and harness" section of `docs/engineering/plan.md`.
- The launcher, hooks, permissions, the browser launcher, review selection and the proving-ground feature (decision 10).
- Whether a hook can stop a tool call on a budget: unobserved here; the step that builds it audits it.
- Windows and Linux sessions; the sessions are written against macOS.
