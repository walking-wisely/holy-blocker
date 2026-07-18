from pathlib import Path

import pytest
import torch

from holy_blocker_ml.dataset import LocalImageDataset, load_dataset
from holy_blocker_ml.labels import EXPLICIT, POSITIVE_INDEX, SAFE


def test_discovers_every_image_under_each_label(image_tree: Path) -> None:
    dataset = LocalImageDataset(image_tree, image_size=32, augment=False)
    assert len(dataset) == 5


def test_label_indices_follow_the_pinned_ordering_not_alphabetical(image_tree: Path) -> None:
    dataset = LocalImageDataset(image_tree, image_size=32, augment=False)

    # Alphabetically "explicit" sorts first; the pinned contract puts it last.
    assert dataset.classes == [SAFE, EXPLICIT]
    labels = {dataset[i][1] for i in range(len(dataset))}
    assert labels == {0, 1}

    explicit_samples = [dataset[i] for i in range(len(dataset)) if dataset.samples[i][1] == POSITIVE_INDEX]
    assert len(explicit_samples) == 2


def test_items_are_normalized_chw_float_tensors(image_tree: Path) -> None:
    dataset = LocalImageDataset(image_tree, image_size=32, augment=False)
    tensor, label = dataset[0]

    assert tensor.shape == (3, 32, 32)
    assert tensor.dtype == torch.float32
    assert isinstance(label, int)
    # ImageNet normalization pushes values outside [0, 1].
    assert tensor.min() < 0.0


def test_augmentation_makes_repeat_reads_differ(image_tree: Path) -> None:
    plain = LocalImageDataset(image_tree, image_size=32, augment=False)
    assert torch.equal(plain[0][0], plain[0][0])

    torch.manual_seed(0)
    augmented = LocalImageDataset(image_tree, image_size=32, augment=True)
    reads = [augmented[0][0] for _ in range(8)]
    assert any(not torch.equal(reads[0], other) for other in reads[1:])


def test_unknown_label_directory_is_rejected(tmp_path: Path) -> None:
    (tmp_path / "maybe").mkdir()
    from tests.conftest import write_image

    write_image(tmp_path / "maybe" / "0.png", (10, 10, 10))

    with pytest.raises(ValueError, match="maybe"):
        LocalImageDataset(tmp_path, image_size=32, augment=False)


def test_empty_root_is_rejected(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="no images"):
        LocalImageDataset(tmp_path, image_size=32, augment=False)


def test_loader_batches_and_preserves_total_count(image_tree: Path) -> None:
    loader = load_dataset(image_tree, image_size=32, augment=False, batch_size=2)

    batches = list(loader)
    assert len(batches) == 3  # 5 samples at batch size 2
    assert sum(images.shape[0] for images, _ in batches) == 5
    assert batches[0][0].shape[1:] == (3, 32, 32)
