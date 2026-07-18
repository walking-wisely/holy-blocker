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


#: Width of the MobileNetV3-Small backbone output, i.e. the input width of
#: `model.classifier`. Feature artifacts are only interchangeable with heads
#: built for this dimension.
BACKBONE_FEATURE_DIM = 576


class BackboneFeatures(nn.Module):
    """MobileNetV3-Small up to (but excluding) the classifier head.

    Emits the 576-d vector that `create_classifier(...).classifier` consumes, so
    a head can be trained or evaluated against cached features without the
    source images ever being needed again.
    """

    def __init__(self, backbone: nn.Module) -> None:
        super().__init__()
        self.features = backbone.features
        self.avgpool = backbone.avgpool

    def forward(self, images: torch.Tensor) -> torch.Tensor:
        return torch.flatten(self.avgpool(self.features(images)), 1)


def create_feature_extractor(pretrained: bool = True) -> BackboneFeatures:
    """Backbone-only model used to precompute reusable feature vectors."""
    return BackboneFeatures(create_classifier(pretrained=pretrained)).eval()


def freeze_backbone(model: nn.Module) -> nn.Module:
    """Freeze everything except the classifier head.

    This is what makes cached feature artifacts durable. If the backbone keeps
    training, every checkpoint produces different features and a stored artifact
    silently stops matching the model it is scoring — so extraction would have
    to be re-run against the source images for each new checkpoint, which is
    exactly what caching features is meant to avoid.
    """
    for name, parameter in model.named_parameters():
        parameter.requires_grad = name.startswith("classifier.")
    return model


def trainable_parameter_names(model: nn.Module) -> list[str]:
    """Names of parameters that will receive gradients."""
    return [name for name, parameter in model.named_parameters() if parameter.requires_grad]


def unfreeze_last_blocks(model: nn.Module, count: int | None = None) -> nn.Module:
    """Unfreeze the classifier plus the last `count` feature blocks.

    `count=None` unfreezes everything, `count=0` is head-only (equivalent to
    `freeze_backbone`). Partial unfreezing is the middle option: early blocks
    hold generic edge and texture filters that transfer fine, while the later
    blocks carry the semantic features that need to change for a distinction
    ImageNet never had to make.
    """
    block_indices = sorted(
        {int(name.split(".")[1]) for name, _ in model.named_parameters() if name.startswith("features.")}
    )
    if count is None:
        keep = set(block_indices)
    else:
        keep = set(block_indices[len(block_indices) - count :]) if count else set()

    for name, parameter in model.named_parameters():
        if name.startswith("classifier."):
            parameter.requires_grad = True
        elif name.startswith("features."):
            parameter.requires_grad = int(name.split(".")[1]) in keep
        else:
            parameter.requires_grad = count is None
    return model
