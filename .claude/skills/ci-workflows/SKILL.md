---
name: ci-workflows
description: >
  Write, review, and fix GitHub Actions workflow files following established best practices.
  Use this skill whenever the user asks to create a new CI/CD workflow, audit or review an
  existing workflow file, fix a workflow bug, add a new job or step, or improve a workflow
  in any way. Also trigger when the user pastes a workflow and asks "what's wrong with this",
  "can you improve this", or "does this look right". Covers Node/pnpm, Rust/Cargo, Python,
  and generic workflows.
---

# GitHub Actions CI Workflow Best Practices

When writing or reviewing any `.github/workflows/*.yml` file, apply every item in this checklist. These aren't style preferences — each one has caused real CI failures or supply-chain incidents.

---

## 1. Pin all actions to immutable commit SHAs

Tags like `@v4` are mutable — a maintainer can silently move them to a different commit. Always pin to a full commit SHA and add a comment showing the human-readable version so future readers (and Dependabot) know what it resolves to.

```yaml
# Wrong
- uses: actions/checkout@v4

# Right
- uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4
```

If you don't know the current SHA, use the GitHub API:
```bash
gh api repos/<owner>/<repo>/git/ref/tags/<tag> --jq '.object.sha'
# If the result type is "tag" (annotated tag), dereference it:
gh api repos/<owner>/<repo>/git/tags/<sha> --jq '.object.sha'
```

For actions that don't use version tags (e.g. `dtolnay/rust-toolchain@stable`), pin to the HEAD commit of the branch:
```bash
gh api repos/dtolnay/rust-toolchain/git/ref/heads/master --jq '.object.sha'
```

**Dependabot keeps SHAs current automatically** — it opens PRs when pinned SHAs need updating, so maintenance cost is low. Without SHA pinning, Dependabot can't detect a tag that was silently moved to a malicious commit (zero-day tag mutation).

---

## 2. Include the workflow file itself in path filters

When using `paths:` to filter when a workflow runs, always add the workflow's own path to the list. Without it, changes to the workflow file itself won't trigger a run — you can't validate workflow edits, including the PR that first adds the workflow.

```yaml
# Wrong — workflow changes never trigger CI
on:
  pull_request:
    paths:
      - "src/**"

# Right
on:
  pull_request:
    paths:
      - ".github/workflows/ci.yml"
      - "src/**"
```

Apply this to both `pull_request` and `push` trigger blocks.

---

## 3. Install package managers before setup steps that depend on them

`actions/setup-node` with `cache: pnpm` calls `pnpm` to resolve the store path. If pnpm isn't installed yet, the caching step silently fails or errors. Always install the package manager first.

```yaml
# Wrong — setup-node runs before pnpm is available
- uses: actions/setup-node@...
  with:
    cache: pnpm
- uses: pnpm/action-setup@...
  with:
    version: 9.15.0

# Right — pnpm first, then node with cache
- uses: pnpm/action-setup@...
  with:
    version: 9.15.0
- uses: actions/setup-node@...
  with:
    node-version: 22
    cache: pnpm
```

The same applies to Yarn (`cache: yarn`) — install Yarn before setup-node if you're using a custom version.

---

## 4. Use `--frozen-lockfile` (or equivalent) when installing dependencies

In CI, dependency installs should be deterministic. Never let the lock file be updated silently.

```yaml
- run: pnpm install --frozen-lockfile   # pnpm
- run: npm ci                            # npm (always frozen)
- run: yarn install --frozen-lockfile   # yarn classic
- run: yarn install --immutable         # yarn berry
```

---

## 5. Add `concurrency` groups to cancel redundant runs

Without concurrency groups, pushing two commits quickly will run CI twice on the same branch. The first run is wasted.

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

Use `cancel-in-progress: false` on the default branch if you need every push to complete (e.g., deployment workflows).

---

## 6. Declare minimum permissions; don't rely on defaults

The default `GITHUB_TOKEN` permissions are broad (write on most scopes in many repo configurations). Declare only what the job needs.

```yaml
permissions:
  contents: read      # for checkout
  # add others only as needed:
  # pull-requests: write   # to post comments
  # packages: write        # to push to GHCR
```

Put `permissions` at the job level, not the workflow level, so each job is independently scoped.

---

## 7. Set timeouts on jobs

Jobs that hang (flaky tests, network waits, infinite loops) consume runners indefinitely without a timeout. Set a reasonable ceiling.

```yaml
jobs:
  build:
    timeout-minutes: 30
```

Typical values: 10–15 min for fast CI, 30–60 min for integration/build-heavy workflows.

---

## 8. Use `fail-fast: false` for matrix builds when diagnosing failures

The default `fail-fast: true` cancels remaining matrix entries as soon as one fails. This is usually fine for CI, but set it to `false` when you want to see failures across all matrix combinations at once (useful for cross-platform or multi-version matrices).

```yaml
strategy:
  fail-fast: false
  matrix:
    os: [ubuntu-latest, windows-latest]
```

---

## 9. Cache Cargo artifacts correctly for Rust

Always scope the cache to the workspace being built, not the repo root (unless all packages are in one Cargo workspace at the root).

```yaml
- uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32 # v2
  with:
    workspaces: packages/my-crate   # path to the Cargo.toml directory
```

For a matrix over multiple crates, use `${{ matrix.package }}`.

---

## Checklist for every workflow

Before finishing, verify:

- [ ] Every `uses:` line is pinned to a full SHA with a `# version` comment
- [ ] The workflow file's own path is in every `paths:` filter block
- [ ] Package managers are installed before setup steps that cache them
- [ ] Dependencies are installed with `--frozen-lockfile` or `npm ci`
- [ ] A `concurrency` group is defined
- [ ] `permissions` is declared at the job level with only required scopes
- [ ] `timeout-minutes` is set on every job
- [ ] Matrix builds use `fail-fast: false` where full coverage is needed

---

## Common action SHAs (as of mid-2025 — always verify with `gh api`)

Provided for reference only. Always verify before use — these change as new versions release.

| Action | Tag | SHA (verify!) |
|---|---|---|
| `actions/checkout` | v4 | `11bd71901bbe5b1630ceea73d27597364c9af683` |
| `actions/setup-node` | v4 | `49933ea5288caeca8642d1e84afbd3f7d6820020` |
| `pnpm/action-setup` | v4 | `b906affcce14559ad1aafd4ab0e942779e9f58b1` |
| `Swatinem/rust-cache` | v2 | `e18b497796c12c097a38f9edb9d0641fb99eee32` |
| `dtolnay/rust-toolchain` | stable (HEAD) | `3c5f7ea28cd621ae0bf5283f0e981fb97b8a7af9` |
