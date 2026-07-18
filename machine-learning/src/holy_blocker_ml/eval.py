"""Evaluation metrics.

Pure computation over a model and a loader — no file I/O, no artifact
handling, so the numbers can be tested without any real corpus.
"""

from dataclasses import dataclass

import torch
from torch import nn
from torch.utils.data import DataLoader


@dataclass(frozen=True)
class EvalResult:
    accuracy: float
    per_class_precision: dict[int, float]
    per_class_recall: dict[int, float]
    # Rows are true labels, columns are predictions.
    confusion_matrix: list[list[int]]


def _safe_ratio(numerator: int, denominator: int) -> float:
    """Ratio with the undefined case pinned to 0.0.

    A class that is never predicted has undefined precision, and a class with
    no true samples has undefined recall. Reporting 0.0 keeps the guardrail
    arithmetic total; it is deliberately pessimistic.
    """
    return numerator / denominator if denominator else 0.0


def evaluate(model: nn.Module, loader: DataLoader) -> EvalResult:
    """Run inference over `loader` and return metrics. Sets the model to eval mode."""
    model.eval()

    class_count = 0
    pairs: list[tuple[int, int]] = []
    with torch.no_grad():
        for images, labels in loader:
            logits = model(images)
            class_count = max(class_count, logits.shape[1])
            predictions = logits.argmax(dim=1)
            pairs.extend(
                (int(truth), int(prediction))
                for truth, prediction in zip(labels, predictions, strict=True)
            )

    matrix = [[0] * class_count for _ in range(class_count)]
    for truth, prediction in pairs:
        matrix[truth][prediction] += 1

    correct = sum(matrix[index][index] for index in range(class_count))
    precision = {
        index: _safe_ratio(
            matrix[index][index], sum(matrix[row][index] for row in range(class_count))
        )
        for index in range(class_count)
    }
    recall = {
        index: _safe_ratio(matrix[index][index], sum(matrix[index]))
        for index in range(class_count)
    }

    return EvalResult(
        accuracy=_safe_ratio(correct, len(pairs)),
        per_class_precision=precision,
        per_class_recall=recall,
        confusion_matrix=matrix,
    )


def report(result: EvalResult) -> str:
    """Return a human-readable summary of an `EvalResult`."""
    lines = [f"accuracy: {result.accuracy:.4g}"]
    for index in sorted(result.per_class_precision):
        lines.append(
            f"  class {index}: "
            f"precision={result.per_class_precision[index]:.4g} "
            f"recall={result.per_class_recall[index]:.4g}"
        )
    lines.append("confusion matrix (rows=true, cols=predicted):")
    lines.extend(f"  {row}" for row in result.confusion_matrix)
    return "\n".join(lines)


__all__ = ["EvalResult", "evaluate", "report"]
