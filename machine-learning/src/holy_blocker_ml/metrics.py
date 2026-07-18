"""Threshold-free and operating-point metrics.

Accuracy at 0.5 is the least useful number available here. It conflates two
errors with very different costs, and it moves when the score distribution
shifts even if the model's ranking is unchanged — which is exactly what
happened while comparing checkpoints during fine-tuning.

The metrics here separate those concerns:

- `roc_auc` / `average_precision` depend only on ranking, so two models are
  comparable without arguing about where the cut sits.
- `fpr_at_fnr` answers the question this project actually cares about: given a
  ceiling on explicit content slipping through, what does it cost in
  over-blocking? A false negative defeats the product's purpose, so the miss
  rate is the budget and over-blocking is the price paid for it.
- `error_confidence` distinguishes a model that is uncertain from one that is
  confidently contradicted by its labels. The second pattern points at label
  noise rather than missing capacity.

Implemented directly rather than pulled from scikit-learn: each is a few lines
over the prediction list, and the dependency would not otherwise be needed.
"""

from collections.abc import Sequence

from holy_blocker_ml.eval import DEFAULT_THRESHOLD, Predictions
from holy_blocker_ml.labels import NEGATIVE_INDEX, POSITIVE_INDEX

#: A prediction is "confident" when it sits this far from the decision point.
CONFIDENT_MARGIN = 0.4


def _split_scores(predictions: Predictions) -> tuple[list[float], list[float]]:
    positives = [
        s for s, t in zip(predictions.positive_scores, predictions.targets, strict=True)
        if t == POSITIVE_INDEX
    ]
    negatives = [
        s for s, t in zip(predictions.positive_scores, predictions.targets, strict=True)
        if t == NEGATIVE_INDEX
    ]
    if not positives or not negatives:
        raise ValueError("both classes must be present to compute a ranking metric")
    return positives, negatives


def roc_auc(predictions: Predictions) -> float:
    """Area under the ROC curve, via the rank-sum (Mann-Whitney U) identity.

    Equals the probability that a random explicit sample outranks a random safe
    one. Ties contribute 0.5, which is why average ranks are used.
    """
    positives, negatives = _split_scores(predictions)

    ordered = sorted(positives + negatives)
    ranks: dict[float, float] = {}
    index = 0
    while index < len(ordered):
        end = index
        while end + 1 < len(ordered) and ordered[end + 1] == ordered[index]:
            end += 1
        # Average rank across the tied block, 1-based.
        ranks[ordered[index]] = (index + end) / 2 + 1
        index = end + 1

    rank_sum = sum(ranks[s] for s in positives)
    n_pos, n_neg = len(positives), len(negatives)
    return (rank_sum - n_pos * (n_pos + 1) / 2) / (n_pos * n_neg)


def average_precision(predictions: Predictions) -> float:
    """Area under the precision-recall curve (interpolation-free).

    More informative than ROC-AUC when the positive class is the minority, since
    it ignores the large pool of true negatives.
    """
    positives, _ = _split_scores(predictions)
    pairs = sorted(
        zip(predictions.positive_scores, predictions.targets, strict=True),
        key=lambda item: -item[0],
    )

    total_positives = len(positives)
    hits = 0
    score = 0.0
    for seen, (_, target) in enumerate(pairs, start=1):
        if target == POSITIVE_INDEX:
            hits += 1
            score += hits / seen
    return score / total_positives


def roc_curve(predictions: Predictions) -> list[tuple[float, float]]:
    """(FPR, TPR) points, ascending, including (0,0) and (1,1)."""
    positives, negatives = _split_scores(predictions)
    n_pos, n_neg = len(positives), len(negatives)

    pairs = sorted(
        zip(predictions.positive_scores, predictions.targets, strict=True),
        key=lambda item: -item[0],
    )

    points = [(0.0, 0.0)]
    tp = fp = 0
    previous: float | None = None
    for scored, target in pairs:
        if previous is not None and scored != previous:
            points.append((fp / n_neg, tp / n_pos))
        if target == POSITIVE_INDEX:
            tp += 1
        else:
            fp += 1
        previous = scored
    points.append((fp / n_neg, tp / n_pos))
    return points


