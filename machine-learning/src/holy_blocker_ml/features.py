"""Turn an explicit-image corpus into a non-viewable feature artifact.

Images enter as bytes, pass through the backbone in memory, and leave as float
vectors. Nothing decoded is ever written to disk. After one extraction pass the
`.npz` is the permanent evaluation asset and the source corpus can be deleted —
every later FP/FN run reads vectors, never pixels.

The honest limits of that guarantee:

- Feature vectors are not images, but they are also not cryptographically
  one-way. They cannot be casually viewed; they are not a redaction primitive.
- The source archive still exists on disk during the pass. It is never
  unpacked, and `extract_hf_dataset` deletes it afterwards, but that download
  window is the one moment the material is present in a decodable form.

Features are tied to the backbone that produced them: a stored artifact can
train or score any head of width `BACKBONE_FEATURE_DIM`, but changing backbone
means re-running extraction against the source corpus.
"""

import hashlib
import io
import json
import zipfile
from collections.abc import Iterable, Iterator, Mapping
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import torch
from PIL import Image
from torch import nn

from holy_blocker_ml.labels import BINARY_LABELS, EXPLICIT, SAFE
from holy_blocker_ml.model import BACKBONE_FEATURE_DIM

IMAGE_SUFFIXES = frozenset({".png", ".jpg", ".jpeg", ".webp", ".bmp"})

#: The five classes shipped by deepghs/nsfw_detect, which follows the
#: nsfw_data_scraper taxonomy.
SOURCE_CLASSES: tuple[str, ...] = ("neutral", "drawing", "sexy", "hentai", "porn")

#: How source classes collapse onto the binary decision.
#:
#: This mapping *is* the false-positive/false-negative definition — nothing
#: downstream can recover a distinction discarded here, so it is the first
#: thing to revisit when the numbers look wrong.
#:
#: "sexy" sits on the safe side deliberately. It is the hard-negative class:
#: suggestive but not explicit, and precisely the boundary a blocker gets wrong
#: in the direction users notice. Scoring it as safe means over-blocking it
#: shows up as a false positive instead of being quietly counted as a win.
#: "hentai" is blocked because illustrated explicit content is still the thing
#: being filtered. Override either via the `policy` argument.
DEFAULT_LABEL_POLICY: Mapping[str, str] = {
    "neutral": SAFE,
    "drawing": SAFE,
    "sexy": SAFE,
    "hentai": EXPLICIT,
    "porn": EXPLICIT,
}


@dataclass
class FeatureSet:
    """Extracted vectors plus everything needed to reproduce them."""

    features: np.ndarray  # (N, D) float32
    labels: np.ndarray  # (N,) int64, indices into BINARY_LABELS
    source_labels: list[str]
    digests: list[str]
    metadata: dict = field(default_factory=dict)

    def __len__(self) -> int:
        return len(self.labels)


def map_source_label(
    name: str,
    policy: Mapping[str, str] = DEFAULT_LABEL_POLICY,
) -> int | None:
    """Map a source class to a binary index, or None if the policy omits it.

    Returning None rather than defaulting is deliberate: silently bucketing an
    unrecognised class would corrupt the error rates the harness exists to
    measure.
    """
    mapped = policy.get(name)
    return BINARY_LABELS.index(mapped) if mapped is not None else None


class ArchiveLayoutError(RuntimeError):
    """The archive's directory layout does not expose the expected classes."""


@dataclass
class ArchiveSummary:
    """What a scan of the archive actually found, without decoding anything."""

    total_members: int
    image_members: int
    matched: dict[str, int]
    unmatched_images: int
    unmatched_examples: list[str]
    top_level: list[str]

    def describe(self) -> str:
        lines = [
            f"members: {self.total_members} ({self.image_members} images)",
            f"top-level entries: {', '.join(self.top_level) or '(none)'}",
        ]
        for name, count in sorted(self.matched.items()):
            lines.append(f"  {name:<10}{count:>8}")
        if self.unmatched_images:
            lines.append(f"  unmatched images: {self.unmatched_images}")
            lines += [f"    {example}" for example in self.unmatched_examples]
        return "\n".join(lines)


def _class_of(path: Path, known: set[str]) -> str | None:
    """The first path component naming a known class, tolerating any nesting."""
    return next((part for part in path.parts if part in known), None)


def inspect_archive(
    archive_path: Path,
    classes: Iterable[str] = SOURCE_CLASSES,
    example_limit: int = 5,
) -> ArchiveSummary:
    """Scan the archive's index without decoding images.

    Cheap preflight: the published layout of a corpus is easy to get wrong, and
    finding out after a full extraction pass is expensive.
    """
    known = set(classes)
    matched: dict[str, int] = {}
    unmatched_examples: list[str] = []
    top_level: set[str] = set()
    total = image_members = unmatched = 0

    with zipfile.ZipFile(archive_path) as archive:
        for info in archive.infolist():
            if info.is_dir():
                continue
            total += 1
            path = Path(info.filename)
            if path.parts:
                top_level.add(path.parts[0])
            if path.suffix.lower() not in IMAGE_SUFFIXES:
                continue

            image_members += 1
            name = _class_of(path, known)
            if name is None:
                unmatched += 1
                if len(unmatched_examples) < example_limit:
                    unmatched_examples.append(info.filename)
            else:
                matched[name] = matched.get(name, 0) + 1

    return ArchiveSummary(
        total_members=total,
        image_members=image_members,
        matched=matched,
        unmatched_images=unmatched,
        unmatched_examples=unmatched_examples,
        top_level=sorted(top_level),
    )


