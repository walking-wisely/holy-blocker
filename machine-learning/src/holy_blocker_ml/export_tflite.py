"""TFLite export for Android. (Windows uses ONNX — see `export.py`.)

Conversion goes straight from a torch `ExportedProgram` to a LiteRT/TFLite
flatbuffer via `litert-torch`, the renamed successor to `ai-edge-torch`. This
replaces the plan's original `torch -> ONNX -> TF SavedModel -> TFLite` route:
that path needs `tensorflow`, which drags in a second full framework and has no
wheel on current Python builds. `litert-torch` is the converter Google now
points at for PyTorch sources, so `tensorflow` is no longer a dependency.

Reference documents:
- LiteRT (ex-TFLite) PyTorch conversion: https://ai.google.dev/edge/litert/models/convert_pytorch
- litert-torch (formerly ai-edge-torch): https://github.com/google-ai-edge/ai-edge-torch
"""

from pathlib import Path

import torch
from torch import nn

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.model import create_classifier, example_input

TFLITE_NAME = "baseline-v0.tflite"


def convert_module(model: nn.Module, sample_input: torch.Tensor, output_path: Path) -> Path:
    """Convert an nn.Module to a TFLite flatbuffer at `output_path`.

    Quantization is deliberately not applied. `litert-torch` exposes PT2E
    quantization, which is static int8 and needs a calibration dataset — out of
    scope until real training data exists. The float32 MobileNetV3-Small
    artifact is ~6 MB, comfortably inside `TrainingConfig.max_model_mb`.
    """
    import litert_torch  # imported lazily: an optional `tflite` extra

    model.eval()
    output_path.parent.mkdir(parents=True, exist_ok=True)

    # sample_args must be a tuple of positional args matching forward().
    converted = litert_torch.convert(model, (sample_input,))
    converted.export(str(output_path))
    return output_path


def export_tflite(checkpoint_path: Path, output_path: Path, config: TrainingConfig) -> Path:
    """Convert a saved checkpoint to TFLite for the Android runtime."""
    checkpoint = torch.load(checkpoint_path, map_location="cpu", weights_only=False)

    # pretrained=False: the checkpoint supplies every weight, so skip the download.
    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
    model.load_state_dict(checkpoint["model_state"])

    return convert_module(model, example_input(config.image_size), output_path)


def main() -> None:
    config = TrainingConfig()
    checkpoint_path = config.output_dir / "baseline-v0.pt"
    artifact = export_tflite(checkpoint_path, config.output_dir / TFLITE_NAME, config)
    size_mb = artifact.stat().st_size / (1024 * 1024)
    print(f"exported TFLite artifact: {artifact} ({size_mb:.2f} MB)")
