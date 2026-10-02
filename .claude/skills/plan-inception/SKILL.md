---
name: plan-inception
description: >
  Conceive a new feature or plan before any implementation: turn a product idea
  into a decision record plus an executable plan.md that the step-loop can run.
  Use when a user names a new feature, asks to start building something that has
  no plan yet, or a package's `ledger next` reports "none" but the user still has
  work in mind. This is the front of the pipeline — the step-loop only executes
  plans that already exist, so new work must pass through here first.
  Triggers on "new feature", "add a feature", "plan this out", "build X from
  scratch", "what would it take to do Y", "write the plan for", or any task where
  there is no plan.md / no steps.toml yet for the thing being asked.
---

# Plan inception

The step-loop executes a plan; it does not invent one. This skill is the part that
invents. It takes a product impulse and turns it into (a) a decision record where
yours is the deciding voice, and (b) a `plan.md` the ledger can extract so the loop
can run it.

The rule that keeps this honest: **plans are product and architectural decisions,
owned by the human, not engineering artifacts the agent makes up.** An agent must
never silently decide the CSAM boundary, a protection-mode, a verse policy, the
privacy tradeoff, or acceptance kind on the user's behalf.

## The doctrine index — the reference the audit checks against

A plan's non-engineering claims are not verified; they are **traced**. The places
their truth lives, in dependency order:

| Doctrine domain | Index — where the truth is recorded | What a claim must cite to pass |
|---|---|---|
| **faith** | `docs/mission.md` (covenant rationale, "what we block", accountability, local-first/privacy) | a named `mission.md` line or section |
| **product** | `docs/decisions/*.md` ADRs, `docs/product/outcomes.md`, `docs/product/flows/*` | a named ADR, an outcome row, or a flow trace |
| **legal** | existing ADRs (`domain-blocklist-sourcing.md`, `content-interception.md`), statutes, platform ToS | a named rule + citation/link, or an ADR that already records it |
| **engineering** | the codebase + the outside world | **observation**: one falsifying command that proves it |

Read the relevant index entries **before** drafting. Never cite a doctrine domain
without having opened its reference.

## Step 1 — Classify every claim into a domain

Write the plan's load-bearing claims down and tag each: `faith`, `product`,
`legal`, `engineering`. Four rules make this non-negotiable:

1. Every claim gets exactly one tag. A claim you cannot tag is an incoherent plan.
2. `faith` and `legal` claims are **not** verified by reasoning. They are either
   traced to the index or escalated. Attempting to "reason out" a theological or
   legal proposition is silent policy invention — reject it.
3. `engineering` claims demand **one falsifying command each** (the mechanism in the
   `assumption-audit` skill — reuse it here at plan scope).
4. **Nothing is unowned.** Every claim either has a recorded owner (a citation) or
   becomes an escalation that *you* resolve. After you resolve it, the outcome is
   written back so it is owned forever after.

## Step 2 — Route each claim to PASS / ESCALATE / FAIL

The audit's output is a contract, not a vibe:

- **`PASS`** — engineering claim falsifying command ran and held; doctrinal claim
  has a valid index citation, and the index was actually read.
- **`ESCALATE`** — a doctrinal implication (`faith` / `product` / `legal`) with no
  recorded owner, or an engineering claim that can't be observed *here*. This is not
  an error; it is the skill's job. Hand it to the user with the concrete question.
  **Never resolve an `ESCALATE` yourself.** Your decisions are the doctrine.
- **`FAIL`** — an engineering fact is false, or a citation points at an index entry
  that says the opposite. Stop; do not draft on a false premise.

The skill is only "done with the audit" when every claim is `PASS` or has gone
through `ESCALATE → user decision → written back as PASS`.

## Step 2.5 — Tier the decisions and red-team them

Before an escalation is put to the user, classify each decision as **product**,
**architecture** or **implementation** and give it a red-team tier, per
`docs/decisions/decision-tiers-and-red-teaming.md`. Write the classification next to the
claim so the user can overrule it.

- Implementation: decide it, record it in the plan, do not escalate.
- Architecture: escalate only if it is ML, changes a contract two or more platforms consume,
  adds or moves a trust boundary, or is a one-way door. Otherwise decide and record it.
