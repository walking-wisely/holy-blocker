"""Local image dataset loading.

Mirrors the `torchvision.datasets.ImageFolder` contract but keeps discovery and
augmentation explicit so the policy is easy to audit and change. Label indices
come from `labels.BINARY_LABELS`, never from sorted directory names.
"""

from collections.abc import Sequence
from pathlib import Path

import torch
from PIL import Image
from torch import Tensor
from torch.utils.data import DataLoader, Dataset
from torchvision import transforms

from holy_blocker_ml.labels import BINARY_LABELS

IMAGE_SUFFIXES = frozenset({".png", ".jpg", ".jpeg", ".webp", ".bmp"})

# torchvision ships MobileNetV3 weights trained with ImageNet statistics; the
# fine-tuned head inherits that expectation, so inputs must use the same values.
IMAGENET_MEAN = (0.485, 0.456, 0.406)
IMAGENET_STD = (0.229, 0.224, 0.225)


def build_transform(image_size: int, augment: bool) -> transforms.Compose:
    """Training uses light geometric/colour jitter; validation is deterministic.

    Augmentation is deliberately conservative — no vertical flip or heavy
    rotation, since neither reflects how this content is actually framed.
    """
    if augment:
        stages: list[transforms.Transform] = [
            transforms.RandomResizedCrop(image_size, scale=(0.7, 1.0)),
            transforms.RandomHorizontalFlip(),
            transforms.ColorJitter(brightness=0.2, contrast=0.2, saturation=0.2),
        ]
    else:
        stages = [
            transforms.Resize(int(image_size * 1.14)),
            transforms.CenterCrop(image_size),
        ]
    return transforms.Compose(
        [*stages, transforms.ToTensor(), transforms.Normalize(IMAGENET_MEAN, IMAGENET_STD)]
    )


class LocalImageDataset(Dataset):
    """Images under `root/<label>/`, where `<label>` is in `BINARY_LABELS`."""

    def __init__(
        self,
        root: Path,
        image_size: int,
        augment: bool,
        classes: Sequence[str] = BINARY_LABELS,
    ) -> None:
        self.root = Path(root)
        self.classes = list(classes)
        self.transform = build_transform(image_size, augment)

        index_of = {name: index for index, name in enumerate(self.classes)}
        present = sorted(p.name for p in self.root.iterdir() if p.is_dir()) if self.root.is_dir() else []
        unknown = [name for name in present if name not in index_of]
        if unknown:
            raise ValueError(
                f"unexpected label directories under {self.root}: {', '.join(unknown)}. "
                f"expected one of: {', '.join(self.classes)}"
            )

        self.samples: list[tuple[Path, int]] = []
        for name in present:
            for path in sorted((self.root / name).rglob("*")):
                if path.is_file() and path.suffix.lower() in IMAGE_SUFFIXES:
                    self.samples.append((path, index_of[name]))

        if not self.samples:
            raise ValueError(f"no images found under {self.root}")

    def __len__(self) -> int:
        return len(self.samples)

    def __getitem__(self, idx: int) -> tuple[Tensor, int]:
        path, label = self.samples[idx]
        with Image.open(path) as image:
            # Datasets contain palette/grayscale/alpha images; the model wants 3 channels.
            return self.transform(image.convert("RGB")), label


def load_dataset(
    root: Path,
    image_size: int,
    augment: bool,
    batch_size: int = 32,
    num_workers: int = 0,
) -> DataLoader:
    """Return a DataLoader over `LocalImageDataset`. Shuffles only when augmenting."""
    dataset = LocalImageDataset(root, image_size=image_size, augment=augment)
    return DataLoader(
        dataset,
        batch_size=batch_size,
        shuffle=augment,
        num_workers=num_workers,
        generator=torch.Generator().manual_seed(0) if augment else None,
    )
