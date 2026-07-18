from pathlib import Path

import pytest
import torch

from holy_blocker_ml.dataset import LocalImageDataset, load_dataset


def test_labels_follow_sorted_directory_order(labelled_tree: Path) -> None:
    dataset = LocalImageDataset(labelled_tree, image_size=32, augment=False)

    assert dataset.classes == ["clean", "explicit"]
    assert dataset.class_to_idx == {"clean": 0, "explicit": 1}


def test_length_counts_every_image(labelled_tree: Path) -> None:
    dataset = LocalImageDataset(labelled_tree, image_size=32, augment=False)

    assert len(dataset) == 4


def test_getitem_returns_chw_tensor_and_int_label(labelled_tree: Path) -> None:
    dataset = LocalImageDataset(labelled_tree, image_size=32, augment=False)

    image, label = dataset[0]

    assert image.shape == (3, 32, 32)
    assert image.dtype == torch.float32
    assert isinstance(label, int)
    assert label in (0, 1)


def test_every_label_is_represented(labelled_tree: Path) -> None:
    dataset = LocalImageDataset(labelled_tree, image_size=32, augment=False)

    labels = sorted(dataset[i][1] for i in range(len(dataset)))

    assert labels == [0, 0, 1, 1]


def test_validation_transform_is_deterministic(labelled_tree: Path) -> None:
    dataset = LocalImageDataset(labelled_tree, image_size=32, augment=False)

    first, _ = dataset[0]
    second, _ = dataset[0]

    assert torch.equal(first, second)


def test_augmented_transform_varies_between_reads(labelled_tree: Path) -> None:
    dataset = LocalImageDataset(labelled_tree, image_size=32, augment=True)
    torch.manual_seed(0)
    first, _ = dataset[0]
    torch.manual_seed(1)
    second, _ = dataset[0]

    assert not torch.equal(first, second)


def test_missing_root_is_rejected(tmp_path: Path) -> None:
    with pytest.raises(FileNotFoundError, match="does not exist"):
        LocalImageDataset(tmp_path / "absent", image_size=32, augment=False)


def test_root_without_label_directories_is_rejected(tmp_path: Path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()

    with pytest.raises(ValueError, match="no label subdirectories"):
        LocalImageDataset(empty, image_size=32, augment=False)


def test_load_dataset_yields_batches(labelled_tree: Path) -> None:
    loader = load_dataset(labelled_tree, image_size=32, augment=False, batch_size=2)

    images, labels = next(iter(loader))

    assert images.shape == (2, 3, 32, 32)
    assert labels.shape == (2,)


def test_load_dataset_covers_the_whole_tree(labelled_tree: Path) -> None:
    loader = load_dataset(labelled_tree, image_size=32, augment=False, batch_size=2)

    seen = sum(len(labels) for _, labels in loader)

    assert seen == 4
