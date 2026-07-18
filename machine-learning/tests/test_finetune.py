import io
import zipfile
from pathlib import Path

import pytest
import torch
from PIL import Image

from holy_blocker_ml.dataset import ZipImageDataset, build_transform
from holy_blocker_ml.finetune import parameter_groups, stratified_split
from holy_blocker_ml.labels import NEGATIVE_INDEX, POSITIVE_INDEX
from holy_blocker_ml.model import create_classifier, trainable_parameter_names, unfreeze_last_blocks


def png_bytes(colour: tuple[int, int, int], size: int = 32) -> bytes:
    buffer = io.BytesIO()
    Image.new("RGB", (size, size), colour).save(buffer, format="PNG")
    return buffer.getvalue()


@pytest.fixture
def corpus_zip(tmp_path: Path) -> Path:
    path = tmp_path / "corpus.zip"
    counts = {"neutral": 6, "drawings": 6, "sexy": 4, "hentai": 4, "porn": 6}
    with zipfile.ZipFile(path, "w") as archive:
        for index, (name, count) in enumerate(counts.items()):
            for item in range(count):
                archive.writestr(
                    f"root/{name}/{item}.png", png_bytes((index * 40, 90, 200 - index * 30))
                )
    return path


# --- zip-backed dataset -----------------------------------------------------


def test_dataset_indexes_every_image_without_unpacking(corpus_zip: Path, tmp_path: Path) -> None:
    before = set(tmp_path.rglob("*"))

    dataset = ZipImageDataset(corpus_zip, image_size=32, augment=False)

    assert len(dataset) == 26
    assert set(tmp_path.rglob("*")) == before  # nothing extracted


def test_dataset_items_are_tensors_with_binary_labels(corpus_zip: Path) -> None:
    dataset = ZipImageDataset(corpus_zip, image_size=32, augment=False)
    tensor, label = dataset[0]

    assert tensor.shape == (3, 32, 32)
    assert tensor.dtype == torch.float32
    assert label in {NEGATIVE_INDEX, POSITIVE_INDEX}


def test_dataset_label_counts_follow_the_policy(corpus_zip: Path) -> None:
    dataset = ZipImageDataset(corpus_zip, image_size=32, augment=False)

    labels = [label for _, label in dataset.samples]
    # hentai(4) + porn(6) are explicit; neutral(6) + drawings(6) + sexy(4) are safe.
    assert labels.count(POSITIVE_INDEX) == 10
    assert labels.count(NEGATIVE_INDEX) == 16


def test_dataset_works_through_a_dataloader(corpus_zip: Path) -> None:
    from torch.utils.data import DataLoader

    loader = DataLoader(ZipImageDataset(corpus_zip, image_size=32, augment=False), batch_size=8)

    batches = list(loader)
    assert sum(images.shape[0] for images, _ in batches) == 26
    assert batches[0][0].shape[1:] == (3, 32, 32)


def test_dataset_can_restrict_to_a_subset_of_indices(corpus_zip: Path) -> None:
    full = ZipImageDataset(corpus_zip, image_size=32, augment=False)
    subset = ZipImageDataset(corpus_zip, image_size=32, augment=False, indices=[0, 1, 2])

    assert len(subset) == 3
    assert subset.samples[0] == full.samples[0]


def test_unrecognised_layout_still_fails_loudly(tmp_path: Path) -> None:
    from holy_blocker_ml.features import ArchiveLayoutError

    path = tmp_path / "bad.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("root/mystery/0.png", png_bytes((1, 2, 3)))

    with pytest.raises(ArchiveLayoutError):
        ZipImageDataset(path, image_size=32, augment=False)


# --- splitting --------------------------------------------------------------


def test_split_is_stratified_across_source_classes() -> None:
    source = ["neutral"] * 100 + ["hentai"] * 50 + ["porn"] * 50

    train, val = stratified_split(source, val_fraction=0.2, seed=0)

    assert len(val) == 40
    for name in {"neutral", "hentai", "porn"}:
        in_val = sum(1 for i in val if source[i] == name)
        assert in_val == pytest.approx(0.2 * source.count(name), abs=1)


def test_split_is_disjoint_and_covers_everything() -> None:
    source = ["porn"] * 30 + ["neutral"] * 30

    train, val = stratified_split(source, val_fraction=0.25, seed=1)

    assert set(train) & set(val) == set()
    assert sorted(train + val) == list(range(60))


def test_split_is_deterministic_for_a_given_seed() -> None:
    source = ["porn"] * 20 + ["neutral"] * 20

    assert stratified_split(source, 0.3, seed=7) == stratified_split(source, 0.3, seed=7)
    assert stratified_split(source, 0.3, seed=7) != stratified_split(source, 0.3, seed=8)


# --- unfreezing and learning rates ------------------------------------------


def test_full_unfreeze_trains_everything() -> None:
    model = unfreeze_last_blocks(create_classifier(pretrained=False), count=None)

    names = trainable_parameter_names(model)
    assert any(n.startswith("features.0") for n in names)
    assert any(n.startswith("classifier.") for n in names)


def test_partial_unfreeze_keeps_early_blocks_frozen() -> None:
    model = unfreeze_last_blocks(create_classifier(pretrained=False), count=3)

    names = trainable_parameter_names(model)
    assert not any(n.startswith("features.0.") for n in names)
    assert any(n.startswith("classifier.") for n in names)
    # The last feature block must be trainable.
    last = max(int(n.split(".")[1]) for n in dict(model.named_parameters()) if n.startswith("features."))
    assert any(n.startswith(f"features.{last}.") for n in names)


def test_zero_unfreeze_is_head_only() -> None:
    model = unfreeze_last_blocks(create_classifier(pretrained=False), count=0)

    assert all(n.startswith("classifier.") for n in trainable_parameter_names(model))


def test_backbone_gets_a_lower_learning_rate_than_the_head() -> None:
    model = unfreeze_last_blocks(create_classifier(pretrained=False), count=None)

    groups = parameter_groups(model, backbone_lr=1e-4, head_lr=1e-3)

    assert len(groups) == 2
    by_name = {g["name"]: g for g in groups}
    assert by_name["backbone"]["lr"] == 1e-4
    assert by_name["head"]["lr"] == 1e-3
    # Pretrained features need gentler updates than a randomly-initialised head,
    # or the first steps wash out what the backbone already knows.
    assert by_name["backbone"]["lr"] < by_name["head"]["lr"]


def test_parameter_groups_exclude_frozen_tensors() -> None:
    model = unfreeze_last_blocks(create_classifier(pretrained=False), count=0)

    groups = parameter_groups(model, backbone_lr=1e-4, head_lr=1e-3)

    assert all(all(p.requires_grad for p in g["params"]) for g in groups)
    assert sum(len(g["params"]) for g in groups) == len(trainable_parameter_names(model))
