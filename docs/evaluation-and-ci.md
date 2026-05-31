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

