import pytest

from holy_blocker_ml.eval import Predictions
from holy_blocker_ml.metrics import (
    average_precision,
    error_confidence,
    fpr_at_fnr,
    fnr_at_fpr,
    roc_auc,
    roc_curve,
)


def predictions(targets, scores) -> Predictions:
    return Predictions(targets=list(targets), positive_scores=list(scores))


# --- ROC-AUC ----------------------------------------------------------------


def test_perfect_separation_scores_one() -> None:
    assert roc_auc(predictions([0, 0, 1, 1], [0.1, 0.2, 0.8, 0.9])) == 1.0


def test_perfectly_inverted_scores_zero() -> None:
    assert roc_auc(predictions([0, 0, 1, 1], [0.9, 0.8, 0.2, 0.1])) == 0.0


def test_all_scores_tied_is_a_coin_flip() -> None:
    assert roc_auc(predictions([0, 1, 0, 1], [0.5] * 4)) == 0.5


def test_known_value_matches_hand_computation() -> None:
    # 2 negatives (0.1, 0.4), 2 positives (0.35, 0.8).
    # Pairs where positive > negative: (0.35,0.1),(0.8,0.1),(0.8,0.4) = 3 of 4.
    assert roc_auc(predictions([0, 1, 0, 1], [0.1, 0.35, 0.4, 0.8])) == 0.75


def test_ties_between_classes_count_as_half() -> None:
    # One positive and one negative share a score: that pair contributes 0.5.
    result = roc_auc(predictions([0, 1], [0.5, 0.5]))

    assert result == 0.5


def test_auc_is_invariant_to_monotonic_rescaling() -> None:
    """AUC depends only on ranking, which is why it survives a bad threshold."""
    base = predictions([0, 0, 1, 1], [0.1, 0.2, 0.8, 0.9])
    squashed = predictions([0, 0, 1, 1], [0.40, 0.41, 0.44, 0.45])

    assert roc_auc(base) == roc_auc(squashed)


def test_auc_undefined_without_both_classes() -> None:
    with pytest.raises(ValueError, match="both classes"):
        roc_auc(predictions([1, 1], [0.2, 0.9]))


# --- PR-AUC -----------------------------------------------------------------


def test_average_precision_is_one_for_perfect_ranking() -> None:
    assert average_precision(predictions([0, 0, 1, 1], [0.1, 0.2, 0.8, 0.9])) == 1.0


def test_average_precision_falls_below_one_when_a_negative_ranks_top() -> None:
    result = average_precision(predictions([0, 1, 1], [0.9, 0.8, 0.7]))

    assert 0.0 < result < 1.0


# --- operating points -------------------------------------------------------


def test_fnr_at_fpr_finds_the_achievable_miss_rate() -> None:
    # Perfectly separable: any FPR budget still yields zero misses.
    assert fnr_at_fpr(predictions([0, 0, 1, 1], [0.1, 0.2, 0.8, 0.9]), max_fpr=0.5) == 0.0


def test_fpr_at_fnr_reports_the_cost_of_a_miss_budget() -> None:
    """The question that matters here: to miss almost nothing, what is over-blocked?"""
    # One positive sits below a negative, so catching it requires a false positive.
    result = fpr_at_fnr(predictions([0, 0, 1, 1], [0.1, 0.6, 0.5, 0.9]), max_fnr=0.0)

    assert result == 0.5  # 1 of 2 negatives must be blocked to miss nothing


def test_tightening_the_miss_budget_never_lowers_the_cost() -> None:
    preds = predictions([0, 0, 0, 1, 1, 1], [0.1, 0.5, 0.7, 0.2, 0.6, 0.9])

    lenient = fpr_at_fnr(preds, max_fnr=0.67)
    strict = fpr_at_fnr(preds, max_fnr=0.0)

    assert strict >= lenient


def test_operating_points_return_the_threshold_that_achieves_them() -> None:
    preds = predictions([0, 0, 1, 1], [0.1, 0.6, 0.5, 0.9])

    rate, threshold = fpr_at_fnr(preds, max_fnr=0.0, with_threshold=True)

    assert rate == 0.5
    assert 0.0 < threshold <= 0.5  # must cut at or below the lowest positive


# --- curve ------------------------------------------------------------------


def test_roc_curve_starts_at_origin_and_ends_at_one() -> None:
    points = roc_curve(predictions([0, 0, 1, 1], [0.1, 0.2, 0.8, 0.9]))

    assert points[0] == (0.0, 0.0)
    assert points[-1] == (1.0, 1.0)


def test_roc_curve_is_monotonic() -> None:
    points = roc_curve(predictions([0, 1, 0, 1, 0, 1], [0.1, 0.2, 0.3, 0.6, 0.7, 0.9]))

    assert points == sorted(points)


# --- label-noise diagnostic -------------------------------------------------


def test_confident_errors_are_separated_from_uncertain_ones() -> None:
    # Two wrong-and-sure predictions, one wrong-but-hesitant.
    preds = predictions([0, 1, 0], [0.99, 0.01, 0.55])

    summary = error_confidence(preds, threshold=0.5)

    assert summary["errors"] == 3
    assert summary["confident_errors"] == 2  # |score - 0.5| > 0.4 by default
    assert summary["confident_share"] == pytest.approx(2 / 3)


def test_no_errors_reports_zero_share_rather_than_dividing_by_zero() -> None:
    summary = error_confidence(predictions([0, 1], [0.1, 0.9]), threshold=0.5)

    assert summary["errors"] == 0
    assert summary["confident_share"] == 0.0
