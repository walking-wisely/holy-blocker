# Machine Learning Pipeline — Implementation Plan

The classification strategy and model gating rationale live in [../content-classification.md](../../architecture/content-classification.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

The package at `machine-learning/` already has:

- `config.py` — `TrainingConfig` dataclass with `data_dir`, `output_dir`, `image_size`, `batch_size`, `epochs`, `learning_rate`, and `max_model_mb`.
- `model.py` — `create_classifier(class_count: int) -> nn.Module` fine-tunes MobileNetV3-Small with a replaced classifier head. `example_input(image_size: int) -> Tensor` provides a synthetic trace input.
- `train.py` — `train(config: TrainingConfig) -> Path` placeholder; creates a model and saves a `.pt` checkpoint but contains no real dataset loading or training loop.
- `export.py` — `export_onnx(checkpoint_path: Path, output_path: Path, config: TrainingConfig) -> Path` loads a checkpoint and traces to ONNX opset 17.

- ~~Real dataset loading and augmentation (`dataset.py`).~~ **Done.**
- ~~Evaluation metrics — accuracy, per-class precision/recall, confusion matrix (`eval.py`).~~ **Done.**
- ~~A `pytest` test suite; there are currently no tests at all.~~ **Done** — 36 tests across `tests/`.
- ~~Single-class evaluation corpora and the release guardrail (`corpus.py`, `gate.py`).~~ **Done.**

What is missing:

- TFLite export for Android (`export_tflite.py`).
- ONNX dynamic quantization for smaller Windows artifacts (`quantize.py`).
- A shippable `baseline-v0` checkpoint. `train.py` is still a placeholder; the
  intended v0 adopts open pretrained NSFW weights rather than training from a
  curated corpus, which avoids taking custody of explicit material.

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
[../../decisions/learning-from-feedback.md](../../decisions/learning-from-feedback.md).
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

1. ~~Add `pytest` to `pyproject.toml` `[project.optional-dependencies]` as `test = ["pytest"]`; add `[tool.pytest.ini_options]` pointing at `tests/`. No code changes yet — this is the harness.~~ **Done.**
2. ~~`dataset.py` — implement `LocalImageDataset` and `load_dataset`; write `tests/test_dataset.py` with synthetic images using `tmp_path`.~~ **Done.**
3. ~~`eval.py` — implement `evaluate` and `report`; write `tests/test_eval.py` with a trivially correct model on a two-label synthetic loader.~~ **Done.**
4. ~~`corpus.py` and `gate.py` — the FPR/recall measurement harness and the release guardrail, with `tests/test_corpus.py` and `tests/test_gate.py`.~~ **Done.**
5. ~~Resolve the dev-environment constraints imposed by `litert-torch`.~~ **Done** — Python 3.13.14 + torch 2.12.1 verified working on macOS arm64; no Docker needed.
6. `baseline-v0` checkpoint — adopt open pretrained NSFW weights and reshape the head; measure it against both corpora to establish the first `MetricSnapshot` baseline.
7. `quantize.py` — implement `quantize_onnx`; extend `tests/test_export.py` to verify the quantized model loads and has a smaller file size than the original.
8. `export_tflite.py` — implement TFLite export via `litert_torch.convert()`; smoke-test MobileNetV3 conversion first, since op support is unverified.

Steps 4 and 6 replace the original plan's "wire into `train.py`" step: a real training
loop is deferred until there is feedback data to train on, and v0 ships pretrained
weights instead.

## The classifier head — design direction, not yet scheduled

**Status:** design direction. No code exists and none is scheduled. Recorded here
because it constrains decisions being made *now* — chiefly the export contract above —
and because it is likely to be needed soon after a backbone ships.

### What the head is

A classifier splits in two:

```
image → [ backbone ] → embedding → [ head ] → logits → clean / explicit
        ~30M params    1024 floats   ~2K params
```

The **backbone** turns pixels into a general-purpose feature vector. It is the
expensive half, adopted pretrained and **frozen** — never retrained, which is what
keeps an explicit training corpus out of developer custody entirely.

The **head** maps that feature vector to this product's specific decision. For a
1024-d embedding and two classes it is a single dense layer — `logits = W·e + b`,
with `W` at 2×1024 and `b` at 2, so roughly 2,050 parameters, about 0.007% of the
model. The backbone knows the image contains skin, a beach, or a medical diagram; the
head is the only part that encodes what counts as blockable.

### Why it must live outside the `.tflite`

**LiteRT weights are immutable constants inside the flatbuffer.** A head baked into
the exported graph can never change on the device — adapting it per user would mean
regenerating and shipping a model file per user.

The head is exactly the part that must change: it is what learns from user-flagged
false positives, and it is what federated learning aggregates. That is the concrete
reason for the backbone-only export contract recorded under `export_tflite.py`.

### Why hand-written, and why Rust

Federated learning requires each device to compute a **gradient** from local feedback
— training, not inference. No viable on-device training framework exists for Android:
LiteRT training needs TensorFlow-authored training graphs (which a `litert-torch`
export cannot produce), ExecuTorch ships no training API in its Android AAR, and
ONNX Runtime's `onnxruntime-training-android` has not been published since 1.19.2
(2024-09-03) with its examples repo archived in May 2026.

For one dense layer the entire training step is:

```
p       = softmax(W·e + b)
dlogits = p − onehot(label)     # 2 numbers
dW      = dlogits ⊗ e           # outer product, 2×1024
W      -= lr · dW
```

Roughly 2,048 multiply-adds — microseconds per update, and textbook-derivable rather
than novel numerics. Adopting a training framework for this would be disproportionate
even if a working one existed.

Rust because the repo already ships a Rust core to both Android and Windows through
UniFFI; a `feedback-head` + `feedback-head-ffi` pair would mirror
`text-policy` / `text-policy-ffi` exactly. Per the test-first rule, gradients must be
verified against PyTorch `autograd` on fixed fixtures plus a finite-difference check.

A privacy consequence follows from the size: because the head is ~2,050 numbers, what
leaves the device during federated aggregation is an ~8 KB weight delta, never an
image. Secure Aggregation operates on that flat vector directly.

### Sequencing caveat — measure before building this

The personalization story in
[../../decisions/learning-from-feedback.md](../../decisions/learning-from-feedback.md)
is kNN over embeddings plus an override table. **kNN needs no gradients and no
training code at all.** It may deliver most of the false-positive reduction on its
own.

The recommended order is therefore: ship the frozen backbone with the vendor's
existing head → add kNN personalization → measure the false-positive rate against the
benign corpus using `corpus.py` and `gate.py` → build the trainable head only if kNN
plateaus short of the target. That defers this crate, the federated protocol, and the
whole DP/SecAgg problem behind a measurement rather than an assumption.

The trainable head is the right architecture. It is not obviously the right next step.

## What this does not cover

- Federated learning, server aggregation, or any cloud components. This pipeline stays
  strictly local and self-contained. The head that federated learning would train is
  described above as a design direction only; if it is ever built it belongs in its own
  Rust crate, not here, and any network path must be opt-in and off by default per the
  local-first rule in AGENTS.md.
- Sourcing or curating training data. The `data/` directory is gitignored; data curation is a separate out-of-repo process.
- Text NLP model training. Text classification is handled deterministically by `packages/text-policy`; see [../content-classification.md](../../architecture/content-classification.md) (§ When To Add ML) for the gating rationale.
- iOS CoreML export — deferred until iOS platform work begins.
- Int8 static quantization — requires a calibration dataset and is noted in `export_tflite.py` comments for a later pass.
