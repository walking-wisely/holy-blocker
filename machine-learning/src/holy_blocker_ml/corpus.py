"""Corpus loading for evaluation.

A corpus is single-class by construction: every image under a `BENIGN`
corpus's root is assumed clean, every image under an `EXPLICIT` corpus's
root is assumed to depict what the classifier should catch. There are no
per-file labels on disk — the label comes from which corpus a file lives in.
This mirrors the shape described in
docs/components/machine-learning/plan.md's corpus.py/gate.py section.

Corpus data never enters the repo. `CorpusSpec.root` is expected to point at
a gitignored local path (see `**/machine-learning/data` in the repo's
.gitignore); `load_corpus` raises `FileNotFoundError` rather than silently
returning an empty list if the root is missing, so a mistyped or
never-generated path fails loudly instead of reporting a perfect 0% score
over zero images.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from pathlib import Path

IMAGE_EXTENSIONS = {".png", ".jpg", ".jpeg", ".webp", ".bmp"}


class CorpusKind(Enum):
    """What a positive prediction means for this corpus.

    BENIGN -> a positive prediction is a false positive.
    EXPLICIT -> a positive prediction is a true positive (contributes to recall).
    """

    BENIGN = "benign"
    EXPLICIT = "explicit"


@dataclass(frozen=True)
class CorpusSpec:
    name: str
    root: Path
    kind: CorpusKind


def load_corpus(spec: CorpusSpec) -> list[Path]:
    """Return every image file under `spec.root`, sorted for determinism.

    Raises FileNotFoundError if `spec.root` does not exist, with a message
    pointing out that corpus data is never committed and must be generated
    or placed locally first.
    """
    if not spec.root.is_dir():
        raise FileNotFoundError(
            f"corpus '{spec.name}' root {spec.root} does not exist. "
            "Corpus data is never committed to this repo — generate it "
            "(see synth_ui.generate_corpus) or place real files there first."
        )
    return sorted(
        path
        for path in spec.root.rglob("*")
        if path.is_file() and path.suffix.lower() in IMAGE_EXTENSIONS
    )
