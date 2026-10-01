# Evaluation and CI

Reliable evaluation is required before tuning the image model, OCR providers, or text policy rules. The project should avoid tuning against guesses alone.

## Eval Layers

Use four layers of evaluation:

```text
1. Unit tests
   normalization, tokenization, matching, scoring

2. Golden text evals
   labeled safe, block, and ambiguous examples

3. OCR screenshot evals
   generated images with expected OCR text and expected decision

4. End-to-end daemon evals
   controlled windows/pages, foreground capture, OCR, classification, latency
```

## Text Eval Cases

Golden text eval rows should explain why they exist:

```json
{"id":"en_block_0001","text":"redacted example","label":"block","category":"direct","language":"en","reason":"direct explicit phrase"}
{"id":"en_allow_0001","text":"redacted example","label":"allow","category":"medical","language":"en","reason":"safe educational context"}
```

Important categories:

- direct blocked words and phrases;
- slang and euphemisms;
- misspellings and OCR-like mistakes;
- leetspeak and separator evasion;
- mixed case and homoglyphs;
- translations;
- safe medical, educational, religious, and news contexts;
- harmless substrings inside safe words;
- ambiguous titles, acronyms, jokes, and quoted policy text.

## OCR Screenshot Evals

Synthetic screenshots make OCR tests repeatable without collecting sensitive real browsing content.

Generate cases across:

- normal and small font sizes;
- light and dark mode;
- high and low contrast;
- browser-like layouts;
- noisy backgrounds;
- scaled display settings;
- multiple scripts and languages.

The OCR eval path should be:

```text
image
  -> OCR provider
  -> normalized text
  -> text policy
  -> expected decision
```

## Metrics

Track at least:

```text
precision
recall
false positive rate
false negative rate
OCR extraction accuracy
p50 latency
p95 latency
```

Support multiple threshold profiles:

```text
strict: higher recall, more false positives
balanced: default
light: higher precision, fewer false positives
```

## CI Tiers

Use three CI tiers:

```text
Tier 1: public CI
  - runs on every pull request
  - uses sanitized fixtures committed to the repo
  - safe for forks

Tier 2: private CI
  - runs only with trusted secrets
  - downloads full private eval packs
  - checks precision, recall, and latency

Tier 3: release or nightly eval
  - larger corpus
  - OCR screenshot suites
  - performance benchmarks
  - comparison against previous release
```

## Current Tier 1 Coverage

The public Tier 1 workflow is intentionally narrow for the current state of the repository:

```text
GitHub Actions workflows: ci.yml (entry point), ci-js.yml, ci-rust.yml, ci-plan.yml,
ci-advisories.yml, codeql.yml

- ci.yml runs on every pull request and push to master. It calls ci-js, ci-rust and
  ci-plan as reusable workflows and ends in a single `CI gate` job that fails if any of
  them failed or was cancelled and accepts skipped. Require `CI gate` in branch
  protection; the individual jobs are path-filtered and may legitimately not run.

- ci-js (apps/desktop and the pnpm workspace):
  - pnpm lint, pnpm typecheck, pnpm build

- ci-rust (every crate under packages/, one matrix leg per affected crate):
  - cargo clippy --all-targets with -D warnings
  - cargo fmt --check on the crates listed in `FMT_CLEAN` in the workflow
  - cargo test --locked on the toolchain pinned in rust-toolchain.toml
  - cargo deny against deny.toml: RustSec advisories, a licence allow-list, wildcard
    dependencies, and crates.io as the only registry
  - for the three *-ffi crates, a `--features bindgen` build that runs `uniffi-bindgen`
    for Kotlin and Swift against the host cdylib and fails if either errors
  - a crate is affected when it or any path dependency changes, e.g. a change
    to domain-normalize also runs domain-blocklist, net-shield and net-shield-ffi

- ci-plan:
  - tools/plan unit tests and ledger validation
  - secret-scan (gitleaks)

- ci-advisories (weekly and on demand): cargo deny advisories for every crate, so a new
  RustSec entry surfaces without waiting for a code change. Not part of the gate.
```

Each job runs only when its paths change. `ci-plan` also exposes `workflow_dispatch`.

`rustfmt` is enforced only on crates that are already formatted. text-policy,
domain-blocklist, net-shield, image-sandbox and mitm-proxy are not rustfmt-clean:
formatting them is a repository-wide mechanical change that would conflict
with every open branch, so it is deferred until the branch queue is drained. Add each
crate to `FMT_CLEAN` in the same change that formats it.

`deny.toml` decisions an owner may want to revisit:

- MPL-2.0 is allowed because the UniFFI crates carry it. It is file-level copyleft and
  the crates are used unmodified.
- The workspace crates are `publish = false` so the licence check skips them; the
  project licence is not an SPDX expression.
- RUSTSEC-2025-0141 (bincode 1.3.3 unmaintained, no vulnerability) is ignored with a
  reason in `deny.toml`.
- The graph is checked with default features. The `onnx` feature of image-sandbox and
  image-sandbox-ffi pulls `ort`; neither `cargo deny` nor clippy covers it.
- cargo-deny reads the RustSec database from GitHub at CI time. This is a CI-only
  network call; no runtime path is affected.

Branch protection for `master` (repository owner to apply; not enabled by a workflow):

- Require a pull request before merging, with at least one approval.
- Require status checks to pass, with only `CI gate` selected, and require branches to
  be up to date.
- Optionally require review from code owners (`.github/CODEOWNERS`). Every entry is
  `@walking-wisely`, the repository owner and the only collaborator; GitHub does not
  count an author's approval of their own PR, so this blocks every PR until a second
  maintainer exists or the owner bypasses it. Replace the handle with a team when one does.
- Block force pushes and deletion.

Not covered yet: Kotlin, Swift and C++ builds, native Windows and macOS runners, and the
ONNX inference and parity tests, which skip without the gitignored model.

## Private Eval Packs

Do not store large explicit blocklists, sensitive multilingual rules, or full OCR screenshot corpora in the public repository.

Recommended layout:

```text
Public repo:
  docs, schemas, sanitized fixtures, eval runner

Private object storage:
  versioned eval archives

Private metadata repository:
  manifests, labels, review notes, changelog
```

Cloudflare R2 is a good candidate for private eval archives because it is S3-compatible and has favorable egress costs.

Example storage layout:

```text
holy-blocker-private-evals/
  text-policy-eval/
    v2026-05-31/
      manifest.json
      text-eval.tar.zst
      screenshot-eval.tar.zst
      checksums.txt
```

CI should:

```text
1. build the relevant tools
2. run public sanitized evals
3. check whether private eval credentials are available
4. download private eval pack only for trusted workflows
5. verify checksums or signatures
6. run private evals
7. upload aggregate metrics
8. fail only on defined metric regressions
```

Never print raw sensitive eval text or screenshots in CI logs. Use opaque case IDs and aggregate metrics:

```text
precision=0.982
recall=0.941
false_positive_rate=0.006
p95_latency_ms=8.4

failed_cases:
  - case_id: en_evasion_0142
    expected: block
    actual: allow
    category: evasion
```

Private evals must not run on untrusted fork pull requests because a malicious PR could exfiltrate secrets.

