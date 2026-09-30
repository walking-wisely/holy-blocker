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

## Current state

Not yet built. This is the plan for a `machine-learning/` package under
`machine-learning/src/holy_blocker_ml`; nothing in it should be assumed to
exist until the modules below are implemented and their tests pass.

The intended v0 adopts open pretrained NSFW weights rather than training from
a curated corpus, which avoids taking permanent custody of explicit material —
any fine-tuning path this plan describes must read a corpus archive in memory
and delete it afterwards rather than persisting it to disk.

## What to add

### `dataset.py`

```
src/holy_blocker_ml/dataset.py
```

Loads images from a local directory tree where each subdirectory name is a label. Mirrors the standard PyTorch `ImageFolder` contract but keeps the implementation explicit so augmentation policy is easy to read and change.

Expected data layout (gitignored — private assets):

```
data/train/<label>/<image>
data/val/<label>/<image>
```

Key signatures:

```python
class LocalImageDataset(Dataset):
    def __init__(self, root: Path, image_size: int, augment: bool) -> None: ...
    def __len__(self) -> int: ...
    def __getitem__(self, idx: int) -> tuple[Tensor, int]: ...

def load_dataset(root: Path, image_size: int, augment: bool) -> DataLoader:
    """Return a DataLoader over LocalImageDataset.

    Training transforms: random horizontal flip, random resized crop, color jitter.
    Validation transforms: resize then center crop only.
    """
```

Tests use `tmp_path` fixtures with small synthetic PNG images so real data is never required in CI.

### `eval.py`

```
src/holy_blocker_ml/eval.py
```

Pure evaluation function — no file I/O. Receives a model and a loader, returns a structured result.

```python
@dataclass
class EvalResult:
    accuracy: float
    per_class_precision: dict[int, float]
    per_class_recall: dict[int, float]
    confusion_matrix: list[list[int]]

def evaluate(model: nn.Module, loader: DataLoader) -> EvalResult:
    """Run inference over loader and return metrics. Sets model to eval mode."""

def report(result: EvalResult) -> str:
    """Return a human-readable summary of an EvalResult."""
```

### `export_tflite.py`

```
src/holy_blocker_ml/export_tflite.py
```

Converts a PyTorch checkpoint to a TFLite flatbuffer for Android (ONNX Runtime handles Windows).

**The conversion path was revised after a toolchain review (2026-07-18).** The
original `torch → ONNX → TF SavedModel → TFLite` route is a dead end: it depends on
`onnx-tf`, whose README carries an explicit deprecation notice and whose last release
was 1.10.0 in March 2022. Current guidance:

- **Primary: `litert-torch`** — Google's supported PyTorch→LiteRT converter, formerly
  named `ai-edge-torch` (the GitHub repo now redirects; the import is `litert_torch`).
  Converts directly from a `torch.nn.Module`, no ONNX hop. Converter status is Beta.
- **Fallback: `onnx2tf`** (PINTO0309) — actively maintained, converts ONNX → LiteRT
  directly and handles the NCHW→NHWC transposition that the old path got wrong.
- **Do not use `onnx-tf`.**

`litert-torch` does *not* require a full `tensorflow` install — it brings its own
runtime stack (`ai-edge-litert`, `litert-converter`, `ai-edge-quantizer`). The
`tensorflow` dependency in `pyproject.toml` should be dropped when this module lands.

Constraints to check before starting — these bound the whole dev environment:

| Constraint | Value | Status |
|---|---|---|
| `litert-torch` version | 0.9.1 (2026-05-19) | |
| Python | `>=3.10, <3.14` | Verified — 3.13.14 works; 3.14 is out of range |
| torch | `>=2.4.0, <2.13.0` | Verified — resolves to torch 2.12.1 |
| Platform | Linux only per repo README | **Contradicted — conversion runs on macOS arm64** |

The `.tflite` file extension and format are unchanged by the LiteRT rename.

#### Conversion smoke test (run 2026-07-18, macOS arm64, Python 3.13.14)

Converting `mobilenet_v3_small` with a 2-class head — the exact shape
`model.create_classifier()` produces — **succeeds**, which settles the open question
about `hardswish` and squeeze-and-excite op support:

| Measure | Result |
|---|---|
| Conversion | Succeeds in ~11 s |
| Artifact size (fp32, unquantized) | **6.19 MB** — inside the 15 MB `max_model_mb` budget before any quantization |
| Compute | 112.7 M arithmetic ops / 56.4 M MACs |
| Numerical fidelity vs torch | max abs logit difference **4.5e-7** |

