import pytest
import torch

from holy_blocker_ml.labels import BINARY_LABELS, EXPLICIT, POSITIVE_INDEX, SAFE
from holy_blocker_ml.model import create_classifier, example_input


@pytest.mark.parametrize("class_count", [2, 3, 5])
def test_head_width_follows_the_requested_class_count(class_count: int) -> None:
    model = create_classifier(class_count=class_count, pretrained=False).eval()

    with torch.no_grad():
        logits = model(example_input(64))

    assert logits.shape == (1, class_count)


def test_defaults_to_the_binary_label_set() -> None:
    model = create_classifier(pretrained=False).eval()

    with torch.no_grad():
        assert model(example_input(64)).shape == (1, len(BINARY_LABELS))


def test_accepts_a_batch() -> None:
    model = create_classifier(pretrained=False).eval()

    with torch.no_grad():
        assert model(torch.randn(4, 3, 64, 64)).shape == (4, len(BINARY_LABELS))


def test_example_input_is_a_square_nchw_batch_of_one() -> None:
    assert example_input(224).shape == (1, 3, 224, 224)


def test_label_ordering_is_pinned_against_accidental_reordering() -> None:
    # Guards the model contract: exported artifacts bake this order in, and the
    # Android/Windows runtimes index into it.
    assert BINARY_LABELS == (SAFE, EXPLICIT)
    assert POSITIVE_INDEX == 1


def test_freeze_backbone_leaves_only_the_head_trainable() -> None:
    from holy_blocker_ml.model import freeze_backbone

    model = freeze_backbone(create_classifier(pretrained=False))

    trainable = {n for n, p in model.named_parameters() if p.requires_grad}
    assert trainable
    assert all(name.startswith("classifier.") for name in trainable)
    assert not any(name.startswith("features.") for name in trainable)


def test_feature_extractor_emits_the_documented_width() -> None:
    from holy_blocker_ml.model import BACKBONE_FEATURE_DIM, create_feature_extractor

    backbone = create_feature_extractor(pretrained=False)

    with torch.no_grad():
        assert backbone(torch.randn(3, 3, 64, 64)).shape == (3, BACKBONE_FEATURE_DIM)


def test_extracted_features_feed_the_classifier_head_directly() -> None:
    from holy_blocker_ml.model import create_feature_extractor

    model = create_classifier(pretrained=False).eval()
    backbone = create_feature_extractor(pretrained=False)
    backbone.features, backbone.avgpool = model.features, model.avgpool
    images = torch.randn(2, 3, 64, 64)

    with torch.no_grad():
        # Head-on-features must equal the full forward pass.
        torch.testing.assert_close(model.classifier(backbone(images)), model(images))
