"""ONNX dynamic quantization for the Windows artifact.

Dynamic range quantization stores weights as int8 and picks activation scales
at inference time, so no calibration dataset is needed — unlike the static int8
path deferred in `export_tflite.py`.

Reference documents:
- ONNX Runtime quantization: https://onnxruntime.ai/docs/performance/model-optimizations/quantization.html
"""

from pathlib import Path

QUANTIZED_NAME = "baseline-v0.int8.onnx"


def quantize_onnx(model_path: Path, output_path: Path) -> Path:
    """Apply dynamic range quantization to an ONNX model. Returns `output_path`."""
    from onnxruntime.quantization import QuantType, quantize_dynamic

    output_path.parent.mkdir(parents=True, exist_ok=True)
    quantize_dynamic(
        model_input=str(model_path),
        model_output=str(output_path),
        # QInt8 over QUInt8: MobileNetV3 weights are roughly zero-centred, and
        # signed quantization avoids the zero-point skew unsigned would add.
        weight_type=QuantType.QInt8,
    )
    return output_path


def main() -> None:
    from holy_blocker_ml.config import TrainingConfig

    config = TrainingConfig()
    source = config.output_dir / "baseline-v0.onnx"
    artifact = quantize_onnx(source, config.output_dir / QUANTIZED_NAME)
    print(f"quantized ONNX artifact: {artifact} ({artifact.stat().st_size / 1024:.0f} KB)")
