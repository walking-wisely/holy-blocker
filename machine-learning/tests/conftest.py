"""Shared synthetic fixtures.

No real imagery ever enters the repo or CI — see AGENTS.md ("Test-First Rule
For Logic") and docs/decisions/learning-from-feedback.md (Evaluation). Every
fixture here builds images on the fly.

Two shapes are needed:

- a **labelled tree** (`root/<label>/<image>`) for training data, where the
  label comes from the directory name;
- a **flat directory** for a single-class evaluation corpus, where every item
  is the same class by construction and the label comes from the corpus kind.
"""

from pathlib import Path

import numpy as np
import pytest
from PIL import Image

from holy_blocker_ml.labels import EXPLICIT, SAFE


def write_image(path: Path, colour: tuple[int, int, int], size: int = 64) -> Path:
    """Write a solid-colour PNG — enough to exercise loading and batching."""
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.new("RGB", (size, size), colour).save(path)
    return path


def write_noise_image(path: Path, size: int = 64, seed: int = 0) -> Path:
    """Write a deterministic random-noise RGB PNG at `path`.

    Preferred where a test needs images that are distinguishable but carry no
    structure a model could latch onto.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(seed)
    pixels = rng.integers(0, 256, size=(size, size, 3), dtype=np.uint8)
    Image.fromarray(pixels, mode="RGB").save(path)
    return path


@pytest.fixture
def image_tree(tmp_path: Path) -> Path:
    """A two-label dataset: 3 `safe` images and 2 `explicit` images."""
    for index, colour in enumerate([(0, 128, 0), (0, 160, 0), (0, 200, 0)]):
        write_image(tmp_path / SAFE / f"{index}.png", colour)
    for index, colour in enumerate([(200, 0, 0), (160, 0, 0)]):
        write_image(tmp_path / EXPLICIT / f"{index}.png", colour)
    return tmp_path


@pytest.fixture
def labelled_tree(tmp_path: Path) -> Path:
    """An ImageFolder-style tree with two images per label.

    Directory names come from `labels.BINARY_LABELS`, which pins the mapping
    rather than deriving it from sorted order.
    """
    root = tmp_path / "train"
    for seed, name in enumerate((SAFE, EXPLICIT)):
        for index in range(2):
            write_noise_image(root / name / f"{index}.png", seed=seed * 10 + index)
    return root


@pytest.fixture
def single_kind_tree(tmp_path: Path) -> Path:
    """A flat directory of images with no label subdirectories.

    The shape an evaluation corpus takes: every item is the same class by
    construction, so the label lives in the corpus kind rather than on disk.
    """
    root = tmp_path / "corpus"
    for index in range(4):
        write_noise_image(root / f"{index}.png", seed=100 + index)
    return root
