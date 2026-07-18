from pathlib import Path

import numpy as np
import pytest
import torch
from torch import nn

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.export import export_onnx
from holy_blocker_ml.export_tflite import convert_module, export_tflite
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.model import create_classifier
from holy_blocker_ml.quantize import quantize_onnx

litert_torch = pytest.importorskip("litert_torch", reason="requires the 'tflite' extra")


class TinyClassifier(nn.Module):
    """Stand-in for MobileNetV3 — same input/output contract, converts in ~1s."""

    def __init__(self, class_count: int = len(BINARY_LABELS)) -> None:
        super().__init__()
        self.features = nn.Conv2d(3, 4, kernel_size=3, stride=2, padding=1)
        self.pool = nn.AdaptiveAvgPool2d(1)
        self.head = nn.Linear(4, class_count)

    def forward(self, images: torch.Tensor) -> torch.Tensor:
        return self.head(self.pool(torch.relu(self.features(images))).flatten(1))


@pytest.fixture
def config() -> TrainingConfig:
    return TrainingConfig(image_size=32)


@pytest.fixture
def checkpoint(tmp_path: Path) -> Path:
    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
    path = tmp_path / "baseline-v0.pt"
    torch.save({"model_state": model.state_dict(), "labels": list(BINARY_LABELS)}, path)
    return path


def run_tflite(model_path: Path, image_size: int) -> np.ndarray:
    """Load a flatbuffer in the on-device runtime and run one inference."""
    from ai_edge_litert.interpreter import Interpreter

    interpreter = Interpreter(model_path=str(model_path))
    interpreter.allocate_tensors()
    input_detail = interpreter.get_input_details()[0]
    output_detail = interpreter.get_output_details()[0]

    sample = np.random.randn(*input_detail["shape"]).astype(np.float32)
    interpreter.set_tensor(input_detail["index"], sample)
    interpreter.invoke()
    return interpreter.get_tensor(output_detail["index"])


def test_tflite_export_produces_a_runnable_flatbuffer(tmp_path: Path) -> None:
    output = tmp_path / "tiny.tflite"

    convert_module(TinyClassifier(), torch.randn(1, 3, 32, 32), output)

    assert output.exists() and output.stat().st_size > 0
    assert run_tflite(output, 32).shape == (1, len(BINARY_LABELS))


def test_tflite_output_matches_torch_within_float_tolerance(tmp_path: Path) -> None:
    from ai_edge_litert.interpreter import Interpreter

    model = TinyClassifier().eval()
    sample = torch.randn(1, 3, 32, 32)
    with torch.no_grad():
        expected = model(sample).numpy()

    output = tmp_path / "parity.tflite"
    convert_module(model, torch.randn(1, 3, 32, 32), output)

    interpreter = Interpreter(model_path=str(output))
    interpreter.allocate_tensors()
    input_detail = interpreter.get_input_details()[0]
    interpreter.set_tensor(input_detail["index"], sample.numpy())
    interpreter.invoke()
    actual = interpreter.get_tensor(interpreter.get_output_details()[0]["index"])

    np.testing.assert_allclose(actual, expected, rtol=1e-4, atol=1e-4)


@pytest.mark.slow
def test_real_checkpoint_exports_within_the_size_budget(
    checkpoint: Path, tmp_path: Path, config: TrainingConfig
) -> None:
    output = tmp_path / "baseline-v0.tflite"

    export_tflite(checkpoint, output, config)

    size_mb = output.stat().st_size / (1024 * 1024)
    assert size_mb < config.max_model_mb, f"artifact is {size_mb:.1f} MB"
    assert run_tflite(output, config.image_size).shape == (1, len(BINARY_LABELS))


def test_onnx_export_is_loadable_and_runnable(
    checkpoint: Path, tmp_path: Path, config: TrainingConfig
) -> None:
    import onnx
    import onnxruntime

    output = tmp_path / "baseline-v0.onnx"
    export_onnx(checkpoint, output, config)

    onnx.checker.check_model(onnx.load(output))
    session = onnxruntime.InferenceSession(str(output))
    logits = session.run(None, {"image": np.random.randn(1, 3, 32, 32).astype(np.float32)})[0]
    assert logits.shape == (1, len(BINARY_LABELS))


def test_onnx_export_accepts_a_dynamic_batch_dimension(
    checkpoint: Path, tmp_path: Path, config: TrainingConfig
) -> None:
    import onnxruntime

    output = tmp_path / "dynamic.onnx"
    export_onnx(checkpoint, output, config)

    session = onnxruntime.InferenceSession(str(output))
    batch = np.random.randn(4, 3, 32, 32).astype(np.float32)
    assert session.run(None, {"image": batch})[0].shape == (4, len(BINARY_LABELS))


def test_quantization_shrinks_the_onnx_artifact_and_keeps_it_runnable(
    checkpoint: Path, tmp_path: Path, config: TrainingConfig
) -> None:
    import onnxruntime

    source = tmp_path / "baseline-v0.onnx"
    export_onnx(checkpoint, source, config)
    quantized = quantize_onnx(source, tmp_path / "baseline-v0.int8.onnx")

    assert quantized.stat().st_size < source.stat().st_size

    session = onnxruntime.InferenceSession(str(quantized))
    logits = session.run(None, {"image": np.random.randn(1, 3, 32, 32).astype(np.float32)})[0]
    assert logits.shape == (1, len(BINARY_LABELS))
