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


def test_summarize_scores_percentiles_use_nearest_rank() -> None:
    # Six ordered scores: round(p * (n - 1)) would pick index 4 (value 4.0)
    # for p90 instead of the correct nearest-rank index 5 (value 5.0) — see
    # _percentile's docstring for why ceil(p * n) - 1 is used instead.
    dist = summarize_scores([0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
    assert dist.p90 == pytest.approx(5.0)
    assert dist.p95 == pytest.approx(5.0)
    assert dist.p99 == pytest.approx(5.0)


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


def test_evaluate_scores_without_labels_omits_top_items_from_report() -> None:
    result = evaluate_scores([0.1, 0.2], "unlabeled", CorpusKind.BENIGN, thresholds=[0.5])
    assert result.labels is None
    assert "highest-scoring" not in report(result)


def test_evaluate_scores_rejects_mismatched_label_count() -> None:
    with pytest.raises(ValueError):
        evaluate_scores([0.1, 0.2], "x", CorpusKind.BENIGN, labels=["only-one.png"])


def test_report_names_the_highest_scoring_items_when_labeled() -> None:
    scores = [0.9, 0.1, 0.5]
    labels = ["worst.png", "best.png", "middle.png"]
    result = evaluate_scores(scores, "labeled", CorpusKind.BENIGN, thresholds=[0.5], labels=labels)
    text = report(result)
    assert "worst.png" in text
    # The highest score should be listed before a lower one.
    assert text.index("worst.png") < text.index("middle.png")
