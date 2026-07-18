"""Shared fixtures.

Every fixture here builds synthetic images from random noise. The repo must
never carry real evaluation samples — see AGENTS.md ("Test-First Rule For
Logic") and docs/decisions/learning-from-feedback.md (Evaluation).
"""

from pathlib import Path

import numpy as np
import pytest
from PIL import Image


def write_noise_image(path: Path, size: int = 64, seed: int = 0) -> Path:
    """Write a deterministic random-noise RGB PNG at `path`."""
    path.parent.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(seed)
    pixels = rng.integers(0, 256, size=(size, size, 3), dtype=np.uint8)
    Image.fromarray(pixels, mode="RGB").save(path)
    return path


@pytest.fixture
def labelled_tree(tmp_path: Path) -> Path:
    """A two-label ImageFolder-style tree: two images per label.

    Sorted directory order fixes the label mapping as clean=0, explicit=1,
    which is the convention the whole package relies on.
    """
    root = tmp_path / "train"
    for seed, name in enumerate(("clean", "explicit")):
        for index in range(2):
            write_noise_image(root / name / f"{index}.png", seed=seed * 10 + index)
    return root


@pytest.fixture
def single_kind_tree(tmp_path: Path) -> Path:
    """A flat directory of images with no label subdirectories.

    This is the shape an evaluation corpus takes: every item is the same class
    by construction, so the label lives in the corpus kind rather than on disk.
    """
    root = tmp_path / "corpus"
    for index in range(4):
        write_noise_image(root / f"{index}.png", seed=100 + index)
    return root
