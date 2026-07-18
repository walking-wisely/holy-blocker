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
