import pytest

from holy_blocker_ml.corpus import CorpusKind
from holy_blocker_ml.eval import (
    evaluate_scores,
    report,
    summarize_scores,
    threshold_sweep,
)


def test_summarize_scores_basic_stats() -> None:
    dist = summarize_scores([0.0, 0.5, 1.0])
    assert dist.count == 3
    assert dist.mean == pytest.approx(0.5)
    assert dist.median == pytest.approx(0.5)
    assert dist.min == pytest.approx(0.0)
    assert dist.max == pytest.approx(1.0)


def test_summarize_scores_rejects_empty() -> None:
    with pytest.raises(ValueError):
        summarize_scores([])


def test_threshold_sweep_counts_positive_rate() -> None:
    scores = [0.0, 0.1, 0.4, 0.6, 0.9]
    results = threshold_sweep(scores, thresholds=[0.5])
    assert len(results) == 1
    assert results[0].threshold == 0.5
    # 0.6 and 0.9 are >= 0.5 -> 2/5
    assert results[0].positive_rate == pytest.approx(0.4)


def test_threshold_sweep_boundary_is_inclusive() -> None:
    results = threshold_sweep([0.5], thresholds=[0.5])
    assert results[0].positive_rate == pytest.approx(1.0)


def test_evaluate_scores_reports_false_positive_rate_for_benign_corpus() -> None:
    scores = [0.01, 0.02, 0.9]
    result = evaluate_scores(scores, "synthetic-ui", CorpusKind.BENIGN, thresholds=[0.5])
    assert result.kind is CorpusKind.BENIGN
    assert result.thresholds[0].positive_rate == pytest.approx(1 / 3)
    text = report(result)
    assert "false-positive rate" in text
    assert "synthetic-ui" in text


def test_evaluate_scores_reports_recall_for_explicit_corpus() -> None:
    scores = [0.9, 0.95]
    result = evaluate_scores(scores, "held-out-explicit", CorpusKind.EXPLICIT, thresholds=[0.5])
    text = report(result)
    assert "recall" in text
