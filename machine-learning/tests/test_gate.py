"""Tests for the release guardrail.

Encodes the rule from docs/decisions/learning-from-feedback.md (Evaluation):
recall on the held-out benchmark must not regress (CRITICAL, auto-rollback);
false-positive rate on the benign corpus is the secondary metric (WARNING if
it fails to improve).
"""

from holy_blocker_ml.gate import (
    GateSeverity,
    GuardrailThresholds,
    MetricSnapshot,
    check_guardrail,
)

BASELINE = MetricSnapshot(recall=0.90, false_positive_rate=0.20)


def test_recall_held_and_false_positives_reduced_passes() -> None:
    candidate = MetricSnapshot(recall=0.90, false_positive_rate=0.15)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.OK
    assert result.should_rollback is False
    assert result.reasons == []


def test_recall_regression_is_critical() -> None:
    candidate = MetricSnapshot(recall=0.85, false_positive_rate=0.05)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.CRITICAL
    assert result.should_rollback is True
    assert any("recall" in reason for reason in result.reasons)


def test_a_big_false_positive_win_cannot_buy_a_recall_regression() -> None:
    # The whole point of the one-directional design: the corruptible metric
    # never gets to trade away the metric users cannot corrupt.
    candidate = MetricSnapshot(recall=0.89, false_positive_rate=0.0)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.CRITICAL


def test_flat_false_positive_rate_is_a_warning_not_a_rollback() -> None:
    candidate = MetricSnapshot(recall=0.92, false_positive_rate=0.20)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.WARNING
    assert result.should_rollback is False
    assert any("false positive" in reason for reason in result.reasons)


def test_worse_false_positive_rate_is_a_warning() -> None:
    candidate = MetricSnapshot(recall=0.90, false_positive_rate=0.25)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.WARNING


def test_identical_recall_is_not_a_regression() -> None:
    candidate = MetricSnapshot(recall=0.90, false_positive_rate=0.10)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.OK


def test_recall_drop_within_tolerance_is_allowed() -> None:
    candidate = MetricSnapshot(recall=0.895, false_positive_rate=0.10)
    thresholds = GuardrailThresholds(max_recall_drop=0.01)

    result = check_guardrail(BASELINE, candidate, thresholds)

    assert result.severity is GateSeverity.OK


def test_recall_drop_beyond_tolerance_is_still_critical() -> None:
    candidate = MetricSnapshot(recall=0.87, false_positive_rate=0.10)
    thresholds = GuardrailThresholds(max_recall_drop=0.01)

    result = check_guardrail(BASELINE, candidate, thresholds)

    assert result.severity is GateSeverity.CRITICAL


def test_required_false_positive_improvement_is_configurable() -> None:
    candidate = MetricSnapshot(recall=0.90, false_positive_rate=0.19)
    thresholds = GuardrailThresholds(min_false_positive_improvement=0.05)

    result = check_guardrail(BASELINE, candidate, thresholds)

    assert result.severity is GateSeverity.WARNING


def test_critical_outranks_warning_when_both_fail() -> None:
    candidate = MetricSnapshot(recall=0.50, false_positive_rate=0.40)

    result = check_guardrail(BASELINE, candidate)

    assert result.severity is GateSeverity.CRITICAL
    assert len(result.reasons) == 2
