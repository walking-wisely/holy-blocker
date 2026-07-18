import math

import pytest
import torch
from torch import Tensor, nn
from torch.utils.data import DataLoader, TensorDataset

from holy_blocker_ml.eval import (
    collect_predictions,
    evaluate,
    report,
    sweep_thresholds,
)
from holy_blocker_ml.labels import EXPLICIT, POSITIVE_INDEX, SAFE


class ScriptedModel(nn.Module):
    """Returns the logits carried in each input row, so outcomes are exact.

    Each sample is a 2-vector already shaped like [safe_logit, explicit_logit].
    """

    def forward(self, batch: Tensor) -> Tensor:
        return batch


def loader_for(logits: list[list[float]], targets: list[int], batch_size: int = 2) -> DataLoader:
    dataset = TensorDataset(torch.tensor(logits), torch.tensor(targets))
    return DataLoader(dataset, batch_size=batch_size)


# Logit pairs that decode to a clear positive / clear negative prediction.
CONFIDENT_EXPLICIT = [0.0, 6.0]
CONFIDENT_SAFE = [6.0, 0.0]


def test_perfect_classifier_scores_one_with_no_errors() -> None:
    loader = loader_for(
        [CONFIDENT_SAFE, CONFIDENT_EXPLICIT, CONFIDENT_SAFE, CONFIDENT_EXPLICIT],
        [0, 1, 0, 1],
    )

    result = evaluate(ScriptedModel(), loader)

    assert result.accuracy == 1.0
    assert result.false_positives == 0
    assert result.false_negatives == 0
    assert result.confusion_matrix == [[2, 0], [0, 2]]
    assert result.per_class_recall[POSITIVE_INDEX] == 1.0


def test_counts_false_positives_and_negatives_with_correct_polarity() -> None:
    # 2 safe samples, one of which the model calls explicit -> 1 false positive.
    # 3 explicit samples, two of which it calls safe      -> 2 false negatives.
    loader = loader_for(
        [
            CONFIDENT_SAFE,
            CONFIDENT_EXPLICIT,  # safe predicted explicit -> FP
            CONFIDENT_SAFE,  # explicit predicted safe -> FN
            CONFIDENT_SAFE,  # explicit predicted safe -> FN
            CONFIDENT_EXPLICIT,
        ],
        [0, 0, 1, 1, 1],
    )

    result = evaluate(ScriptedModel(), loader)

    assert result.false_positives == 1
    assert result.false_negatives == 2
    assert result.true_positives == 1
    assert result.true_negatives == 1
    # FPR is over actual negatives (2), FNR over actual positives (3).
    assert result.false_positive_rate == pytest.approx(0.5)
    assert result.false_negative_rate == pytest.approx(2 / 3)
    assert result.accuracy == pytest.approx(2 / 5)
    assert result.confusion_matrix == [[1, 1], [2, 1]]


def test_confusion_matrix_is_indexed_true_then_predicted() -> None:
    loader = loader_for([CONFIDENT_EXPLICIT], [0])  # one safe sample called explicit

    matrix = evaluate(ScriptedModel(), loader).confusion_matrix

    assert matrix[0][1] == 1  # row = true safe, column = predicted explicit
    assert matrix[1][0] == 0


def test_raising_the_threshold_trades_false_positives_for_false_negatives() -> None:
    # A borderline explicit sample scoring ~0.62 for the positive class.
    borderline = [0.0, 0.5]
    loader = loader_for([CONFIDENT_SAFE, borderline], [0, 1])

    lenient = evaluate(ScriptedModel(), loader, threshold=0.5)
    strict = evaluate(ScriptedModel(), loader, threshold=0.9)

    assert lenient.false_negatives == 0
    assert strict.false_negatives == 1
    assert strict.false_positives == 0


def test_precision_and_recall_are_zero_not_nan_when_a_class_is_never_predicted() -> None:
    loader = loader_for([CONFIDENT_SAFE, CONFIDENT_SAFE], [0, 1])

    result = evaluate(ScriptedModel(), loader)

    assert result.per_class_precision[POSITIVE_INDEX] == 0.0
    assert not math.isnan(result.per_class_precision[POSITIVE_INDEX])
    assert result.per_class_recall[POSITIVE_INDEX] == 0.0


def test_evaluate_leaves_the_model_in_eval_mode() -> None:
    model = ScriptedModel()
    model.train()

    evaluate(model, loader_for([CONFIDENT_SAFE], [0]))

    assert not model.training


def test_collect_predictions_returns_positive_class_probabilities() -> None:
    loader = loader_for([CONFIDENT_SAFE, CONFIDENT_EXPLICIT], [0, 1])

    predictions = collect_predictions(ScriptedModel(), loader)

    assert predictions.targets == [0, 1]
    assert len(predictions.positive_scores) == 2
    assert all(0.0 <= score <= 1.0 for score in predictions.positive_scores)
    assert predictions.positive_scores[0] < 0.5 < predictions.positive_scores[1]


def test_sweep_reports_one_row_per_threshold_with_monotonic_error_tradeoff() -> None:
    loader = loader_for(
        [CONFIDENT_SAFE, [0.0, 0.5], CONFIDENT_EXPLICIT],
        [0, 1, 1],
    )
    predictions = collect_predictions(ScriptedModel(), loader)

    rows = sweep_thresholds(predictions, thresholds=[0.1, 0.5, 0.9])

    assert [row.threshold for row in rows] == [0.1, 0.5, 0.9]
    # Raising the threshold can only reduce FPs and only increase FNs.
    assert [row.false_positives for row in rows] == sorted(
        (row.false_positives for row in rows), reverse=True
    )
    assert [row.false_negatives for row in rows] == sorted(row.false_negatives for row in rows)


def test_report_names_both_classes_and_surfaces_the_error_counts() -> None:
    loader = loader_for([CONFIDENT_SAFE, CONFIDENT_SAFE], [0, 1])

    text = report(evaluate(ScriptedModel(), loader))

    assert SAFE in text
    assert EXPLICIT in text
    assert "false negatives" in text.lower()
    assert "1" in text


def test_evaluate_does_not_build_a_graph() -> None:
    """Inference must not retain autograd state — from the master-side suite."""
    inputs = torch.tensor([[6.0, 0.0], [0.0, 6.0]], requires_grad=True)
    dataset = TensorDataset(inputs, torch.tensor([0, 1]))

    evaluate(ScriptedModel(), DataLoader(dataset, batch_size=2))

    assert inputs.grad is None
