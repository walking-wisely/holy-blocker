"""Pure evaluation metrics for the binary baseline classifier.

No file I/O — `harness.py` owns loading checkpoints and printing. The emphasis
is on false positives and false negatives rather than headline accuracy: the
two error kinds cost very different things here. A false positive blocks
something harmless and erodes trust in the product; a false negative lets
through exactly what the user asked to be shielded from. Accuracy alone hides
that asymmetry, especially on a class-imbalanced evaluation set.
"""

from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path

import torch
from torch import nn
from torch.utils.data import DataLoader

from holy_blocker_ml.labels import BINARY_LABELS, NEGATIVE_INDEX, POSITIVE_INDEX

DEFAULT_THRESHOLD = 0.5

#: Sweep grid used by the CLI harness when no explicit thresholds are given.
DEFAULT_SWEEP = (0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9)


@dataclass(frozen=True)
class Predictions:
    """Raw per-sample inference output, decoupled from any threshold choice.

    Kept separate so a threshold sweep re-scores in memory instead of re-running
    the model once per candidate threshold.
    """

    targets: list[int]
    positive_scores: list[float]
    paths: list[Path] = field(default_factory=list)

    def __len__(self) -> int:
        return len(self.targets)


@dataclass(frozen=True)
class ThresholdRow:
    threshold: float
    false_positives: int
    false_negatives: int
    false_positive_rate: float
    false_negative_rate: float
    accuracy: float


@dataclass(frozen=True)
class EvalResult:
    accuracy: float
    per_class_precision: dict[int, float]
    per_class_recall: dict[int, float]
    confusion_matrix: list[list[int]]
    true_positives: int
    true_negatives: int
    false_positives: int
    false_negatives: int
    false_positive_rate: float
    false_negative_rate: float
    sample_count: int
    threshold: float


def _ratio(numerator: int, denominator: int) -> float:
    """Undefined rates report as 0.0 rather than NaN so reports stay printable."""
    return numerator / denominator if denominator else 0.0


@torch.no_grad()
def collect_predictions(model: nn.Module, loader: DataLoader) -> Predictions:
    """Run inference over `loader`, returning targets and positive-class scores."""
    model.eval()

    targets: list[int] = []
    scores: list[float] = []
    for images, labels in loader:
        probabilities = torch.softmax(model(images), dim=1)
        scores.extend(probabilities[:, POSITIVE_INDEX].tolist())
        targets.extend(int(label) for label in labels)

    paths = [path for path, _ in getattr(loader.dataset, "samples", [])]
    return Predictions(targets=targets, positive_scores=scores, paths=paths)


def score(predictions: Predictions, threshold: float = DEFAULT_THRESHOLD) -> EvalResult:
    """Turn raw predictions into metrics at a given decision threshold."""
    class_count = len(BINARY_LABELS)
    matrix = [[0] * class_count for _ in range(class_count)]

    for target, positive_score in zip(predictions.targets, predictions.positive_scores, strict=True):
        predicted = POSITIVE_INDEX if positive_score >= threshold else NEGATIVE_INDEX
        matrix[target][predicted] += 1

    true_positives = matrix[POSITIVE_INDEX][POSITIVE_INDEX]
    true_negatives = matrix[NEGATIVE_INDEX][NEGATIVE_INDEX]
    false_positives = matrix[NEGATIVE_INDEX][POSITIVE_INDEX]
    false_negatives = matrix[POSITIVE_INDEX][NEGATIVE_INDEX]

    total = len(predictions)
    correct = sum(matrix[index][index] for index in range(class_count))

    precision: dict[int, float] = {}
    recall: dict[int, float] = {}
    for index in range(class_count):
        predicted_count = sum(matrix[row][index] for row in range(class_count))
        actual_count = sum(matrix[index])
        precision[index] = _ratio(matrix[index][index], predicted_count)
        recall[index] = _ratio(matrix[index][index], actual_count)

    return EvalResult(
        accuracy=_ratio(correct, total),
        per_class_precision=precision,
        per_class_recall=recall,
        confusion_matrix=matrix,
        true_positives=true_positives,
        true_negatives=true_negatives,
        false_positives=false_positives,
        false_negatives=false_negatives,
        false_positive_rate=_ratio(false_positives, false_positives + true_negatives),
        false_negative_rate=_ratio(false_negatives, false_negatives + true_positives),
        sample_count=total,
        threshold=threshold,
    )