def _sweep_points(predictions: Predictions) -> list[tuple[float, float, float]]:
    """(threshold, fpr, fnr) at every distinct score, plus the extremes."""
    positives, negatives = _split_scores(predictions)
    n_pos, n_neg = len(positives), len(negatives)

    rows = []
    candidates = sorted(set(predictions.positive_scores))
    # Include a cut below everything (block all) and above everything (block none).
    for threshold in [candidates[0] - 1e-9, *candidates, candidates[-1] + 1e-9]:
        fp = sum(1 for s in negatives if s >= threshold)
        fn = sum(1 for s in positives if s < threshold)
        rows.append((threshold, fp / n_neg, fn / n_pos))
    return rows


def fnr_at_fpr(
    predictions: Predictions,
    max_fpr: float,
    with_threshold: bool = False,
) -> float | tuple[float, float]:
    """Lowest achievable miss rate while over-blocking at most `max_fpr`."""
    feasible = [(fnr, t) for t, fpr, fnr in _sweep_points(predictions) if fpr <= max_fpr]
    best_fnr, threshold = min(feasible) if feasible else (1.0, float("inf"))
    return (best_fnr, threshold) if with_threshold else best_fnr


def fpr_at_fnr(
    predictions: Predictions,
    max_fnr: float,
    with_threshold: bool = False,
) -> float | tuple[float, float]:
    """Lowest over-blocking rate while missing at most `max_fnr` of explicit content.

    The primary operating metric for this project: the miss rate is the budget
    that gets set, and this reports what honouring it costs.
    """
    feasible = [(fpr, t) for t, fpr, fnr in _sweep_points(predictions) if fnr <= max_fnr]
    best_fpr, threshold = min(feasible) if feasible else (1.0, float("-inf"))
    return (best_fpr, threshold) if with_threshold else best_fpr


def error_confidence(
    predictions: Predictions,
    threshold: float = DEFAULT_THRESHOLD,
    margin: float = CONFIDENT_MARGIN,
) -> dict[str, float]:
    """How sure the model was when it got things wrong.

    A high confident share means the model is not hedging on its mistakes — it
    is certain and the label disagrees, which is the signature of label noise
    rather than insufficient capacity.
    """
    errors = 0
    confident = 0
    for scored, target in zip(predictions.positive_scores, predictions.targets, strict=True):
        predicted = POSITIVE_INDEX if scored >= threshold else NEGATIVE_INDEX
        if predicted == target:
            continue
        errors += 1
        if abs(scored - threshold) > margin:
            confident += 1

    return {
        "errors": errors,
        "confident_errors": confident,
        "confident_share": confident / errors if errors else 0.0,
    }


def report_metrics(predictions: Predictions, miss_budgets: Sequence[float] = (0.10, 0.05, 0.02)) -> str:
    """Threshold-free summary plus the cost of each miss budget."""
    lines = [
        f"ROC-AUC:           {roc_auc(predictions):.4f}",
        f"PR-AUC (avg prec): {average_precision(predictions):.4f}",
        "",
        "cost of a miss budget (false negatives are the failure that matters):",
        f"{'max FN':>8}{'-> FP rate':>13}{'threshold':>12}",
    ]
    for budget in miss_budgets:
        rate, threshold = fpr_at_fnr(predictions, budget, with_threshold=True)
        lines.append(f"{budget:>8.1%}{rate:>13.2%}{threshold:>12.4f}")

    summary = error_confidence(predictions)
    lines += [
        "",
        f"errors: {int(summary['errors'])}, of which confident: "
        f"{int(summary['confident_errors'])} ({summary['confident_share']:.1%})",
    ]
    return "\n".join(lines)
