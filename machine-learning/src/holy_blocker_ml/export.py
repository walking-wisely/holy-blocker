from pathlib import Path

import torch

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.model import create_classifier, example_input


def export_onnx(checkpoint_path: Path, output_path: Path, config: TrainingConfig) -> Path:
    checkpoint = torch.load(checkpoint_path, map_location="cpu")
    model = create_classifier(class_count=2)
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
    )
    return output_path


def main() -> None:
    config = TrainingConfig()
    checkpoint_path = config.output_dir / "baseline-v0.pt"
    onnx_path = config.output_dir / "baseline-v0.onnx"
    artifact = export_onnx(checkpoint_path, onnx_path, config)
    print(f"exported ONNX artifact: {artifact}")
