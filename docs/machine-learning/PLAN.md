# Machine Learning Pipeline — Implementation Plan

The classification strategy and model gating rationale live in [../content-classification.md](../content-classification.md).
This document is the build plan: what modules to add, in what order, and what each one is responsible for.

## Current state

The package at `machine-learning/` already has:

- `config.py` — `TrainingConfig` dataclass with `data_dir`, `output_dir`, `image_size`, `batch_size`, `epochs`, `learning_rate`, and `max_model_mb`.
- `model.py` — `create_classifier(class_count: int) -> nn.Module` fine-tunes MobileNetV3-Small with a replaced classifier head. `example_input(image_size: int) -> Tensor` provides a synthetic trace input.
- `train.py` — `train(config: TrainingConfig) -> Path` placeholder; creates a model and saves a `.pt` checkpoint but contains no real dataset loading or training loop.
- `export.py` — `export_onnx(checkpoint_path: Path, output_path: Path, config: TrainingConfig) -> Path` loads a checkpoint and traces to ONNX opset 17.

What is missing:

- Real dataset loading and augmentation (`dataset.py`).
- Evaluation metrics — accuracy, per-class precision/recall, confusion matrix (`eval.py`).
- TFLite export for Android (`export_tflite.py`).
- ONNX dynamic quantization for smaller Windows artifacts (`quantize.py`).
- A `pytest` test suite; there are currently no tests at all.

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

1. Add `pytest` to `pyproject.toml` `[project.optional-dependencies]` as `test = ["pytest"]`; add `[tool.pytest.ini_options]` pointing at `tests/`. No code changes yet — this is the harness.
2. `dataset.py` — implement `LocalImageDataset` and `load_dataset`; write `tests/test_dataset.py` with synthetic images using `tmp_path`. Run `pytest tests/test_dataset.py`.
3. `eval.py` — implement `evaluate` and `report`; write `tests/test_eval.py` with a trivially correct model on a two-label synthetic loader. Run `pytest tests/test_eval.py`.
4. Wire `dataset.py` and `eval.py` into `train.py` — replace the placeholder training loop with a real epoch loop over `load_dataset`, a validation call to `evaluate` after each epoch, and progress printing via `report`.
5. `quantize.py` — implement `quantize_onnx`; extend `tests/test_export.py` to verify the quantized model loads and has a smaller file size than the original. Run `pytest tests/test_export.py`.
6. `export_tflite.py` — implement TFLite export; add `tests/test_export.py` coverage for the TFLite path using a tiny synthetic model. Run `pytest tests/test_export.py`.

## What this does not cover

- Federated learning, server aggregation, or any cloud components — the pipeline is strictly local and self-contained.
- Sourcing or curating training data. The `data/` directory is gitignored; data curation is a separate out-of-repo process.
- Text NLP model training. Text classification is handled deterministically by `packages/text-policy`; see [../content-classification.md](../content-classification.md) (§ When To Add ML) for the gating rationale.
- iOS CoreML export — deferred until iOS platform work begins.
- Int8 static quantization — requires a calibration dataset and is noted in `export_tflite.py` comments for a later pass.
