"""Local image loading for training and validation.

Mirrors the `torchvision.datasets.ImageFolder` contract but keeps the
implementation explicit so the augmentation policy stays easy to read and
change. Data lives outside the repo (`data/` is gitignored); nothing here
assumes any particular corpus.
"""

from collections.abc import Callable
from pathlib import Path
from typing import Any

from PIL import Image
from torch import Tensor
from torch.utils.data import DataLoader, Dataset
from torchvision import transforms

# Torchvision's pretrained MobileNetV3 weights are trained against these
# statistics; inputs must match or the frozen backbone sees a shifted
# distribution. See MobileNet_V3_Small_Weights.IMAGENET1K_V1.transforms().
IMAGENET_MEAN = (0.485, 0.456, 0.406)
IMAGENET_STD = (0.229, 0.224, 0.225)

IMAGE_EXTENSIONS = frozenset({".png", ".jpg", ".jpeg", ".webp", ".bmp"})


def build_transform(image_size: int, augment: bool) -> transforms.Compose:
    """Return the preprocessing pipeline for one split.

    Training augments (flip / resized crop / jitter); validation resizes and
    centre-crops only, so validation numbers stay comparable across runs.
    """
    if augment:
        steps: list[Callable[[Any], Any]] = [
            transforms.RandomResizedCrop(image_size, scale=(0.7, 1.0)),
            transforms.RandomHorizontalFlip(),
            transforms.ColorJitter(brightness=0.2, contrast=0.2, saturation=0.2),
        ]
    else:
        steps = [
            transforms.Resize(image_size),
            transforms.CenterCrop(image_size),
        ]
    steps += [
        transforms.ToTensor(),
        transforms.Normalize(mean=IMAGENET_MEAN, std=IMAGENET_STD),
    ]
    return transforms.Compose(steps)


def find_images(directory: Path) -> list[Path]:
    """Return every image file directly under `directory`, in sorted order."""
    return sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in IMAGE_EXTENSIONS
    )


def load_image(path: Path, transform: transforms.Compose) -> Tensor:
    """Read one image as a normalized CHW float tensor."""
    with Image.open(path) as image:
        return transform(image.convert("RGB"))


class LocalImageDataset(Dataset[tuple[Tensor, int]]):
    """Images in `root/<label>/<image>`, one subdirectory per label.

    Labels are assigned by sorted directory name, so a tree containing
    `clean/` and `explicit/` yields clean=0, explicit=1 — the convention the
    rest of the package assumes.
    """

    def __init__(self, root: Path, image_size: int, augment: bool) -> None:
        root = Path(root)
        if not root.is_dir():
            raise FileNotFoundError(f"dataset root does not exist: {root}")

        self.classes = sorted(child.name for child in root.iterdir() if child.is_dir())
        if not self.classes:
            raise ValueError(f"dataset root has no label subdirectories: {root}")

        self.class_to_idx = {name: index for index, name in enumerate(self.classes)}
        self.transform = build_transform(image_size, augment)
        self.samples: list[tuple[Path, int]] = [
            (path, self.class_to_idx[name])
            for name in self.classes
            for path in find_images(root / name)
        ]

    def __len__(self) -> int:
        return len(self.samples)

    def __getitem__(self, idx: int) -> tuple[Tensor, int]:
        path, label = self.samples[idx]
        return load_image(path, self.transform), label


def load_dataset(
    root: Path,
    image_size: int,
    augment: bool,
    batch_size: int = 32,
) -> DataLoader[tuple[Tensor, int]]:
    """Return a DataLoader over `LocalImageDataset`.

    Shuffles only when augmenting, i.e. only for the training split.
    """
    dataset = LocalImageDataset(root, image_size=image_size, augment=augment)
    return DataLoader(dataset, batch_size=batch_size, shuffle=augment)


__all__ = [
    "IMAGE_EXTENSIONS",
    "LocalImageDataset",
    "build_transform",
    "find_images",
    "load_dataset",
    "load_image",
]