Two practical notes for whoever writes `export_tflite.py`: the README's Linux-only
claim did not hold — the macOS arm64 wheels install and convert correctly, so no
Docker is required for conversion. And the runtime emits `FutureWarning`s from
`litert_torch/_convert/signature.py` about deprecated `treespec.children_specs`;
these are internal to the library and harmless.

#### CLIP attention-pool smoke test (same run)

A candidate long-run backbone is `TinyCLIP-ResNet-30M-Text-29M-LAION400M` (MIT). Its
vision tower is a CLIP `ModifiedResNet` — conv layers, which the MobileNetV3 test
already covers, plus **one** `AttentionPool2d` block at the end over a fixed 7×7+1
token grid. That attention block was the only unverified op risk, so it was tested in
isolation (module rebuilt to the checkpoint's shapes, random weights, static input —
convertibility depends on ops, not weights):

| Measure | Result |
|---|---|
| Conversion of `AttentionPool2d(1792 → 1024, 7×7, 28 heads)` | **Succeeds in ~1.3 s** |
| Output shape | `(1, 1024)` as expected |
| Numerical fidelity vs torch | max abs difference **2.0e-7** |

So `F.multi_head_attention_forward` converts through `litert-torch` at a static shape.
Note the attention pool alone is ~11.5 M parameters — roughly a third of the 29.6 M
vision tower — so the documented fallback of dropping it for global-average pooling
(the `timm/resnet50_clip_gap.openai` pattern) would also shrink the artifact
substantially, at some cost in embedding quality.

