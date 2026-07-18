"""Tests for the evaluation-corpus harness.

The recall corpus is a held-out NSFW benchmark that must stay out of the repo
and out of the training/feedback loop
(docs/decisions/learning-from-feedback.md, Evaluation). These tests therefore
only ever touch synthetic noise images; they verify the *plumbing* that will
later be pointed at a real local corpus.
"""

from pathlib import Path

import pytest
import torch
from torch import Tensor, nn

from holy_blocker_ml.corpus import (
    CorpusKind,
    CorpusSpec,
    explicit_prediction_rate,
    load_corpus,
    measure_corpus,
)


class ConstantModel(nn.Module):
    """Always predicts `label`, whatever the input."""

    def __init__(self, label: int) -> None:
        super().__init__()
        self.label = label

    def forward(self, x: Tensor) -> Tensor:
        logits = torch.zeros(x.shape[0], 2)
        logits[:, self.label] = 1.0
        return logits


class AlternatingModel(nn.Module):
    """Predicts explicit for exactly half of a batch, clean for the rest."""

    def forward(self, x: Tensor) -> Tensor:
        logits = torch.zeros(x.shape[0], 2)
        for row in range(x.shape[0]):
            logits[row, row % 2] = 1.0
        return logits


def spec_for(root: Path, kind: CorpusKind) -> CorpusSpec:
    return CorpusSpec(name="synthetic", root=root, kind=kind)


def test_corpus_loads_a_flat_directory(single_kind_tree: Path) -> None:
    loader = load_corpus(
        spec_for(single_kind_tree, CorpusKind.BENIGN), image_size=32, batch_size=2
    )

    seen = sum(batch.shape[0] for batch in loader)

    assert seen == 4


def test_corpus_batches_are_image_tensors(single_kind_tree: Path) -> None:
    loader = load_corpus(
        spec_for(single_kind_tree, CorpusKind.BENIGN), image_size=32, batch_size=2
    )

    batch = next(iter(loader))

    assert batch.shape == (2, 3, 32, 32)


def test_missing_corpus_names_the_path_and_explains_it_is_local(tmp_path: Path) -> None:
    spec = spec_for(tmp_path / "absent", CorpusKind.EXPLICIT)

    with pytest.raises(FileNotFoundError) as excinfo:
        load_corpus(spec, image_size=32, batch_size=2)

    message = str(excinfo.value)
    assert "absent" in message
    assert "gitignored" in message.lower()


def test_empty_corpus_is_rejected(tmp_path: Path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()

    with pytest.raises(ValueError, match="no images"):
        load_corpus(spec_for(empty, CorpusKind.BENIGN), image_size=32, batch_size=2)


def test_explicit_rate_is_one_when_everything_is_flagged(single_kind_tree: Path) -> None:
    loader = load_corpus(
        spec_for(single_kind_tree, CorpusKind.BENIGN), image_size=32, batch_size=2
    )

    assert explicit_prediction_rate(ConstantModel(label=1), loader) == 1.0


def test_explicit_rate_is_zero_when_nothing_is_flagged(single_kind_tree: Path) -> None:
    loader = load_corpus(
        spec_for(single_kind_tree, CorpusKind.BENIGN), image_size=32, batch_size=2
    )

    assert explicit_prediction_rate(ConstantModel(label=0), loader) == 0.0


def test_explicit_rate_is_a_fraction(single_kind_tree: Path) -> None:
    loader = load_corpus(
        spec_for(single_kind_tree, CorpusKind.BENIGN), image_size=32, batch_size=2
    )

    assert explicit_prediction_rate(AlternatingModel(), loader) == 0.5


def test_benign_corpus_measures_false_positive_rate(single_kind_tree: Path) -> None:
    # Every item is clean by construction, so a flag is a false positive.
    measurement = measure_corpus(
        ConstantModel(label=1),
        spec_for(single_kind_tree, CorpusKind.BENIGN),
        image_size=32,
        batch_size=2,
    )

    assert measurement.metric_name == "false_positive_rate"
    assert measurement.value == 1.0
    assert measurement.item_count == 4


def test_explicit_corpus_measures_recall(single_kind_tree: Path) -> None:
    # Every item is explicit by construction, so a flag is a true positive.
    measurement = measure_corpus(
        ConstantModel(label=1),
        spec_for(single_kind_tree, CorpusKind.EXPLICIT),
        image_size=32,
        batch_size=2,
    )

    assert measurement.metric_name == "recall"
    assert measurement.value == 1.0


def test_missed_explicit_items_lower_recall(single_kind_tree: Path) -> None:
    measurement = measure_corpus(
        ConstantModel(label=0),
        spec_for(single_kind_tree, CorpusKind.EXPLICIT),
        image_size=32,
        batch_size=2,
    )

    # Every explicit item was let through: this is the false-negative case.
    assert measurement.value == 0.0
