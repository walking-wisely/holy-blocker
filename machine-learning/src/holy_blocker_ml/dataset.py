"""Local image dataset loading.

Mirrors the `torchvision.datasets.ImageFolder` contract but keeps discovery and
augmentation explicit so the policy is easy to audit and change. Label indices
come from `labels.BINARY_LABELS`, never from sorted directory names.
"""

import io
import os
import zipfile
from collections.abc import Sequence
from pathlib import Path

import torch
from PIL import Image
from torch import Tensor
from torch.utils.data import DataLoader, Dataset
from torchvision import transforms

from holy_blocker_ml.labels import BINARY_LABELS

IMAGE_SUFFIXES = frozenset({".png", ".jpg", ".jpeg", ".webp", ".bmp"})
#: Alias kept for `corpus.py`, which was written against this name.
IMAGE_EXTENSIONS = IMAGE_SUFFIXES

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


def find_images(directory: Path) -> list[Path]:
    """Return every image file directly under `directory`, in sorted order.

    Flat, non-recursive, and label-free — this is what a single-class evaluation
    corpus looks like, where the label comes from the corpus kind rather than
    from the directory tree. Used by `corpus.py`.
    """
    return sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in IMAGE_SUFFIXES
    )


def load_image(path: Path, transform: transforms.Compose) -> Tensor:
    """Read one image as a normalized CHW float tensor."""
    with Image.open(path) as image:
        return transform(image.convert("RGB"))


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


class ZipImageDataset(Dataset):
    """Images read straight out of a zip archive, decoded in memory.

    The fine-tuning counterpart to `features.iter_zip_images`: backbone
    gradients need pixels, so the archive has to be readable for the whole run,
    but it is still never unpacked and no image file is written.

    The archive handle is opened lazily and cached per process, so DataLoader
    workers each get their own rather than sharing a file descriptor across a
    fork (which yields corrupt reads).
    """

    def __init__(
        self,
        archive_path: Path,
        image_size: int,
        augment: bool,
        indices: Sequence[int] | None = None,
        policy=None,
    ) -> None:
        from holy_blocker_ml.features import (
            DEFAULT_LABEL_POLICY,
            IMAGE_SUFFIXES,
            _class_of,
            inspect_archive,
            map_source_label,
        )
        from holy_blocker_ml.features import ArchiveLayoutError, SOURCE_CLASSES

        self.archive_path = Path(archive_path)
        self.transform = build_transform(image_size, augment)
        policy = policy or DEFAULT_LABEL_POLICY

        summary = inspect_archive(self.archive_path)
        if not summary.matched:
            raise ArchiveLayoutError(
                f"{self.archive_path} exposes no recognised class directory.\n{summary.describe()}"
            )

        known = set(SOURCE_CLASSES)
        entries: list[tuple[str, str, int]] = []
        with zipfile.ZipFile(self.archive_path) as archive:
            for info in archive.infolist():
                if info.is_dir():
                    continue
                path = Path(info.filename)
                if path.suffix.lower() not in IMAGE_SUFFIXES:
                    continue
                source = _class_of(path, known)
                if source is None:
                    continue
                label = map_source_label(source, policy)
                if label is None:
                    continue
                entries.append((info.filename, source, label))

        if indices is not None:
            entries = [entries[i] for i in indices]

        self.samples = [(name, label) for name, _, label in entries]
        self.source_labels = [source for _, source, _ in entries]
        self._handle: zipfile.ZipFile | None = None
        self._pid: int | None = None

    def _archive(self) -> zipfile.ZipFile:
        pid = os.getpid()
        if self._handle is None or self._pid != pid:
            self._handle = zipfile.ZipFile(self.archive_path)
            self._pid = pid
        return self._handle

    def __len__(self) -> int:
        return len(self.samples)

    def __getitem__(self, idx: int) -> tuple[Tensor, int]:
        name, label = self.samples[idx]
        with self._archive().open(name) as member:
            payload = member.read()
        with Image.open(io.BytesIO(payload)) as image:
            return self.transform(image.convert("RGB")), label
