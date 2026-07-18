"""Shared synthetic fixtures.

No real imagery ever enters the repo or CI. Every fixture here builds solid
colour PNGs on the fly, which is enough to exercise loading, batching, and
metric arithmetic.
"""

from pathlib import Path

import pytest
from PIL import Image


def write_image(path: Path, colour: tuple[int, int, int], size: int = 64) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.new("RGB", (size, size), colour).save(path)
    return path


@pytest.fixture
def image_tree(tmp_path: Path) -> Path:
    """A two-label dataset: 3 'safe' images and 2 'explicit' images."""
    for index, colour in enumerate([(0, 128, 0), (0, 160, 0), (0, 200, 0)]):
        write_image(tmp_path / "safe" / f"{index}.png", colour)
    for index, colour in enumerate([(200, 0, 0), (160, 0, 0)]):
        write_image(tmp_path / "explicit" / f"{index}.png", colour)
    return tmp_path
