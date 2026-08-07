# Machine Learning Pipeline — Implementation Plan

**Measured performance lives in [results.md](results.md).** Planned experiments are
under [experiments/](experiments/). The threshold and metric choices are recorded in
[decisions/classifier-operating-point.md](../../decisions/classifier-operating-point.md).

The classification strategy and model gating rationale live in [../content-classification.md](../../architecture/content-classification.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

**Note on the "Current state" section below (added on `feat/ml-nsfw-baseline-eval`,
branched from `master`):** it describes a fine-tuning pipeline (`config.py`, `labels.py`,
`train.py`, `export_tflite.py`, etc.) that does not exist in `machine-learning/` on this
branch — only [results.md](results.md) and this plan file carry that history forward. What
actually exists here is a much smaller, fresh **baseline evaluation harness**, described in
[Baseline evaluation (v0)](#baseline-evaluation-v0) below. Treat the rest of "Current state"
as historical/aspirational until the fine-tuning pipeline is rebuilt or merged in from
elsewhere.

## Baseline evaluation (v0)

**Done**, on `machine-learning/` as it exists on this branch. Uses
[`Falconsai/nsfw_image_detection`](https://huggingface.co/Falconsai/nsfw_image_detection)
(ViT-base, Apache-2.0, `id2label = {0: "normal", 1: "nsfw"}`), downloaded from Hugging Face
on the first evaluation run rather than shipped as weights in this package, exactly as
published — no fine-tuning — and measures its false-positive behavior on **rendered UI / text-heavy
screenshots**, a distribution [classifier-operating-point.md](../../decisions/classifier-operating-point.md)
never measures for any model or geometry this project has shipped. Per that decision's
"a threshold belongs to a model *and* a geometry" rule, **the numbers below describe a
different model under a different geometry** — this classifier's own published
whole-image resize, not `packages/image-sandbox`'s tile-max — and are not comparable to
`image-sandbox`'s thresholds. Notably, two of the six thresholds swept below (0.20 and
0.50) coincide with values `classifier-operating-point.md` records as *superseded* or
*not a real threshold* for the deployed model; that coincidence is numeric only.

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

**What this does not cover, deliberately deferred:**
- **Geometry** — evaluated with the model's own published preprocessing (direct resize to
  224×224, mean/std 0.5), not `packages/image-sandbox`'s tile-max geometry. Building a
  faithful tile-max harness in Python is only worth it once a backbone is actually chosen
  for production.
- **Recall** — only the benign/false-positive side is measured. Recall needs a held-out
  explicit corpus, which this pipeline deliberately does not source or store (see
  `corpus.py`'s docstring).
- **Real screenshots** — the corpus is 100% synthetic; a small batch of real local
  screenshots (never committed) would sanity-check the synthetic set against reality.
- **Fine-tuning** — out of scope for this pass by request.

See `machine-learning/README.md` for setup and how to re-run the eval.

## Current state

All six planned steps are complete. The package at `machine-learning/` has:

- `config.py` — `TrainingConfig` dataclass with `data_dir`, `output_dir`, `image_size`, `batch_size`, `epochs`, `learning_rate`, and `max_model_mb`.
All planned steps are complete, plus the corpus harness and release guardrail
added in parallel (#65). The package at `machine-learning/` has:

- `config.py` — `TrainingConfig` dataclass with `data_dir`, `output_dir`, `image_size`, `batch_size`, `epochs`, `learning_rate`, and `max_model_mb`.
- `labels.py` — pinned `BINARY_LABELS = ("safe", "explicit")` and `POSITIVE_INDEX`. Class order is fixed here rather than derived from sorted directory names, which would invert FP/FN readings if a directory were renamed.
- `model.py` — `create_classifier(class_count, pretrained)`, `create_feature_extractor`, `freeze_backbone`, `unfreeze_last_blocks`.
- `dataset.py` — `LocalImageDataset` and `load_dataset` over `root/<label>/`; `ZipImageDataset` for reading a corpus archive in memory; `find_images` and `load_image` for the flat, label-free listings `corpus.py` consumes.
- `corpus.py` — single-class evaluation corpora. Each corpus is uniform by construction, so one measurement reads as the false-positive rate on a benign corpus and as recall on an explicit one.
- `gate.py` — release guardrail. A recall regression is CRITICAL and rolls back; failure to improve the false-positive rate is only a WARNING.
- `train.py` — fine-tuning loop with per-epoch validation, device selection (MPS/CUDA/CPU), and a checkpoint carrying the label order.
- `finetune.py` — backbone fine-tuning with discriminative learning rates, resumable across interruptions via `checkpointing.py`.
- `features.py` / `extract.py` — convert a corpus into non-viewable feature vectors so evaluation never touches images again.
- `eval.py` — pure metrics: `collect_predictions`, `score`, `evaluate`, `sweep_thresholds`, `misclassified`, `report`.
- `metrics.py` — ROC-AUC, PR-AUC, miss-budget operating points, error-confidence diagnostic.
- `harness.py` — `holy-blocker-eval` CLI over either images or cached vectors.
- `export.py` / `quantize.py` / `export_tflite.py` — ONNX (5.81 MB), int8 ONNX (1.61 MB), TFLite (5.90 MB), all verified end to end.
- `tests/` — 125+ tests. All fixtures are synthetic; no real imagery is required.

Measured performance is in [results.md](results.md).

**Still open:** a shippable `baseline-v0` checkpoint. The intended v0 adopts open
pretrained NSFW weights rather than training from a curated corpus, which avoids
taking permanent custody of explicit material — the fine-tuning path here reads a
corpus archive in memory and deletes it afterwards.

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

### `synth_composite.py` and `transport.py` — the screen-path measurement

**Planned 2026-08-08, and the next thing to build in this package.** It is the Stage 0 of
[image-sandbox's screen path](../image-sandbox/plan.md#the-screen-path), and everything downstream
of it waits on the numbers it produces.

#### Why this exists

[classifier-operating-point.md](../../decisions/classifier-operating-point.md) records that the
screen path operates outside every measurement this project has, and the macOS daemon has since
shipped a classifier onto real frames anyway. The gap cannot be closed the obvious way —
[image-corpus-custody.md](../../decisions/image-corpus-custody.md) forbids acquiring a corpus of
explicit screen frames, and nothing else would do.

What can be measured instead is the **transport shift**: what the pipeline's *geometry* does to a
score, independent of content.

```
shift(X) = s(X) - s(pipeline(X))
```

where `s` is the classifier under its own published preprocessing and `pipeline` is
composite-into-UI → capture-geometry → `check_raw`. Because crop, scale, composite and overlay are
**content-blind** — they do the same thing to a photo of a dog as to anything else — a shift
measured on benign imagery describes what the pipeline does to any image, including ones this
project cannot hold. If the shift is near zero, the checkpoint's published operating point
transfers to the screen path; if it is not, the size of it is the quantified loss.

This is a **paired** design, and that is load-bearing twice over: it licenses the transfer
argument, and it cancels photo-source bias, since a source's quirks sit in both terms and subtract
out.

#### `synth_composite.py` — the corpus is a compositor, not a download

Reuses `synth_ui.py`'s pattern exactly: a pure, Pillow-free `plan_*` function that is
deterministic in `(count, seed)`, a separate renderer, and a `manifest.json` so a stale run is
detectable.

```python
@dataclass(frozen=True)
class OverlaySpec:                 # style, coverage, opacity, font, colour, scrim
@dataclass(frozen=True)
class CompositeSpec:
    background: BackgroundKind     # "synthetic_ui" | "own_screen"
    inset: InsetKind               # "photo" | "flat_panel"   <- the keep/discard label
    inset_rect: Rect               # ground truth for any future region proposal
    overlay: OverlaySpec
    seed: int

def plan_composites(count: int, seed: int) -> list[CompositeSpec]: ...
def render_composite(spec: CompositeSpec, photos: PhotoPool) -> Image: ...
```

Every label falls out of the spec — where the picture is, whether it is one, and how much text
covers it. No human annotation, nothing to review, nothing explicit anywhere near it.

**Overlay styles are generated, not sourced.** The overlays that matter here — a caption bar with a
dark scrim, burned-in subtitles, a corner watermark, meme text, a timestamp pill, play-button
chrome — are axis-aligned and drawn by a compositor. `SynthText`'s value is perspective-warping
text onto scene geometry (a sign on a wall), which is the opposite of what is needed; borrow
`SynthTIGER`'s font/colour/opacity sampling *ranges* and cite them, but take neither dependency.
Fonts from Google Fonts (OFL), vendored so runs reproduce.

**Photo sources**, per [image-corpus-custody.md](../../decisions/image-corpus-custody.md):
developer's camera roll first, 3D renders second, COCO `val2017` third. Mix them deliberately —
a single-source pool shares one compression signature, and the statistics under test are sensitive
to exactly that, so a result could be "can I detect JPEG" wearing the wrong label. **Report every
number sliced by photo source**; a source-specific effect then announces itself.

#### `transport.py` — the measurement

```python
@dataclass(frozen=True)
class TransportResult:             # per-cell shift distribution + n
def measure_transport(scorer, composites) -> list[TransportResult]
```

Report median and p95 of the shift, sliced by **inset scale**, **text-overlay coverage**, and
**photo source**. Improvement means the distribution collapsing toward zero, especially in the
small-inset and high-overlay cells, which is where it is expected to be worst today.

Two negative controls that must hold, or the corpus is not testing what it claims:

- a photographic inset must survive at *every* overlay coverage level;
- a flat-UI inset under the *same* overlays must not.

#### What this measures, and what stays an argument

| Term | How | Needs explicit content? | Kind |
|---|---|---|---|
| Classifier quality on images | inherited from the checkpoint's published eval | someone else's | measurement |
| **Transport shift** | paired, benign, above | no | **measurement** |
| False positives on real screens | run the daemon on the developer's own screen and count | no | measurement |
| Gate false-discard rate | gated vs ungated, 1 frame in 20 | no | measurement |
| Latency | timer | no | measurement |
| **Screen miss rate** | inherited miss rate × measured ≈0 shift | — | **argument** |

The last row is the only inference, and it is labelled as one. Everything else is a number.

**Note the harness this builds on lives on `feat/ml-nsfw-baseline-eval`, not `master`** —
`synth_ui.py`, `corpus.py` and `eval.py` were removed from `master` when the ML pipeline was
reverted. Restoring or rebasing that branch is a prerequisite, not an optional cleanup.

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

1. ~~Add `pytest` to `pyproject.toml` `[project.optional-dependencies]` as `test = ["pytest"]`; add `[tool.pytest.ini_options]` pointing at `tests/`.~~ **Done.**
2. ~~`dataset.py` — implement `LocalImageDataset` and `load_dataset`; write `tests/test_dataset.py` with synthetic images using `tmp_path`.~~ **Done.**
3. ~~`eval.py` — implement `evaluate` and `report`; write `tests/test_eval.py` with a trivially correct model on a two-label synthetic loader.~~ **Done.** Extended past the original scope with `collect_predictions`, `sweep_thresholds`, `misclassified`, and a `harness.py` CLI (`holy-blocker-eval`) that reports false positives and negatives against a local evaluation set.
4. ~~Wire `dataset.py` and `eval.py` into `train.py` — replace the placeholder training loop with a real epoch loop over `load_dataset`, a validation call to `evaluate` after each epoch, and progress printing via `report`.~~ **Done.**
5. ~~`quantize.py` — implement `quantize_onnx`; extend `tests/test_export.py` to verify the quantized model loads and has a smaller file size than the original.~~ **Done.** Verified at 5.81 MB → 1.61 MB.
6. ~~`export_tflite.py` — implement TFLite export; add `tests/test_export.py` coverage for the TFLite path using a tiny synthetic model.~~ **Done.** Verified end to end: a trained checkpoint converts to a 5.90 MB flatbuffer that loads and runs in the LiteRT interpreter.

## Deviations from the original plan

Two decisions above differ from what this document originally specified.

**TFLite conversion no longer goes through TensorFlow.** The planned path was
`torch → ONNX → TF SavedModel → TFLite`, which requires `tensorflow` as a
dependency. `tensorflow` publishes no wheel for current Python builds, and the
route pulls in a second full framework purely as a conversion middleman.
`export_tflite.py` instead uses `litert-torch` (the renamed successor to
`ai-edge-torch`), which converts a torch `ExportedProgram` straight to a LiteRT
flatbuffer. `tensorflow` has been dropped from `pyproject.toml`; the converter
lives behind an optional `tflite` extra.

**Dynamic range quantization is not applied to the TFLite artifact.** The plan
called for it on the first export. `litert-torch` exposes PT2E quantization,
which is static int8 and needs a calibration dataset — the same prerequisite
this plan defers under "What this does not cover". The float32 artifact is
~6 MB against a 15 MB budget, so the size pressure that motivated quantization
is not there yet. ONNX dynamic quantization for Windows *is* implemented in
`quantize.py`, since `onnxruntime` supports it without calibration data.

**Python version ceiling.** `litert-torch` depends on `torchao`, which does not
import on Python 3.14. The package now declares `requires-python = ">=3.11,<3.14"`.

**ONNX export pins the legacy exporter.** `torch>=2.9` defaults to the dynamo
ONNX exporter, whose MobileNetV3 graph carries inconsistent shape metadata in
the classifier; `onnxruntime`'s shape inference rejects it during quantization
("Inferred shape and existing shape differ in dimension 0: (576) vs (1024)"),
reproduced across opsets 17–21 at both 32px and 224px. `export.py` passes
`dynamo=False`. The legacy exporter is deprecated, so this needs revisiting.

## Next steps

1. ~~**Full unfreeze.** Training accuracy is 94.6% against 92.2% validation — a 2.4pp
   gap. The model underfits, so more capacity is the cheapest remaining gain and a
   prerequisite for judging whether more data helps.~~ **Done** — improved every
   metric on both evaluation sets with no regression; see
   [experiments/full-unfreeze.md](experiments/full-unfreeze.md).
2. **Threshold from the miss budget**, not 0.5. See
   [results.md](results.md#cost-of-a-miss-budget).
3. **[Anime subsampling experiment](experiments/anime-subsampling.md)** — pre-registered,
   run only after the full unfreeze. Unblocked; baselines re-fixed against the
   full-unfreeze model.
4. Relabelling study to settle the label-noise question, which remains open.
5. **The screen-path transport measurement** — `synth_composite.py` + `transport.py` above. This is
   the highest-value next step in this package: it is the baseline for every claim about the macOS
   daemon's image path, it needs only benign imagery, and it splits the live-observed failures into
   geometry (fixable and measurable here) versus concept (needs a second expert, unmeasurable
   here). Run it against the pipeline as it exists **before** changing anything.
6. **Sourcing rules are now a decision, not a judgement call** — see
   [image-corpus-custody.md](../../decisions/image-corpus-custody.md). It supersedes the
   "read a corpus archive in memory and delete it afterwards" allowance for corpora of *unknown*
   provenance: no third-party imagery is ingested at all.


## What this does not cover

- Federated learning, server aggregation, or any cloud components. This pipeline stays
  strictly local and self-contained. The head that federated learning would train is
  described above as a design direction only; the crate it belongs in is planned as
  [classifier-head](../classifier-head/plan.md), not here, and any network path must be opt-in and off by default per the
  local-first rule in AGENTS.md.
- Sourcing or curating training data. The `data/` directory is gitignored; data curation is a separate out-of-repo process.
- Text NLP model training. Text classification is handled deterministically by `packages/text-policy`; see [../content-classification.md](../../architecture/content-classification.md) (§ When To Add ML) for the gating rationale.
- iOS CoreML export — deferred until iOS platform work begins.
- Int8 static quantization — requires a calibration dataset and is noted in `export_tflite.py` comments for a later pass.
