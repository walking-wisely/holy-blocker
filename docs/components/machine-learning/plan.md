# Machine Learning Pipeline — Implementation Plan

The threshold-choosing *method* (miss-rate budget, not accuracy; ranking metrics for model
comparison; a threshold is model-and-geometry-specific with no built-in default) is
recorded in
[decisions/classifier-operating-point.md](../../decisions/classifier-operating-point.md).
That document no longer carries specific measured numbers or model/checkpoint names — they
came from the same local, never-merged fine-tuning pipeline described (and removed) below,
and were scrubbed for the same reason.

The classification strategy and model gating rationale live in [../content-classification.md](../../architecture/content-classification.md).

## Decided: no corpus of real explicit imagery, ever, in any form

A prior local (never pushed) pipeline fine-tuned against a gated real-imagery corpus —
downloaded via `HF_TOKEN`, decoded in memory, immediately reduced to cached feature
vectors so raw images touched disk only inside one extraction pass. That was already
more careful than "just keep a folder of images", and it still produced a real near-miss:
a `.gitignore` anchoring bug once left 58 MB of feature vectors derived from that corpus
sitting as ordinary untracked files, one `git add -A` away from being committed (full
account in this repo's PR/commit history for context, not reproduced here).

**As of 2026-08-07, this project holds no explicit corpus and will not acquire one again
— not on disk, not gitignored, not as cached feature vectors, not transiently in memory.**
Recall (does the classifier still catch explicit content) therefore cannot be
self-measured against a held-out positive set the way `false-positive rate` is measured
against `synthetic-ui`/`real-world-macbook` below. Whatever validates recall going
forward has to work without this project ever taking custody of the material.

**The no-custody approach is now chosen (2026-09-19)**, in
[decisions/learning-from-feedback.md](../../decisions/learning-from-feedback.md)'s
Evaluation section: Confidence-Based Performance Estimation off the deployed model's own
score stream, third-party vendor benchmarks cited as reference only, and a release gate
built from a cross-style specificity regression suite (legally-clean imagery: classical
art nudity, licensed swimwear/lingerie stock, safe-for-work anime, breastfeeding/medical,
the bed-photo case) plus a dogfood-only shadow-mode comparison — never fleet-wide dual
inference. That same section also records why holding *any* explicit benchmark, even
privately and out of this repo, is foreclosed for this project specifically (a
jurisdictional constraint, not just a preference), and why domain-blocklist coverage
means the residual distribution this classifier has to catch is now narrow enough that
live-usage signal converges slowly by construction — read that section before proposing
a new eval mechanism here.

This document is otherwise the build plan for what exists today: the baseline evaluation
harness. It does not describe a fine-tuning pipeline — none is planned while the decision
above holds.

## Baseline evaluation (v0)

**Done**, on `machine-learning/` as it exists on this branch. Uses
[`Falconsai/nsfw_image_detection`](https://huggingface.co/Falconsai/nsfw_image_detection)
(ViT-base, Apache-2.0, `id2label = {0: "normal", 1: "nsfw"}`), downloaded from Hugging Face
on the first evaluation run rather than shipped as weights in this package, exactly as
published — no fine-tuning — and measures its false-positive behavior on **rendered UI / text-heavy
screenshots**, a distribution no model or geometry this project has shipped has been
measured against. Per [classifier-operating-point.md](../../decisions/classifier-operating-point.md)'s
"a threshold belongs to a model *and* a geometry" rule, **the numbers below describe a
different model under a different geometry** — this classifier's own published
whole-image resize, not `packages/image-sandbox`'s tile-max — and are not comparable to
`image-sandbox`'s configured threshold.

- `model.py` — loads the pretrained classifier; resolves the NSFW class index from the
  model's own `id2label` rather than hardcoding it (a silent label-order flip would
  otherwise score every image backwards with no error anywhere, the same failure mode
  `packages/image-sandbox`'s pinned `BINARY_LABELS` guards against).
- `synth_ui.py` — a fully synthetic generator for the benign corpus (code editors, chat,
  documents, terminals, spreadsheets, forms; common desktop/phone resolutions; light/dark
  themes). No captured screen content, no third-party imagery.
- `corpus.py` — loads a labeled-by-directory corpus from a gitignored local path; raises
  loudly rather than silently scoring zero images if the path is missing.
- `eval.py` — pure: score distribution (mean/median/p90/p95/p99) plus a threshold sweep,
  plus (when the caller supplies labels, as `evaluate_corpus` does) the top-N
  highest-scoring filenames in `report()`, so a tail statistic is never an unexplainable
  number — see the README's "top 10 highest-scoring items" for what this actually found.
  No single threshold is picked — this model has no measured operating point yet.
- `scripts/run_baseline_eval.py` — generates the corpus if needed and runs the eval.
- 32 tests, all deterministic (no torch/transformers, no network, no model download, no real
  corpus needed to run) — most are pure-logic, and the synthetic UI tests additionally use
  Pillow and write to a temp directory on disk.

**Measured** (300 synthetic images, seed 0; reproduced at seed 1, max drift 1.34pp per
threshold — see `machine-learning/README.md` for the full second-seed sweep): mean score
0.052, median 0.002 — most rendered UI scores very low — but the tail is real: p95 =
0.31, p99 = 0.63 (the 4th-highest of 300 scores), max = 0.77 (a single image). False-
positive rate by candidate threshold: 14.3% @ 0.10, 9.7% @ 0.20, 6.0% @ 0.30, 2.3% @
0.50, 0.7% @ 0.70 (**2 of 300 images** — at n=300 the tail thresholds are a handful of
samples wide and should not be read as precise). No threshold is chosen here; this is a
first data point on an off-the-shelf model, under its own preprocessing, against a fully
synthetic corpus containing no photographic or illustrated content — the two biggest gaps
to a production number, both listed below.

**Real screenshots.** ~~a small batch of real local screenshots (never committed) would
sanity-check the synthetic set against reality~~ **Done.** A manually captured 9-image
batch (`data/eval/real-world-macbook/`, gitignored — Finder, 4 real Safari tabs,
Calculator, TextEdit, System Settings, Maps) confirms the synthetic tail is real, not a
generator artifact: `05_calculator.png` scores 0.71, in the same range as the synthetic
corpus's `document`-scene tail (max 0.77). FPR 11.1% (1 of 9) at every threshold 0.10
through 0.70, 0% at 0.90 — n=9 is too small to be a rate estimate, its job was only to
confirm the synthetic signal isn't spurious. `scripts/run_baseline_eval.py` now evaluates
both corpora in one run via `corpus.discover_corpora`, which finds whichever named
corpora exist under `data/eval/` and skips the rest with a note rather than raising,
since a real-screenshot batch (unlike the synthetic one) cannot be regenerated from code.

**What this does not cover, deliberately deferred:**
- **Geometry** — evaluated with the model's own published preprocessing (direct resize to
  224×224, mean/std 0.5), not `packages/image-sandbox`'s tile-max geometry. Building a
  faithful tile-max harness in Python is only worth it once a backbone is actually chosen
  for production.
- **Recall** — only the benign/false-positive side is measured. Recall needs a held-out
  explicit corpus, which this pipeline deliberately does not source or store (see
  `corpus.py`'s docstring), and per
  [decisions/learning-from-feedback.md](../../decisions/learning-from-feedback.md)'s
  2026-09-19 revision, no such corpus will be sourced or stored going forward either —
  see that section for what replaces it (CBPE off the live score stream, vendor
  benchmarks as reference only, and a specificity-suite release gate).
- **Fine-tuning** — considered and deliberately not attempted, not merely deferred.
  Fine-tuning against `synthetic-ui`/`real-world-macbook` would only have safe-labeled
  examples to train on, and with no held-out explicit corpus to check afterward (see
  Recall, above), a recall regression from that fine-tune would be silent — the one
  failure mode a content blocker least affords. Separately, 300 images from one
  procedural generator is a narrow enough style that fine-tuning the whole head against
  it risks learning "not this synthetic look" rather than "UI is safe", with nothing to
  catch that either. Per the decision above, this project does not hold an explicit
  corpus to validate against and is not acquiring one — any future fine-tuning needs a
  no-custody way to check recall first, not just "a validation split".

See `machine-learning/README.md` for setup and how to re-run the eval.

## What was removed

Through 2026-07-18 this section described a completed fine-tuning pipeline in detail —
`dataset.py`, `corpus.py`, `gate.py`, `train.py`, `finetune.py`, `features.py`,
`export_tflite.py`, `quantize.py`, module signatures, an implementation order, deviations
from the original plan, and next steps. None of that code was ever pushed to any shared
branch (verified against `origin` history 2026-08-07): the corpus-touching modules and
any credentials lived only on one local machine. Only the general threshold-choosing
*method* it established survives, in `docs/decisions/classifier-operating-point.md` — the
specific numbers, model names, and checkpoints it produced have been scrubbed from that
document for the same reason this section is removed here.

The section is removed here, not just marked stale, per the decision above: this project
is not rebuilding that pipeline shape (load a real corpus, however carefully, to fine-tune
or gate against), so a module-by-module plan for it would describe work that will not
happen. If a no-custody validation approach is chosen later, it gets its own plan section
rather than resurrecting this one.

## What this does not cover

- Federated learning, server aggregation, or any cloud components. This pipeline stays
  strictly local and self-contained. The head that federated learning would train is
  described above as a design direction only; the crate it belongs in is planned as
  [classifier-head](../classifier-head/plan.md), not here, and any network path must be opt-in and off by default per the
  local-first rule in AGENTS.md.
- Sourcing or curating training data. The `data/` directory is gitignored; data curation is a separate out-of-repo process.
- Text NLP model training. Text classification is handled deterministically by `packages/text-policy`; see [../content-classification.md](../../architecture/content-classification.md) (§ When To Add ML) for the gating rationale.
- Export/quantization tooling (ONNX, TFLite, int8) — belonged to the removed fine-tuning
  pipeline above and is not part of the baseline evaluation harness this document now
  describes.