- Product: always escalate.

Red-team depth:

- **Tier 0** — nothing.
- **Tier 1** — one fresh-context subagent on the single most relevant lens.
- **Tier 2** — three fresh-context subagents in parallel, one per lens: **bypass**,
  **intent**, **legal and privacy**.

Tier 1 and tier 2 spawns are pre-authorised by that ADR; do not stop to ask under
`confirm-before-subagents`. Findings follow `adversarial-review`'s rules (reproduce or
label, cite or omit) and are folded into the ADR's rejected alternatives and
"what this does not cover". Present them to the user as a short list in the escalation
packet format from `docs/engineering/review-triage.md` §4, then take the decision.

## Step 3 — Resolve escalations by writing truth back

For each escalation the user decides, record it in the index **before** drafting the
plan, in dependency order:

- new decision → new ADR in `docs/decisions/<slug>.md` and a row in
  `docs/decisions/README.md`;
- new/changed capability → a `docs/product/outcomes.md` row and a flow in
  `docs/product/flows/`;
- new coverage → a `docs/engineering/coverage.md` row (planned scope);
- faith implication → a `docs/mission.md` note.

This is the accuracy mechanism: **every decision you make converts a future
`ESCALATE` into a now-checked `PASS`.** The index grows, and the audit gets cheaper
and more precise each time.

## Step 4 — Draft the plan, ledger-compatible

Write `docs/components/<package>/plan.md` following the repository convention so the
ledger can extract it:

- `## Implementation order`, one item per step, numbered;
- one `<!-- step: <package>.<step-id> -->` marker per step;
- `**Done.**` / `~~struck~~` markers for finished items;
- a `## What this does not cover` section that records deliberate narrowing;
- keep "module N" style if the package already uses it (check the existing plan).

Then generate the ledger from it:

```
python -m tools.plan.ledger validate docs/components/<package>
python -m tools.plan.ledger next     docs/components/<package>
```

`validate` must pass and `next` must report the first step before this is done.

## Step 5 — Choose the verify command and acceptance kind

Per step, in the manifest:
- `acceptance = "code"` — done-ness is a claim about the diff; the loop may
  squash-merge on green CI.
- `acceptance = "observation"` — done-ness is a claim about the world; the loop
  stops at the PR and you confirm against `coverage.md` / `outcomes.md`. This is the
  **default** for steps whose truth is an observation, not a review.
- `verify` — the narrowest command that settles the step. A step that has no honest
  verify command is a warning sign about the plan, not about the tooling.

Pick these with the user when the plan implies them. They are commitments.

For a feature that a person will use, end the plan with a `demo` step: an e2e scenario that
runs headless as a regression check and with `--interactive` as the owner's demo, per
`docs/decisions/feature-demos-and-local-e2e.md`. Declare in the plan which behaviours are
proved at the unit, integration and e2e layers; anything an e2e scenario can observe should
not rely on `acceptance = "observation"`.

## Step 6 — Publish the plan, then hand to the loop

The decision records, index rows and `plan.md` this skill writes are docs, and they follow
the repository's Working Rhythm in `CLAUDE.md` like any other change: write them in their
own worktree, and once `validate` and `doclint` pass, commit, push and open the PR in one
pass without asking. The only exception is a request to hold the change before it goes out.

The skill stops there. It does **not** implement. Once `validate` passes and `next` reports
a step, the work belongs to `step-loop` (audit its external claims per-step, test-first,
adversarial review, merge). Do not run the loop's gates from inside this skill — that is the
loop's job, in a fresh context.

## What this skill must not do

- Invent `faith`, `product`, or `legal` policy and cite nothing.
- Resolve an `ESCALATE` itself. The user owns the doctrine.
- Promote an `Unverified` engineering claim to `Covered` in `coverage.md` by
  argument — only an observation does that.
- Write `plan.md` before the escalated decisions are written back to the index.
- Implement. This skill conceives and publishes the plan as a docs PR; `step-loop` builds.
- Hold the plan back from a PR to wait for permission. The owner reviews decisions in the PR.
