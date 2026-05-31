import torch
from torch import nn
from torchvision.models import MobileNet_V3_Small_Weights, mobilenet_v3_small


def create_classifier(class_count: int) -> nn.Module:
    weights = MobileNet_V3_Small_Weights.DEFAULT
    model = mobilenet_v3_small(weights=weights)
    input_features = model.classifier[-1].in_features
    model.classifier[-1] = nn.Linear(input_features, class_count)
    return model


def example_input(image_size: int) -> torch.Tensor:
    return torch.randn(1, 3, image_size, image_size)
