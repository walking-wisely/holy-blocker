"""Single-class evaluation corpora.

The evaluation strategy in docs/decisions/learning-from-feedback.md splits the
two metrics that matter across two corpora, each uniform by construction:

- a **benign** corpus (medical, sex-ed, biology, news, ordinary browsing) where
  every item is clean, so any flag is a false positive;
- an **explicit** corpus — a held-out public NSFW benchmark — where every item
  is explicit, so any item let through is a false negative.

Because each corpus is single-class, no per-file labels are needed: the label
comes from the corpus kind. That is why this loads a flat directory rather than
reusing `dataset.LocalImageDataset`.

Neither corpus lives in this repo. The explicit corpus must stay out of the
repo and out of the training/feedback loop; the benign corpus is publishable in
principle but is still kept local. Point `CorpusSpec.root` at a gitignored path.
"""

from dataclasses import dataclass
from enum import Enum
from pathlib import Path

import torch
from torch import Tensor, nn
from torch.utils.data import DataLoader, Dataset

from holy_blocker_ml.dataset import build_transform, find_images, load_image

# Label convention, fixed by sorted directory order in `LocalImageDataset`
# ("clean" < "explicit").
CLEAN_LABEL = 0
EXPLICIT_LABEL = 1


class CorpusKind(Enum):
    """What every item in a corpus is, by construction."""

    BENIGN = "benign"
    EXPLICIT = "explicit"

    @property
    def metric_name(self) -> str:
        """The metric this corpus measures."""
        return "false_positive_rate" if self is CorpusKind.BENIGN else "recall"


@dataclass(frozen=True)
class CorpusSpec:
    name: str
    root: Path
    kind: CorpusKind


@dataclass(frozen=True)
class CorpusMeasurement:
    name: str
    kind: CorpusKind
    item_count: int
    value: float

    @property
    def metric_name(self) -> str:
        return self.kind.metric_name


class FlatImageCorpus(Dataset[Tensor]):
    """Every image directly under a directory, unlabelled."""

    def __init__(self, root: Path, image_size: int) -> None:
        self.paths = find_images(Path(root))
        self.transform = build_transform(image_size, augment=False)

    def __len__(self) -> int:
        return len(self.paths)

    def __getitem__(self, idx: int) -> Tensor:
        return load_image(self.paths[idx], self.transform)


def load_corpus(spec: CorpusSpec, image_size: int, batch_size: int = 32) -> DataLoader[Tensor]:
    """Return a DataLoader over the corpus at `spec.root`."""
    if not spec.root.is_dir():
        raise FileNotFoundError(
            f"corpus '{spec.name}' not found at {spec.root}. Evaluation corpora are "
            f"local and gitignored — they are never committed to this repo. See "
            f"docs/decisions/learning-from-feedback.md (Evaluation) for what each "
            f"corpus must contain."
        )

    corpus = FlatImageCorpus(spec.root, image_size=image_size)
    if len(corpus) == 0:
        raise ValueError(f"corpus '{spec.name}' contains no images: {spec.root}")

    return DataLoader(corpus, batch_size=batch_size, shuffle=False)


def explicit_prediction_rate(model: nn.Module, loader: DataLoader) -> float:
    """Fraction of items the model flags as explicit.

    On a benign corpus this is the false-positive rate; on an explicit corpus
    it is recall. One computation, two readings — which is why the corpus kind
    has to be tracked alongside the number.
    """
    model.eval()

    flagged = 0
    total = 0
    with torch.no_grad():
        for images in loader:
            predictions = model(images).argmax(dim=1)
            flagged += int((predictions == EXPLICIT_LABEL).sum())
            total += int(predictions.numel())

    return flagged / total if total else 0.0


def measure_corpus(
    model: nn.Module,
    spec: CorpusSpec,
    image_size: int,
    batch_size: int = 32,
) -> CorpusMeasurement:
    """Measure `model` against one corpus, tagging the result with its kind."""
    loader = load_corpus(spec, image_size=image_size, batch_size=batch_size)
    rate = explicit_prediction_rate(model, loader)
    return CorpusMeasurement(
        name=spec.name,
        kind=spec.kind,
        item_count=len(loader.dataset),  # type: ignore[arg-type]
        value=rate,
    )


__all__ = [
    "CLEAN_LABEL",
    "EXPLICIT_LABEL",
    "CorpusKind",
    "CorpusMeasurement",
    "CorpusSpec",
    "FlatImageCorpus",
    "explicit_prediction_rate",
    "load_corpus",
    "measure_corpus",
]
