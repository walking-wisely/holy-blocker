from pathlib import Path

import pytest
import torch

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.eval import collect_predictions, misclassified
from holy_blocker_ml.harness import evaluate_checkpoint, format_examples, load_checkpoint
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.model import create_classifier


@pytest.fixture
def checkpoint(tmp_path: Path) -> Path:
    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
    path = tmp_path / "ckpt.pt"
    torch.save({"model_state": model.state_dict(), "labels": list(BINARY_LABELS)}, path)
    return path


def test_load_checkpoint_returns_an_eval_mode_model(checkpoint: Path) -> None:
    model = load_checkpoint(checkpoint)

    assert not model.training
    assert model(torch.randn(1, 3, 64, 64)).shape == (1, len(BINARY_LABELS))


def test_load_checkpoint_rejects_inverted_label_order(tmp_path: Path) -> None:
    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
    path = tmp_path / "inverted.pt"
    torch.save({"model_state": model.state_dict(), "labels": list(reversed(BINARY_LABELS))}, path)

    with pytest.raises(ValueError, match="inverted"):
        load_checkpoint(path)


def test_evaluate_checkpoint_scores_every_image_in_the_tree(
    checkpoint: Path, image_tree: Path
) -> None:
    config = TrainingConfig(image_size=32, batch_size=2)

    result, predictions = evaluate_checkpoint(checkpoint, image_tree, config)

    assert result.sample_count == 5
    assert len(predictions) == 5
    # An untrained head is arbitrary, but every sample must land in the matrix.
    assert sum(sum(row) for row in result.confusion_matrix) == 5
    assert result.false_positives + result.true_negatives == 3  # 3 safe images
    assert result.false_negatives + result.true_positives == 2  # 2 explicit images


def test_misclassified_reports_real_file_paths(checkpoint: Path, image_tree: Path) -> None:
    config = TrainingConfig(image_size=32, batch_size=2)
    _, predictions = evaluate_checkpoint(checkpoint, image_tree, config)

    # Force every sample to count as a false positive/negative by thresholding
    # at 0.0, which predicts the positive class for everything.
    false_positives, false_negatives = misclassified(predictions, threshold=0.0)

    assert len(false_positives) == 3  # all safe images now predicted explicit
    assert len(false_negatives) == 0
    assert all(path.exists() for path, _ in false_positives)
    assert all(path.parent.name == "safe" for path, _ in false_positives)


def test_format_examples_is_empty_when_nothing_is_misclassified() -> None:
    from torch.utils.data import DataLoader, TensorDataset

    from torch import nn

    class Perfect(nn.Module):
        def forward(self, batch: torch.Tensor) -> torch.Tensor:
            return batch

    loader = DataLoader(TensorDataset(torch.tensor([[6.0, 0.0]]), torch.tensor([0])))
    predictions = collect_predictions(Perfect(), loader)

    assert format_examples(predictions, threshold=0.5, limit=5) == ""
