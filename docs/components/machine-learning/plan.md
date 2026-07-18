# Machine Learning Pipeline — Implementation Plan

The classification strategy and model gating rationale live in [../content-classification.md](../../architecture/content-classification.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

All six planned steps are complete. The package at `machine-learning/` has:

- `config.py` — `TrainingConfig` dataclass with `data_dir`, `output_dir`, `image_size`, `batch_size`, `epochs`, `learning_rate`, and `max_model_mb`.
- `labels.py` — pinned `BINARY_LABELS = ("safe", "explicit")` and `POSITIVE_INDEX`. Class order is fixed here rather than derived from sorted directory names, which would invert FP/FN readings.
- `model.py` — `create_classifier(class_count, pretrained)` fine-tunes MobileNetV3-Small with a replaced head; `example_input(image_size)` provides a trace input.
- `dataset.py` — `LocalImageDataset` and `load_dataset` over `root/<label>/`, with train-time augmentation and deterministic validation transforms.
- `train.py` — real fine-tuning loop: per-epoch training over `load_dataset`, validation via `evaluate`, device selection (MPS/CUDA/CPU), and a checkpoint carrying the label order.
- `eval.py` — pure metrics: `collect_predictions`, `score`, `evaluate`, `sweep_thresholds`, `misclassified`, `report`, `report_sweep`.
- `harness.py` — `holy-blocker-eval` CLI: FP/FN report, threshold sweep, and worst-misclassification listing against a local (gitignored) evaluation set.
- `export.py` — ONNX export for Windows.
- `quantize.py` — ONNX dynamic quantization (5.81 MB → 1.61 MB verified).
- `export_tflite.py` — TFLite export for Android via `litert-torch` (5.90 MB verified, loads and runs in the LiteRT interpreter).
- `tests/` — 40 tests covering dataset, eval, harness, model, train, and both export paths. All fixtures are synthetic; no real imagery is required.

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

Converts a PyTorch checkpoint to a TFLite flatbuffer for Android (ONNX Runtime handles Windows). The conversion path is `torch → ONNX → TF SavedModel → TFLite`; `tensorflow` is already a declared dependency.

```python
def export_tflite(
    checkpoint_path: Path,
    output_path: Path,
    config: TrainingConfig,
) -> Path:
    """Convert checkpoint to TFLite using the ONNX → TF SavedModel → TFLite path.

    Applies dynamic range quantization for the first export.
    Int8 static quantization requires a calibration dataset and is not done here.
    Target artifact: data/models/baseline-v0.tflite
    """
```

If `ai-edge-torch` is available in the environment it can be used as an alternative first step; fall back to the ONNX path otherwise.

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

## What this does not cover

- Federated learning, server aggregation, or any cloud components — the pipeline is strictly local and self-contained.
- Sourcing or curating training data. The `data/` directory is gitignored; data curation is a separate out-of-repo process.
- Text NLP model training. Text classification is handled deterministically by `packages/text-policy`; see [../content-classification.md](../../architecture/content-classification.md) (§ When To Add ML) for the gating rationale.
- iOS CoreML export — deferred until iOS platform work begins.
- Int8 static quantization — requires a calibration dataset and is noted in `export_tflite.py` comments for a later pass.
