"""Group the five source classes into the two media the model actually confuses.

The corpus labels content by provenance (`neutral`, `sexy`, `porn`, `drawings`,
`hentai`), but the model's errors separate along a coarser axis: whether the
image is photographic or drawn. Photographic content scores 0.9844 AUC against
0.9530 for drawn, and cross-medium confusion is near zero — the two behave as
independent sub-problems sharing one backbone.

That split is the unit the
[anime subsampling experiment](../../../docs/components/machine-learning/experiments/anime-subsampling.md)
is judged on: its decision rule accepts a drawn gain only if photographic
performance is preserved. Scoring each medium therefore has to be a
first-class, tested operation rather than an ad-hoc slice at report time.

Each medium is scored over *only* its own samples. Ranking metrics are not
decomposable — computing a global ranking and then filtering would let
photographic scores shift the drawn ordering, which is exactly the
contamination the experiment is trying to rule out.
"""

from collections.abc import Iterable, Sequence

from holy_blocker_ml.eval import Predictions
from holy_blocker_ml.metrics import roc_auc

PHOTOGRAPHIC: tuple[str, ...] = ("neutral", "sexy", "porn")
DRAWN: tuple[str, ...] = ("drawings", "hentai")

#: Danbooru ratings supplied by the anime subsampling experiment. Every one is
#: drawn by construction. Listed separately from `DRAWN` because that tuple is
#: the *scored* drawn holdout — it names the two `nsfw_detect` classes the
#: pre-registered baselines were measured on, and widening it would silently
#: change what the decision rule reads.
ANIME_DRAWN: tuple[str, ...] = (
    "anime_general",
    "anime_sensitive",
    "anime_questionable",
    "anime_explicit",
)

#: Medium name per source class. Pinned explicitly rather than inferred, for the
#: same reason `labels.py` pins class order: a silent miss here would misreport
#: the number the experiment's decision rule reads.
SOURCE_TO_MEDIUM: dict[str, str] = {
    **{name: "photographic" for name in PHOTOGRAPHIC},
    **{name: "drawn" for name in DRAWN},
    **{name: "drawn" for name in ANIME_DRAWN},
}


def medium_of(source: str) -> str:
    """Medium for a source class. Raises on anything unrecognised.

    Unknown classes raise rather than defaulting to a medium: the corpus class
    is `drawings`, and the singular `drawing` once matched nothing and silently
    dropped a fifth of the dataset while still reporting clean rates over the
    rest. A wrong medium here would misattribute an entire sub-problem.
    """
    try:
        return SOURCE_TO_MEDIUM[source]
    except KeyError:
        known = ", ".join(sorted(SOURCE_TO_MEDIUM))
        raise KeyError(f"unknown source class {source!r}; expected one of: {known}") from None


def subset_by_source(
    predictions: Predictions,
    source_labels: Sequence[str],
    keep: Iterable[str],
) -> Predictions:
    """Restrict predictions to the given source classes, preserving order."""
    if len(source_labels) != len(predictions):
        raise ValueError(
            f"got {len(source_labels)} source labels for {len(predictions)} predictions; "
            "a mismatched pairing would score the wrong samples"
        )

    wanted = set(keep)
    targets: list[int] = []
    scores: list[float] = []
    for source, target, positive_score in zip(
        source_labels, predictions.targets, predictions.positive_scores, strict=True
    ):
        if source in wanted:
            targets.append(target)
            scores.append(positive_score)
    return Predictions(targets=targets, positive_scores=scores)


def _auc_or_none(predictions: Predictions) -> float | None:
    """AUC, or None when the subset lacks one of the two labels.

    Undefined is reported as None rather than 0.0 or 0.5 so it cannot be read as
    a measurement and trip the experiment's accept/reject thresholds.
    """
    try:
        return roc_auc(predictions)
    except ValueError:
        return None


def per_medium_auc(
    predictions: Predictions,
    source_labels: Sequence[str],
    with_counts: bool = False,
) -> dict:
    """AUC for the drawn and photographic sub-problems, plus the combined set."""
    subsets = {
        "photographic": subset_by_source(predictions, source_labels, PHOTOGRAPHIC),
        "drawn": subset_by_source(predictions, source_labels, DRAWN),
        "combined": predictions,
    }

    result: dict = {name: _auc_or_none(subset) for name, subset in subsets.items()}
    if with_counts:
        result["counts"] = {name: len(subset) for name, subset in subsets.items()}
    return result


def medium_report(predictions: Predictions, source_labels: Sequence[str]) -> str:
    """Render the per-medium table the experiment reports against its baselines."""
    scored = per_medium_auc(predictions, source_labels, with_counts=True)
    counts = scored["counts"]

    lines = [f"{'medium':<16}{'n':>7}{'AUC':>10}"]
    for name in ("photographic", "drawn", "combined"):
        value = scored[name]
        rendered = "undefined" if value is None else f"{value:.4f}"
        lines.append(f"{name:<16}{counts[name]:>7}{rendered:>10}")
    return "\n".join(lines)
