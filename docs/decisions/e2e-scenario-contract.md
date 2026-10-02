# ADR: The local e2e scenario contract

| Field | Value |
|---|---|
| **Status** | Accepted — design only; nothing here is built |
| **Date** | 2026-10-02 |
| **Owner** | Ivan Dutov |
| **Supersedes** | — |
| **Superseded by** | — |

---

## Context

[feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md) decided that regression runs locally, that a
demo is a scenario with an interactive tail, and that a runner maps changed paths to scenarios, separates
*environment not ready* from failure, and reports staleness. It left the contract unwritten, and a contract
that Android, macOS and later Windows scenarios all consume is a cross-platform architecture decision
([decision-tiers-and-red-teaming.md](decision-tiers-and-red-teaming.md)).

Two rules pull against each other. `coverage.md` wants `Covered` to point at a dated observation, and the
step loop derives status from git and `gh` and never stores it. A pass is an observation on one machine, and CI
cannot reproduce it.

The existing de-facto contract is `apps/mobile/scripts/smoke-test*.sh`: a last line of `SMOKE PASS` or
`SMOKE FAIL: <why>` and `exit 1`. It has no preflight, no declared coverage and no record.

This decision was red-teamed at tier 2, one fresh-context reviewer per lens. The findings that changed the design
are listed under each decision; those that needed the owner are under "Decided by the owner".

## Decision

### 1. A scenario is a manifest plus an entry point

`demos/<feature>/scenario.toml` declares:

| Field | Meaning |
|---|---|
| `id` | Stable scenario id, cited from `coverage.md` |
| `platform` | `android`, `macos` or `windows` |
| `covers` | Repo path prefixes whose change makes this scenario's result stale |
| `proves` | `coverage.md` row ids this scenario can observe |
| `does_not_prove` | Free text: what a pass here says nothing about (vendor skins, real devices, DoT) |
| `preflight`, `run`, `interactive` | Commands. `interactive` is mandatory on a feature's final scenario |

The runner finds manifests by globbing; there is no central path map. Existing smoke scripts are wrapped by a
manifest, not moved.

`covers` is checked, not trusted: a doclint check fails when it is not closed under `cargo metadata` path
dependencies of the crates it names, or omits the scenario's own script and seed paths. An under-declared
`covers` is a false `Covered`.

### 2. Exit codes and the pass token

`0` pass, `1` fail, `2` environment not ready. `run` never returns `2`; `preflight` may only return `2` for a
declared capability (SDK, emulator, grant, signing identity), and any build failure is `1`. The runner tries to fix
the environment first (boot the declared AVD) before it reports `2`, and always names the missing capability.

The runner, not the script, decides a pass: exit `0`, plus at least one blocked and one permitted assertion
reported as `ASSERT blocked <n>` / `ASSERT permitted <n>`. A script printing a PASS line does not count. A
scenario passes only if it asserts both directions, as the Android gotchas already require.

### 3. The pass record is shared through git notes

A pass is recorded as a git note on the **tree hash of the scenario's `covers` paths**, in
`refs/notes/hb-e2e/<scenario-id>`. The tree hash is stable across unrelated commits, rebases and squash merges,
which commit SHAs are not. The note carries: result, UTC date, host class (for example `android-36 arm64 AVD`),
and the SHA-256 of the built artifact. It is pushed with `git push origin refs/notes/hb-e2e/*` and fetched with
`git fetch origin 'refs/notes/*:refs/notes/*'`.

Verified here: a note on a tree object survives an unrelated commit, an amend, a push and a fresh clone.

- **Shared, not local.** Every collaborator sees the same record. Nothing is committed to a branch, so nothing
  conflicts or goes stale in a PR.
- **The runner builds, or records the artifact digest.** A pass against an APK built from older sources must not be
  recorded against the current tree.
- **It refuses to record on a dirty covered tree.** `git rev-parse HEAD:<path>` ignores uncommitted edits.
- **A forged note is possible.** Anyone with push access can write one. The note is attributable in git, and the
  contract states that the record is evidence, not proof. An empty stale list is never a merge gate.

The PR body carries one line, `<scenario-id> @ <tree-hash>`, and `loop check` recomputes the tree hash for the PR's
head and requires a matching note. This catches a claim for code that has since changed and a claim with no run
behind it. It does not catch someone who writes both the note and the line.

