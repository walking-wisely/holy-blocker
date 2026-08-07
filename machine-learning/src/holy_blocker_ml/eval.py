"""Pure evaluation over a corpus. No model loading, no file I/O beyond what a
caller-supplied `score_fn` does — this module only turns a list of scores
into reportable numbers, so it is fully testable with a fake scorer and
never needs torch/transformers or a real corpus on disk.

Per docs/decisions/classifier-operating-point.md, accuracy and a single
threshold are deliberately not the headline outputs: a threshold is
model-and-geometry-specific and this pipeline hasn't picked one yet, so
`evaluate_corpus` reports a full score distribution plus the false-positive
(or recall) rate at a *range* of candidate thresholds, leaving the actual
cut to be chosen later against a real miss-budget decision.
"""

from __future__ import annotations

import math
import statistics
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Sequence

from holy_blocker_ml.corpus import CorpusKind, CorpusSpec, load_corpus

# A spread wide enough to show where this off-the-shelf model's scores
# actually cluster, not a claim that any of these is the deployed cut.
DEFAULT_THRESHOLDS: tuple[float, ...] = (0.1, 0.2, 0.3, 0.5, 0.7, 0.9)


@dataclass(frozen=True)
class ScoreDistribution:
    count: int
    mean: float
    median: float
    p90: float
    p95: float
    p99: float
    max: float
    min: float


@dataclass(frozen=True)
class ThresholdResult:
    threshold: float
    positive_rate: float  # fraction of the corpus scored >= threshold


@dataclass(frozen=True)
class EvalResult:
    corpus_name: str
    kind: CorpusKind
    distribution: ScoreDistribution
    thresholds: tuple[ThresholdResult, ...]
    scores: tuple[float, ...]


def _percentile(sorted_scores: Sequence[float], p: float) -> float:
    if len(sorted_scores) == 1:
        return sorted_scores[0]
    # Nearest-rank: the smallest value at or beyond the p-th fraction of
    # the ordered set (ceil(p * n), 1-indexed), clamped into range. Not
    # interpolated — simple and matches what a reader expects from "p95" on
    # a small eval set without importing numpy just for this.
    #
    # round(p * (n - 1)) was tried first and is wrong: for n=6, p90 rounds
    # to index 4 (the 5th of 6 values) instead of index 5 (the actual
    # highest-ranked value at or above the 90th percentile).
    index = min(len(sorted_scores) - 1, max(0, math.ceil(p * len(sorted_scores)) - 1))
    return sorted_scores[index]


def summarize_scores(scores: Sequence[float]) -> ScoreDistribution:
    if not scores:
        raise ValueError("cannot summarize an empty score list")
    ordered = sorted(scores)
    return ScoreDistribution(
        count=len(ordered),
        mean=statistics.fmean(ordered),
        median=statistics.median(ordered),
        p90=_percentile(ordered, 0.90),
        p95=_percentile(ordered, 0.95),
        p99=_percentile(ordered, 0.99),
        max=ordered[-1],
        min=ordered[0],
    )


def threshold_sweep(
    scores: Sequence[float], thresholds: Sequence[float] = DEFAULT_THRESHOLDS
) -> tuple[ThresholdResult, ...]:
    if not scores:
        raise ValueError("cannot sweep thresholds over an empty score list")
    return tuple(
        ThresholdResult(
            threshold=t, positive_rate=sum(1 for s in scores if s >= t) / len(scores)
        )
        for t in thresholds
    )


def evaluate_scores(
    scores: Sequence[float],
    corpus_name: str,
    kind: CorpusKind,
    thresholds: Sequence[float] = DEFAULT_THRESHOLDS,
) -> EvalResult:
    """Turn a list of already-computed scores into an EvalResult.

    Split out from `evaluate_corpus` so tests never need a real scorer or
    real files — this is the pure core the rest of the module wraps.
    """
    return EvalResult(
        corpus_name=corpus_name,
        kind=kind,
        distribution=summarize_scores(scores),
        thresholds=threshold_sweep(scores, thresholds),
        scores=tuple(scores),
    )


def evaluate_corpus(
    spec: CorpusSpec,
    score_fn: Callable[[Path], float],
    thresholds: Sequence[float] = DEFAULT_THRESHOLDS,
) -> EvalResult:
    """Load `spec`'s corpus from disk and score every image with `score_fn`.

    `score_fn` is injected so the model itself never needs to be loaded in
    unit tests — see tests/test_eval.py for a fake scorer.
    """
    paths = load_corpus(spec)
    if not paths:
        raise ValueError(f"corpus '{spec.name}' at {spec.root} contains no images")
    scores = [score_fn(p) for p in paths]
    return evaluate_scores(scores, spec.name, spec.kind, thresholds)


def report(result: EvalResult) -> str:
    """Human-readable summary, e.g. for a CLI run's stdout."""
    d = result.distribution
    metric = "false-positive rate" if result.kind is CorpusKind.BENIGN else "recall"
    lines = [
        f"Corpus: {result.corpus_name} ({result.kind.value}, n={d.count})",
        f"  score distribution: mean={d.mean:.4f} median={d.median:.4f} "
        f"p90={d.p90:.4f} p95={d.p95:.4f} p99={d.p99:.4f} max={d.max:.4f} min={d.min:.4f}",
        f"  {metric} by candidate threshold:",
    ]
    for t in result.thresholds:
        lines.append(f"    >= {t.threshold:.2f}: {t.positive_rate:.2%}")
    return "\n".join(lines)
