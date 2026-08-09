---
name: adversarial-review
description: >
  Adversarially review a diff, branch, PR, or module in Holy Blocker against this repository's
  recorded failure modes — guessed constants, silent fail-open, plan-versus-world mismatches,
  scope narrowed to what is testable, and done-at-the-module-boundary. Use when asked to review,
  critique, or pressure-test written code, a PR, or a completed module. Triggers on "adversarial
  review", "review this PR", "what did we miss", "is this actually done", "pressure-test this",
  "find the holes", or a request to run a review subagent over a package.
---

# Adversarial Review

Review written code against the failure modes this repository has actually produced, not against a
generic checklist. The catalogue is in `references/failure-catalogue.md`; the package-specific
traps are in `references/<package>.md`.

For claims about the outside world that a *plan* makes, use `assumption-audit` instead — it runs
before implementation, where those findings are twenty commits cheaper.

## The two rules that make a review trustworthy

**Reproduce or label.** Every finding is either reproduced by running something — a test, a scratch
`examples/` binary, a `dig`, a `codesign`, a device command — or it is explicitly marked
`UNVERIFIED` with the reason. Both outcomes are acceptable. A finding presented as certain that was
never run is not.

**Cite or omit.** Every quotation from a file in this repo is copied from the file, and every
specification reference names the document and section. A review in this project has already
attributed a quotation to `docs/decisions/classifier-operating-point.md` that does not appear in
that file. If a citation cannot be checked, drop the claim rather than paraphrase it into one.

Do not soften findings, and do not inflate them. Rank by what breaks, not by how much was written.

## Step 1: scope and read

Establish exactly what is under review — a diff, a branch's full commit range, a PR, or a package.
Read **all** of it. When reviewing a PR, read every commit on the branch, not the first: fix rounds
change the thing being reviewed, and a review of the opening commit reports findings that were
already addressed.

Check where the work actually lives before reporting on it. Branches in this repository frequently
sit unmerged and far behind `master`; a finding about "the code" that describes a branch nobody has
merged should say so.

## Step 2: run the catalogue

Read `references/failure-catalogue.md` and work each entry against the change. It is ordered by
how often each has occurred here, and the first three account for most of the real defects.

Then load the package reference for whatever the change touches:

| Path touched | Reference |
|---|---|
| `packages/domain-blocklist`, `packages/net-shield*`, `apps/mobile/**/NetworkGuard*` | `references/dns.md` |
| `native-modules/mac-daemon` | `references/macos.md` |
| `apps/mobile` (guards, services, accessibility) | `references/android.md` |
| `packages/image-sandbox*`, `packages/classifier-head`, `machine-learning` | `references/models.md` |

Security-boundary questions — parsers, the local CA, IPC, privileged processes — belong to
`holy-blocker-security`. Invoke it rather than duplicating its rules here.

## Step 3: ask the two questions no module asks itself

These produce the findings this repo has historically missed, and they are not in any per-file
checklist.

**What is now uncovered that a user would expect covered?** Every module here is scoped honestly
and narrowly, and each narrowing is recorded as a note in its own row. Nobody computes the union.
The union is the product. Name the real-world case that falls between this module and its
neighbours — an unfocused window, a plain-HTTP request, a DoH query — and state plainly whether
anything catches it. If `docs/engineering/coverage.md` exists, check the change against it and say
which rows move.

**Is this done at the capability boundary or the module boundary?** "20 tests, module complete"
while the two ends of the pipeline have never met is not done. Ask what a user-visible statement of
this work would be, and whether anything has demonstrated it.

## Step 4: report

If `ReportFindings` is available, use it, most severe first, with `verdict` set from the reproduce
step. Otherwise report ranked prose. Either way each finding carries:

- what breaks, as a concrete failure scenario with inputs and the wrong result;
- `file:line`;
- whether it was reproduced, and by what command;
- severity driven by consequence — **a guard that fails open silently outranks a crash**, because a
  crash is reported and an open guard reads as a working one.

State the count of findings you could not reproduce. A review that reports only what it proved,
and says how much it could not reach, is worth more than one that reports everything with equal
confidence.

## Step 5: fixes are a separate pass

Do not fix during the review. If fixes are requested afterwards, one finding per commit, and
re-run the review's own reproduction command against the fix — a fix verified only by the test that
was written alongside it is verified by its author's understanding of the bug.