### 4. The gate reports what it did not cover

`tools/plan/e2e run` selects scenarios whose `covers` intersect the diff and prints, every time, `N changed paths
covered by no scenario`. Zero scenarios selected is reported as *unverified*, never as success.

An exit `2` on a step with `acceptance = "code"` lets the code merge. The PR records the missing capability, no
coverage row is promoted, and the debt appears in `e2e stale`. This avoids the stranded-branch pattern that
[feature-demos-and-local-e2e.md](feature-demos-and-local-e2e.md) rejects owner sign-off at every step for.

### 5. Staleness is surfaced where work already happens

`e2e stale` reads the notes and lists scenarios whose covered-path tree hash has no passing note. It is shown in
three places so that it is not remembered but met:

1. `step-loop` gate 0 runs it at the start of every step, and gate 6 before a merge.
2. `python -m tools.plan.todos` lists stale scenarios beside pending steps.
3. A scheduled run on the dev machine is a separate, optional decision and is not part of this one.

Ignoring a stale scenario is safe: rows stay `Unverified`. The cost is that coverage does not improve.

### 6. Promotion to Covered is the owner's

The loop batches one promotion request per PR, one line per row: the scenario, the assertions it made, and the host
class. The owner confirms or rejects them as a group. Only rows named in the scenario's `proves` can be requested,
and a pass on an emulator for a route that names a real device is requested as `Partial`. `coverage.md` records this
as rule 6.

### 7. Privacy rules the runner enforces

These follow from [image-corpus-custody.md](image-corpus-custody.md),
[domain-blocklist-sourcing.md](domain-blocklist-sourcing.md) and the local-first rule in `CLAUDE.md`.

- Scenarios and the runner never persist pixels, screenshots or UI-tree dumps on the host, and delete device-side
  dumps in teardown. Only scores, verdicts and fixed-vocabulary reasons are recorded.
- Image scenarios assert only the negative direction: wiring and plausible scores on benign generated images.
  They never assert a block on real content, never fetch fixtures at run time, and never commit a personal photo.
- Scenario domains are RFC 2606 / 6761 reserved names. The blocked set comes from the scenario's own seeded rule
  file; a real listed domain is never resolved or fetched.
- The PR payload is a closed list: scenario id, result, tree hash, UTC date, host class. No hostname, username,
  absolute path or log text. `FAIL: <why>` is a bounded fixed vocabulary, never a logcat line or scanned text.
- The runner captures scenario stdout and pastes only the final line, so a scenario that sees a signing identity
  or credential cannot echo it into a transcript.
- Notes are append-only and content-free. Hosted device farms and any cloud runner are rejected on privacy grounds,
  not only on cost.

## Decided by the owner

| Question | Decision |
|---|---|
| Where a pass is recorded | Shared with collaborators, not one machine. Git notes keyed on the covered-path tree hash, per §3 |
| Exit `2` on a `code` step | Merge; rows stay `Unverified` |
| Who promotes a row to `Covered` | The owner confirms every promotion, batched per PR |

## Rejected

- **A committed results file.** It conflicts between branches, goes stale in a PR and is the stored state the loop
  avoids.
- **A local-only results file.** Only the machine owner could see staleness; rejected by the owner.
- **GitHub commit statuses.** Shared, but they need `gh` write access, which this machine's account did not have
  when `docs/engineering/plan.md` last checked, and they key on commit SHAs that a squash merge changes.
- **Trusting the script's PASS line.** A `run` of `echo SCENARIO PASS` passes; the runner decides.
- **Loop-promoted `Covered`.** Reviewers showed a scenario can pass while the row claims more than it observed.
- **A central path map.** It drifts from the scenarios; the cost is the fail-open gap closed in §4.

## What this does not cover

- Forgery. Anyone with push access can write a note. The record is evidence, not proof, and the trusted-host
  question is deferred.
- Real devices and vendor skins. A pass says "verified on an emulator" with its API level, never "works on Android".
- The owner's feel for a feature. `interactive` and the "last watched by a human" stamp are tracked apart from machine
  passes, and a green headless run is never read as the feel having been checked.
- macOS and Windows scenarios. The contract is written to fit them; none is specified here.
- Unattended runs. Nothing runs scenarios on a schedule.
- Whether CI runners with a GUI session or an emulator are now usable. Not checked.