def iter_zip_images(
    archive_path: Path,
    classes: Iterable[str] = SOURCE_CLASSES,
    strict: bool = True,
) -> Iterator[tuple[Image.Image, str]]:
    """Yield (image, source class) from a zip without unpacking it.

    Members are decoded from an in-memory buffer, so no image file is ever
    created. The class comes from whichever path component matches a known
    class name, which tolerates the archive's top-level directory nesting.

    With `strict` (the default) an archive that exposes no recognisable class
    raises instead of yielding nothing. A silent empty result would otherwise
    surface as a confident report over zero samples.
    """
    summary = inspect_archive(archive_path, classes)
    if strict:
        if summary.image_members == 0:
            raise ArchiveLayoutError(
                f"{archive_path} contains no image files "
                f"({summary.total_members} members). Expected suffixes: "
                f"{', '.join(sorted(IMAGE_SUFFIXES))}.\n{summary.describe()}"
            )
        if not summary.matched:
            raise ArchiveLayoutError(
                f"{archive_path} has {summary.image_members} images but none sit "
                f"under a recognised class directory ({', '.join(sorted(classes))}). "
                f"The layout likely differs from <root>/<class>/<image>; pass "
                f"`classes=` to match it.\n{summary.describe()}"
            )

    known = set(classes)
    with zipfile.ZipFile(archive_path) as archive:
        for info in archive.infolist():
            if info.is_dir():
                continue
            path = Path(info.filename)
            if path.suffix.lower() not in IMAGE_SUFFIXES:
                continue
            match = _class_of(path, known)
            if match is None:
                continue
            with archive.open(info) as member:
                payload = member.read()
            with Image.open(io.BytesIO(payload)) as image:
                yield image.convert("RGB"), match


@torch.no_grad()
def extract_features(
    samples: Iterable[tuple[Image.Image, str]],
    backbone: nn.Module,
    transform,
    policy: Mapping[str, str] = DEFAULT_LABEL_POLICY,
    batch_size: int = 64,
) -> FeatureSet:
    """Run `samples` through `backbone`, returning vectors instead of images."""
    backbone.eval()

    vectors: list[np.ndarray] = []
    labels: list[int] = []
    source_labels: list[str] = []
    digests: list[str] = []
    source_counts: dict[str, int] = {}
    dropped = 0

    pending: list[torch.Tensor] = []

    def flush() -> None:
        if pending:
            vectors.append(backbone(torch.stack(pending)).cpu().numpy().astype(np.float32))
            pending.clear()

    for image, source_label in samples:
        label = map_source_label(source_label, policy)
        if label is None:
            dropped += 1
            continue

        # Digest the normalized pixels: stable per image, and a way to dedupe or
        # refer to a specific sample later without opening it.
        tensor = transform(image)
        digests.append(hashlib.sha256(tensor.numpy().tobytes()).hexdigest())

        pending.append(tensor)
        labels.append(label)
        source_labels.append(source_label)
        source_counts[source_label] = source_counts.get(source_label, 0) + 1
        if len(pending) >= batch_size:
            flush()

    flush()

    if not vectors:
        raise ValueError(
            f"extraction produced no samples ({dropped} dropped as unmapped). "
            "Either the source yielded nothing or every class fell outside the "
            f"policy: {sorted(policy)}"
        )

    stacked = np.concatenate(vectors, axis=0)
    return FeatureSet(
        features=stacked,
        labels=np.asarray(labels, dtype=np.int64),
        source_labels=source_labels,
        digests=digests,
        metadata={
            "source_counts": source_counts,
            "dropped": dropped,
            "feature_dim": int(stacked.shape[1]) if stacked.size else BACKBONE_FEATURE_DIM,
            "policy": dict(policy),
            "labels": list(BINARY_LABELS),
        },
    )


def save_feature_set(feature_set: FeatureSet, path: Path) -> Path:
    """Persist to a compressed `.npz`. Metadata rides along as JSON."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(
        path,
        features=feature_set.features,
        labels=feature_set.labels,
        source_labels=np.asarray(feature_set.source_labels, dtype=object),
        digests=np.asarray(feature_set.digests, dtype=object),
        metadata=json.dumps(feature_set.metadata),
    )
    return path


def load_feature_set(path: Path) -> FeatureSet:
    """Read back a `.npz` written by `save_feature_set`."""
    with np.load(path, allow_pickle=True) as payload:
        return FeatureSet(
            features=payload["features"],
            labels=payload["labels"],
            source_labels=[str(name) for name in payload["source_labels"]],
            digests=[str(digest) for digest in payload["digests"]],
            metadata=json.loads(str(payload["metadata"])),
        )
