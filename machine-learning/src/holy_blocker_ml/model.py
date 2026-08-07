"""Wraps a pretrained Hugging Face NSFW image classifier for scoring.

v0 ships the model exactly as published — no fine-tuning, no re-exported
weights. This module exists to give that "as is" model one seam
(`NsfwClassifier` + `score_image`) that the rest of the pipeline depends on,
so swapping the backbone later (a fine-tuned checkpoint, a different
pretrained model) does not ripple into `eval.py` or `corpus.py`.

The NSFW class index is *resolved from the model's own config*, never
hardcoded. Silently assuming index 1 is "nsfw" is exactly the kind of bug
`packages/image-sandbox` guards against with `BINARY_LABELS` pinned against
`labels.py` (see docs/decisions/classifier-operating-point.md) — a model
swap that quietly inverted its label order would otherwise score every image
backwards with no error anywhere.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

DEFAULT_MODEL_ID = "Falconsai/nsfw_image_detection"

# Tolerant match against a model's id2label strings. Falconsai/nsfw_image_detection
# ships {0: "normal", 1: "nsfw"} (verified against the model's config.json on the
# Hub, 2026-08-07) but a future model swap may use different label spellings.
_NSFW_LABEL_ALIASES = {"nsfw", "porn", "explicit", "unsafe"}
_SAFE_LABEL_ALIASES = {"normal", "safe", "sfw"}


@dataclass(frozen=True)
class NsfwClassifier:
    """A loaded model + processor pair, plus the resolved NSFW class index."""

    model: Any
    processor: Any
    nsfw_index: int
    model_id: str


def resolve_nsfw_index(id2label: dict[int, str]) -> int:
    """Find which output index corresponds to the NSFW class.

    Raises ValueError rather than guessing if the label set is not
    recognizable, so an incompatible model fails loudly at load time instead
    of silently scoring backwards.
    """
    matches = [
        index
        for index, label in id2label.items()
        if label.strip().lower() in _NSFW_LABEL_ALIASES
    ]
    if len(matches) == 1:
        return matches[0]

    if len(matches) == 0:
        # Fall back to elimination only when there are exactly two classes and
        # the other one is recognizably "safe" — still explicit, still fails
        # loudly otherwise.
        safe_matches = [
            index
            for index, label in id2label.items()
            if label.strip().lower() in _SAFE_LABEL_ALIASES
        ]
        if len(id2label) == 2 and len(safe_matches) == 1:
            (nsfw_index,) = set(id2label.keys()) - set(safe_matches)
            return nsfw_index

    raise ValueError(
        f"could not uniquely resolve the NSFW class index from id2label={id2label!r}; "
        f"expected exactly one label in {sorted(_NSFW_LABEL_ALIASES)}"
    )


def load_classifier(model_id: str = DEFAULT_MODEL_ID) -> NsfwClassifier:
    """Load a pretrained Hugging Face image classifier for scoring.

    Requires network access on first call (Hub download, cached locally by
    `transformers` afterward) — this is a one-time local model fetch, not a
    runtime cloud call; the daemon paths never call out to a remote service.
    """
    # Imported lazily so importing this module (e.g. from corpus.py or tests)
    # never requires torch/transformers to be installed.
    import torch
    from transformers import AutoImageProcessor, AutoModelForImageClassification

    model = AutoModelForImageClassification.from_pretrained(model_id)
    model.eval()
    processor = AutoImageProcessor.from_pretrained(model_id)
    nsfw_index = resolve_nsfw_index(
        {int(k): v for k, v in model.config.id2label.items()}
    )
    del torch  # imported only to fail fast if unavailable; unused otherwise
    return NsfwClassifier(
        model=model, processor=processor, nsfw_index=nsfw_index, model_id=model_id
    )


def score_image(classifier: NsfwClassifier, image: Any) -> float:
    """Score one PIL image. Returns softmax(logits)[nsfw_index] as a float in [0, 1].

    Uses the model's own processor (its published resize/normalize recipe)
    rather than `packages/image-sandbox`'s tile-max geometry: this measures
    the model "as is", the way its publisher evaluated it. Whether to feed it
    through tile-max instead is a separate question for once a backbone is
    actually chosen for production (see the module docstring).
    """
    import torch

    inputs = classifier.processor(images=image.convert("RGB"), return_tensors="pt")
    with torch.no_grad():
        logits = classifier.model(**inputs).logits[0]
    probs = torch.softmax(logits, dim=-1)
    return float(probs[classifier.nsfw_index])


def score_path(classifier: NsfwClassifier, path: Path) -> float:
    """Score one image file. Opens and closes the file; does not cache."""
    from PIL import Image

    with Image.open(path) as image:
        return score_image(classifier, image)