def evaluate(
    model: nn.Module,
    loader: DataLoader,
    threshold: float = DEFAULT_THRESHOLD,
) -> EvalResult:
    """Run inference over `loader` and return metrics. Sets model to eval mode."""
    return score(collect_predictions(model, loader), threshold=threshold)


def sweep_thresholds(
    predictions: Predictions,
    thresholds: Sequence[float] = DEFAULT_SWEEP,
) -> list[ThresholdRow]:
    """Re-score one set of predictions across thresholds to expose the tradeoff."""
    rows = []
    for threshold in thresholds:
        result = score(predictions, threshold=threshold)
        rows.append(
            ThresholdRow(
                threshold=threshold,
                false_positives=result.false_positives,
                false_negatives=result.false_negatives,
                false_positive_rate=result.false_positive_rate,
                false_negative_rate=result.false_negative_rate,
                accuracy=result.accuracy,
            )
        )
    return rows


def misclassified(
    predictions: Predictions,
    threshold: float = DEFAULT_THRESHOLD,
) -> tuple[list[tuple[Path, float]], list[tuple[Path, float]]]:
    """Return (false positives, false negatives) as (path, score) pairs.

    Empty when the loader's dataset does not expose `samples` (e.g. a synthetic
    tensor dataset). Ordering follows the dataset, so it lines up with a
    non-shuffled loader — which is why validation loaders never shuffle.
    """
    if len(predictions.paths) != len(predictions):
        return [], []

    false_positives: list[tuple[Path, float]] = []
    false_negatives: list[tuple[Path, float]] = []
    for path, target, positive_score in zip(
        predictions.paths, predictions.targets, predictions.positive_scores, strict=True
    ):
        predicted = POSITIVE_INDEX if positive_score >= threshold else NEGATIVE_INDEX
        if target == NEGATIVE_INDEX and predicted == POSITIVE_INDEX:
            false_positives.append((path, positive_score))
        elif target == POSITIVE_INDEX and predicted == NEGATIVE_INDEX:
            false_negatives.append((path, positive_score))

    false_positives.sort(key=lambda item: item[1], reverse=True)
    false_negatives.sort(key=lambda item: item[1])
    return false_positives, false_negatives


def report(result: EvalResult) -> str:
    """Human-readable summary of an EvalResult."""
    lines = [
        f"samples: {result.sample_count}   threshold: {result.threshold:.2f}",
        f"accuracy: {result.accuracy:.4f}",
        "",
        f"false positives (safe blocked):    {result.false_positives}"
        f"   rate {result.false_positive_rate:.4f}",
        f"false negatives (explicit missed): {result.false_negatives}"
        f"   rate {result.false_negative_rate:.4f}",
        "",
        f"{'class':<10}{'precision':>11}{'recall':>9}",
    ]
    for index, name in enumerate(BINARY_LABELS):
        lines.append(
            f"{name:<10}{result.per_class_precision[index]:>11.4f}"
            f"{result.per_class_recall[index]:>9.4f}"
        )

    lines += ["", "confusion matrix (rows = actual, columns = predicted)"]
    header = " " * 10 + "".join(f"{name:>10}" for name in BINARY_LABELS)
    lines.append(header)
    for index, name in enumerate(BINARY_LABELS):
        lines.append(f"{name:<10}" + "".join(f"{count:>10}" for count in result.confusion_matrix[index]))

    return "\n".join(lines)


def report_sweep(rows: Sequence[ThresholdRow]) -> str:
    """Table of the false-positive / false-negative tradeoff across thresholds."""
    lines = [f"{'thresh':>7}{'FP':>7}{'FN':>7}{'FPR':>9}{'FNR':>9}{'acc':>9}"]
    for row in rows:
        lines.append(
            f"{row.threshold:>7.2f}{row.false_positives:>7}{row.false_negatives:>7}"
            f"{row.false_positive_rate:>9.4f}{row.false_negative_rate:>9.4f}{row.accuracy:>9.4f}"
        )
    return "\n".join(lines)
