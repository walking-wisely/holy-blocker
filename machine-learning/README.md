# machine-learning

Evaluation (and, later, fine-tuning) pipeline for Holy Blocker's image
classifier. See [`docs/components/machine-learning/plan.md`](../docs/components/machine-learning/plan.md)
for the full build plan and [`docs/decisions/classifier-operating-point.md`](../docs/decisions/classifier-operating-point.md)
for why a single threshold/accuracy number is deliberately not the headline
output here.

## What exists today

A **baseline evaluation harness only** — no fine-tuning, no custom weights.
It uses [`Falconsai/nsfw_image_detection`](https://huggingface.co/Falconsai/nsfw_image_detection)
(a ViT-base classifier, Apache-2.0, `{0: "normal", 1: "nsfw"}`), downloaded
from Hugging Face on the first evaluation run — this package ships the
harness and a default model identifier, not the weights — exactly as
published, and measures how it behaves on **rendered UI / text-heavy
screenshots** — the distribution `docs/decisions/classifier-operating-point.md`
flags as untested ("the screen path operates outside this measurement").

- `src/holy_blocker_ml/model.py` — loads a pretrained HF image classifier and
  scores one image, resolving the NSFW class index from the model's own
  `id2label` rather than hardcoding it.
- `src/holy_blocker_ml/synth_ui.py` — a fully synthetic generator for the
  benign corpus (code editors, chat, documents, terminals, spreadsheets,
  forms, across common desktop/phone resolutions and light/dark themes). No
  captured screen content, no third-party images — nothing that needs review
  or licensing.
- `src/holy_blocker_ml/corpus.py` — loads a labeled-by-directory corpus from
  a gitignored local path.
- `src/holy_blocker_ml/eval.py` — pure scoring: score distribution
  (mean/median/p90/p95/p99) plus a threshold sweep, no single threshold
  picked yet.
- `scripts/run_baseline_eval.py` — generates the synthetic corpus if needed
  and runs the full baseline eval, printing a report.

## Setup

Needs Python `>=3.10,<3.14` (this directory pins 3.13.14 via `.python-version`
for `pyenv`; torch/transformers do not yet ship wheels for 3.14).

```sh
pyenv install 3.13.14   # if not already installed
python3 -m venv .venv
.venv/bin/pip install -e ".[dev]"
```

## Running the baseline eval

```sh
.venv/bin/python scripts/run_baseline_eval.py --count 300 --seed 0
```

First run downloads the model from the Hugging Face Hub (a one-time local
fetch for offline inference afterward, not a runtime cloud call — the same
pattern `packages/image-sandbox` uses for its bundled ONNX model) and
generates the synthetic corpus into `data/eval/synthetic-ui/`
(gitignored). Example output:

```
Corpus: synthetic-ui (benign, n=300)
  score distribution: mean=0.0517 median=0.0016 p90=0.1724 p95=0.3091 p99=0.6346 max=0.7678 min=0.0002
  false-positive rate by candidate threshold:
    >= 0.10: 14.33%
    >= 0.20: 9.67%
    >= 0.30: 6.00%
    >= 0.50: 2.33%
    >= 0.70: 0.67%
    >= 0.90: 0.00%
```

Reproduced with a second seed (`--seed 1`, separate `--data-dir`): FPR@0.5 =
2.00%, FPR@0.3 = 6.00% — stable across draws, not a fluke of one corpus.

## Tests

```sh
.venv/bin/python -m pytest
```

All 27 tests are deterministic (corpus loading, score summarization, threshold
sweep, synthetic-corpus planning and regeneration, NSFW-index resolution) and
run without torch/transformers, network access, a model download, or a real
corpus existing on disk. Most are pure-logic; the synthetic-UI tests also use
Pillow and write to a temp directory.

## What this does not cover yet

- **Geometry.** This evaluates the model with its own published preprocessing
  (direct resize to 224×224, mean/std 0.5) — "as is", the way its publisher
  measured it — not through `packages/image-sandbox`'s tile-max geometry.
  Building a faithful tile-max harness in Python is deferred until a
  backbone is actually chosen for production; there's no point pinning a
  geometry-parity test against a model that gets rejected on the cheap
  whole-image pass.
- **Recall.** Only the benign (false-positive) side is measured. Recall needs
  a held-out explicit corpus, which this repo deliberately does not source
  or store (see `corpus.py`'s docstring and the top of
  `docs/components/machine-learning/plan.md`).
- **Real screenshots.** The corpus here is 100% synthetic. A small batch of
  real local screenshots (never committed) would sanity-check the synthetic
  set against reality — flagged as a fast-follow, not done in this pass.
- **Fine-tuning.** Out of scope for this pass by request; `plan.md` still
  describes the training-oriented path (`dataset.py`, `export_tflite.py`,
  `gate.py`) for whenever that's picked up.
