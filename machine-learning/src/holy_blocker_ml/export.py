from pathlib import Path

import torch

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.model import create_classifier, example_input


def export_onnx(checkpoint_path: Path, output_path: Path, config: TrainingConfig) -> Path:
    checkpoint = torch.load(checkpoint_path, map_location="cpu", weights_only=False)

    # pretrained=False: the checkpoint supplies every weight, so skip the download.
    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
    model.load_state_dict(checkpoint["model_state"])
    model.eval()

    output_path.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        model,
        example_input(config.image_size),
        output_path,
        input_names=["image"],
        output_names=["logits"],
        dynamic_axes={"image": {0: "batch"}, "logits": {0: "batch"}},
        opset_version=17,
        # dynamo=False pins the legacy TorchScript exporter. torch>=2.9 defaults
        # to the dynamo exporter, whose MobileNetV3 graph carries inconsistent
        # shape metadata in the classifier: onnxruntime's shape inference (run
        # by quantize_dynamic) rejects it with "Inferred shape and existing
        # shape differ in dimension 0: (576) vs (1024)". Reproduced on every
        # opset from 17 to 21 and at both 32px and 224px inputs; the TorchScript
        # graph quantizes cleanly. Revisit when the dynamo path is fixed, since
        # the legacy exporter is deprecated and will eventually be removed.
        dynamo=False,
    )
    return output_path


def main() -> None:
    config = TrainingConfig()
    checkpoint_path = config.output_dir / "baseline-v0.pt"
    onnx_path = config.output_dir / "baseline-v0.onnx"
    artifact = export_onnx(checkpoint_path, onnx_path, config)
    print(f"exported ONNX artifact: {artifact}")