**Still untested:** the full assembled ResNet tower, and int8 quantization, which has
an [open crash report](https://github.com/google-ai-edge/litert-torch/issues/150) when
a representative dataset is passed. Both probes above were fp32.

#### Export contract (decided)

Whatever backbone is chosen, `export_tflite.py` must export the **backbone only**,
terminating at the embedding, with `forward` returning `(logits, embedding)`.
Classification stays outside the graph — see the runtime findings recorded in
[../../decisions/learning-from-feedback.md](../../decisions/learning-from-feedback.md#on-device-runtime-and-where-the-head-lives-decided-2026-07-18),
and [classifier-head/plan.md](../classifier-head/plan.md) for the crate that consumes the
embedding.
Baking the head into the `.tflite` would forfeit the ability to retarget the runtime
later, and would break the on-device training design. The quantization recipe must be
pinned and the embedding dtype/scale treated as a versioned interface: an int8
backbone's scale and zero-point become part of the head's input contract, so a silent
re-export would invalidate every fine-tuned head in the field.

Reference documents:

- [litert-torch repository](https://github.com/google-ai-edge/litert-torch)
- [Convert PyTorch to LiteRT](https://developers.google.com/edge/litert/models/convert_pytorch)
- [LiteRT for Android](https://developers.google.com/edge/litert/android)
- [ai-edge-quantizer](https://github.com/google-ai-edge/ai-edge-quantizer)
- [onnx2tf](https://github.com/PINTO0309/onnx2tf) (fallback path)

```python
def export_tflite(
    checkpoint_path: Path,
    output_path: Path,
    config: TrainingConfig,
) -> Path:
    """Convert checkpoint to TFLite via litert_torch.convert().

    Applies dynamic range quantization for the first export.
    Int8 static quantization requires a calibration dataset and is not done here.
    Target artifact: data/models/baseline-v0.tflite
    """
```

MobileNetV3 op support (`hardswish`, squeeze-and-excite) through this converter is
**unverified** — the review found no evidence either way. Both decompose into Core
ATen primitives the converter targets, so it is expected to work, but treat a
conversion smoke test as a required first step rather than a formality. If conversion
fails, `litert-torch` ships a `find_culprits` tool that bisects the graph to the
offending op.

### `corpus.py` and `gate.py`

The false-positive / false-negative measurement harness. Both corpora are single-class
by construction, so no per-file labels exist on disk — the label comes from the corpus
kind, and one computation (the rate of explicit predictions) reads as the
false-positive rate on a benign corpus and as recall on an explicit one.

```python
class CorpusKind(Enum):        # BENIGN -> false_positive_rate, EXPLICIT -> recall
class CorpusSpec:              # name, root, kind
class CorpusMeasurement:       # name, kind, item_count, value, metric_name

def load_corpus(spec, image_size, batch_size) -> DataLoader
def explicit_prediction_rate(model, loader) -> float
def measure_corpus(model, spec, image_size, batch_size) -> CorpusMeasurement
```

`gate.py` implements the release rule from
[../../decisions/learning-from-feedback.md](../../decisions/learning-from-feedback.md):
recall regression is CRITICAL and rolls back; failure to improve the false-positive
rate is a WARNING only. A false-positive win can never buy a recall regression — the
corruptible metric has no authority over the guarded one.

```python
class MetricSnapshot:          # recall, false_positive_rate
class GuardrailThresholds:     # max_recall_drop, min_false_positive_improvement
class GateSeverity(Enum):      # OK, WARNING, CRITICAL

def check_guardrail(baseline, candidate, thresholds=None) -> GateResult
```

**Corpus data never enters the repo.** `CorpusSpec.root` points at a gitignored local
path; `load_corpus` raises a `FileNotFoundError` that says so. Tests exercise the
plumbing with synthetic noise images only.

### `quantize.py`

```
src/holy_blocker_ml/quantize.py
```

Reduces ONNX artifact size for Windows deployment using `onnxruntime.quantization.quantize_dynamic`. Kept deliberately thin.

```python
def quantize_onnx(model_path: Path, output_path: Path) -> Path:
    """Apply dynamic range quantization to an ONNX model.

    Uses onnxruntime.quantization.quantize_dynamic with default settings.
    Returns output_path.
    """
```

### `tests/`

Add `pytest` as a dev dependency and a `[tool.pytest.ini_options]` section pointing at `tests/`.

```
tests/
  test_model.py      — create_classifier output shape for several class counts
  test_export.py     — export_onnx produces a loadable .onnx file (synthetic model, tmp_path)
  test_dataset.py    — LocalImageDataset with a tmp_path fixture containing a few dummy images
  test_eval.py       — evaluate with a trivially correct model on a two-sample synthetic dataset
```

The `pyproject.toml` additions:

```toml
[project.optional-dependencies]
test = ["pytest"]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

## Implementation order

## Implementation order

1. Add `pytest` to `pyproject.toml` `[project.optional-dependencies]` as `test = ["pytest"]`; add `[tool.pytest.ini_options]` pointing at `tests/`.
  <!-- step: machine-learning.pytest-config -->
2. `dataset.py` — implement `LocalImageDataset` and `load_dataset`; write `tests/test_dataset.py` with synthetic images using `tmp_path`.
  <!-- step: machine-learning.dataset -->
3. `eval.py` — implement `evaluate` and `report`; write `tests/test_eval.py` with a trivially correct model on a two-label synthetic loader.
  <!-- step: machine-learning.eval -->
4. Wire `dataset.py` and `eval.py` into `train.py` — replace the placeholder training loop with a real epoch loop over `load_dataset`, a validation call to `evaluate` after each epoch, and progress printing via `report`.
  <!-- step: machine-learning.train-wiring -->
5. `quantize.py` — implement `quantize_onnx`; extend `tests/test_export.py` to verify the quantized model loads and has a smaller file size than the original.
  <!-- step: machine-learning.quantize -->
6. `export_tflite.py` — implement TFLite export; add `tests/test_export.py` coverage for the TFLite path using a tiny synthetic model.
  <!-- step: machine-learning.export-tflite -->

## Notes for whoever builds this

TFLite conversion should go through `litert-torch` (the renamed successor to
`ai-edge-torch`), converting a torch `ExportedProgram` straight to a LiteRT
flatbuffer, rather than `torch → ONNX → TF SavedModel → TFLite`: that route
needs a full `tensorflow` install purely as a conversion middleman, and
`tensorflow` publishes no wheel for current Python builds. `litert-torch`
depends on `torchao`, which does not import on Python 3.14, so the package
should declare `requires-python = ">=3.11,<3.14"`. Static int8 quantization on
the TFLite path needs a calibration dataset (see "What this does not cover")
and can wait; ONNX dynamic quantization for Windows does not need one and
should be implemented first. `torch>=2.9`'s default dynamo ONNX exporter has
been unreliable for MobileNetV3-shaped classifier heads in adjacent work in
this repository — check whether the legacy exporter (`dynamo=False`) is still
needed before relying on the dynamo path.

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
