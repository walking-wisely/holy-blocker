# ADR: The local e2e scenario contract

| Field | Value |
|---|---|
| **Status** | Accepted — design only; nothing here is built. Amended 2026-10-03: §3 and §4 superseded, see [Amendment](#amendment-2026-10-03) |
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

> **Superseded by the [amendment](#amendment-2026-10-03).** Kept for the reasoning; do not build it.

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

> **Amended:** the exit `2` rule below is replaced by the amendment's merge-versus-promotion rule.

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

## Amendment 2026-10-03

A red-team of this contract and the case-table idea ran three lenses (maintainer attention, engineering speed,
engineering quality) over four rounds. The agents were one model with different instructions and argued from a
brief, not from this ADR, so the result is a recommendation the owner has adopted, not independent review.

### A1. The claim is the unit

A claim is a `coverage.md` row. A check is anything that can observe it: a unit test, a hermetic e2e, a host e2e
or a human checklist. Evidence is a check's result with its provenance. Status is derived from evidence and
freshness and is never stored. The claim id is the `coverage.md` row id, so checks cite rows and growth is
additive. A scenario manifest is one kind of check, not the model.

### A2. Every check declares the lowest layer that can observe it

| Layer | Runs on | Evidence writer |
|---|---|---|
| `ci` | A hermetic runner: in-process CA, local test server, no TCC, no OS trust store, no browser | CI only |
| `local` | The dev machine, automated | Nothing recorded; a result is a diagnostic |
| `human` | The owner, from a checklist, once per feature | The owner's promotion PR |

A `ci` check never promotes a host-level row (TCC, system extensions, VPN consent, Keychain trust): a hermetic run
proves the code, not the host. Until about ten cases exist this is a review-checklist line, not a lint.

### A3. Git notes are removed

§3 is withdrawn. A note is local, forgeable by anything with a shell, and CI can only check that one exists. CI
reruns every `ci` check itself and is the only writer of evidence for that layer. The PR-body `<scenario-id> @
<tree-hash>` line is dropped with it. Host-layer rows carry no recorded pass; they stay `Unverified` until the
owner promotes them.

### A4. Merge and promotion are separate

Exit `2` still never counts as a pass. A step may merge on exit `2` only if every row it touches stays
`Unverified`, so Android, macOS and Windows work does not strand. `loop check` hard-fails any change that sets a
row to `Covered`, or removes it from `Unverified`, on exit `2` or on a non-CI result. Promotion is a separate
owner-owned PR, citing CI output for `ci` rows and the owner's checklist for `human` rows. The guard is "never
promotes", not "never merges". This replaces the unbounded merge on exit `2` in §4.

### A5. A case must fail when the code is broken

A passing case proves nothing if it would pass against broken code.

- Every case asserts verdict fields (rule id, reason code), not only the outcome.
- Every expected-fail test for a known gap asserts the specific wrong outcome (for plain HTTP: the request passed
  through unscanned with the body intact) and pairs with a positive control that succeeds through the same
  harness, so a connect or CA error cannot pass as a gap.
- The test client is independent of the code under test (rustls or curl, not the proxy's own helpers).
- Mutation testing uses the language's tool, not hand-written mutants: `cargo-mutants` for Rust, with Kotlin and
  others to follow with their own. It runs against the claim's own check only, scoped by the tool's file and
  function filters to the code the claim is about. A surviving mutant in scope fails the job, or is excluded in
  the tool's config and listed as a known survivor in `claim.md` with a reason. A committed patch file is allowed
  only for a mutant the tool cannot generate, with a one-line reason; none exists yet. A path-filtered CI job runs
  it only when a PR touches the crate or moves a `coverage.md` row. A "N mutants killed" line in a PR body that no
  job reproduces is not evidence.
- Measured on `mitm-proxy` (cargo-mutants 27.1.0, `-j2`, the dev machine): the scoped run is 116 mutants in
  about three minutes, 64 caught, none missed, 52 unviable. An unscoped first run over six files found 26
  survivors, two of them real gaps in the claim (the body-limit boundary), which became tests; the rest are the
  listed known survivors. The tool generated the mutants the gaps called for: the gate, the hook call sites and the
  leaf validity window. Runtime on a CI runner is not yet measured.
- No mutation work on shared crates (`text-policy`) until the CI cost is measured.

### A6. The owner approves a claim, not the rows

Each scenario has a short `claim.md`: what it proves, what it does not prove, and a three-minute owner checklist
whose items are falsifiable by looking. `does_not_prove` names unproven consumers explicitly (a `mitm-proxy` pass
says nothing about the Android and macOS consumers of `text-policy`) and each entry names an existing
`Unverified` row id. The owner approves that paragraph. Fixture rows are checked mechanically, because the owner
cannot audit them given [image-corpus-custody.md](image-corpus-custody.md). The PR template shows the count of
`Unverified` rows.

### A7. When a feature needs a case

A feature that adds or moves a `coverage.md` row, or touches preload/IPC, TLS/CA, TUN, named pipes, capture/OCR
or OS permissions, needs at least one case, and the case is what justifies moving the row. The requirement
attaches to the feature, not the step. Not required: docs, typo fixes, refactors whose `covers` already match a
scenario, and pure logic a unit test fully observes.

### A8. Deferred until a second consumer exists

The `scenario.toml` schema in §1, a runner, the `layer` enum as a manifest field, the claim lint, per-case `demo`
markers, the instrumented-coverage diagnostic of `covers`, and signing or attestation of evidence. The first
consumer is a hermetic `mitm-proxy` scenario; mobile is the second. `platform` in §1 should become a declared
capability list when a host that is not Android, macOS or Windows appears.

### Amendment: what this does not cover

- Host-layer rows (TCC, consent dialogs, Keychain trust) get no cheaper. They are owner-attested or `Unverified`.
- ML evaluation rows stay capped at `Unverified` or owner-attested; the data is legally unavailable.
- Forgery of owner promotions. The owner's own review and merge remain the only control.
