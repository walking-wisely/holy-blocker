import torch
from torch import nn
from torchvision.models import MobileNet_V3_Small_Weights, mobilenet_v3_small

from holy_blocker_ml.labels import BINARY_LABELS


def create_classifier(class_count: int = len(BINARY_LABELS), pretrained: bool = True) -> nn.Module:
    """MobileNetV3-Small with the final layer resized to `class_count`.

    `pretrained=False` skips the ImageNet weight download, which keeps tests
    offline. Real training should leave it on — the point of this backbone is
    that fine-tuning it needs far less data than training from scratch.
    """
    weights = MobileNet_V3_Small_Weights.DEFAULT if pretrained else None
    model = mobilenet_v3_small(weights=weights)
    input_features = model.classifier[-1].in_features
    model.classifier[-1] = nn.Linear(input_features, class_count)
    return model


def example_input(image_size: int) -> torch.Tensor:
    return torch.randn(1, 3, image_size, image_size)
