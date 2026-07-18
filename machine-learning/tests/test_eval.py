import torch
from torch import Tensor, nn
from torch.utils.data import DataLoader, TensorDataset

from holy_blocker_ml.eval import EvalResult, evaluate, report


class LogitPassthrough(nn.Module):
    """Treats each input row as the model's logits.

    Lets a test state the exact predictions it wants without training anything.
    """

    def forward(self, x: Tensor) -> Tensor:
        return x


def loader_for(logits: list[list[float]], labels: list[int]) -> DataLoader:
    dataset = TensorDataset(torch.tensor(logits), torch.tensor(labels))
    return DataLoader(dataset, batch_size=2)


PERFECT = [[1.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 1.0]]
ALWAYS_CLEAN = [[1.0, 0.0]] * 4
LABELS = [0, 0, 1, 1]


def test_perfect_model_scores_one() -> None:
    result = evaluate(LogitPassthrough(), loader_for(PERFECT, LABELS))

    assert result.accuracy == 1.0
    assert result.per_class_precision == {0: 1.0, 1: 1.0}
    assert result.per_class_recall == {0: 1.0, 1: 1.0}
    assert result.confusion_matrix == [[2, 0], [0, 2]]


def test_single_class_model_scores_partial_credit() -> None:
    result = evaluate(LogitPassthrough(), loader_for(ALWAYS_CLEAN, LABELS))

    assert result.accuracy == 0.5
    # Rows are true labels, columns are predictions.
    assert result.confusion_matrix == [[2, 0], [2, 0]]
    assert result.per_class_recall == {0: 1.0, 1: 0.0}
    assert result.per_class_precision[0] == 0.5


def test_precision_is_zero_for_a_class_that_is_never_predicted() -> None:
    result = evaluate(LogitPassthrough(), loader_for(ALWAYS_CLEAN, LABELS))

    assert result.per_class_precision[1] == 0.0


def test_evaluate_puts_the_model_in_eval_mode() -> None:
    model = LogitPassthrough()
    model.train()

    evaluate(model, loader_for(PERFECT, LABELS))

    assert model.training is False


def test_evaluate_does_not_build_a_graph() -> None:
    inputs = torch.tensor(PERFECT, requires_grad=True)
    dataset = TensorDataset(inputs, torch.tensor(LABELS))

    evaluate(LogitPassthrough(), DataLoader(dataset, batch_size=2))

    assert inputs.grad is None


def test_report_mentions_the_headline_metrics() -> None:
    result = EvalResult(
        accuracy=0.75,
        per_class_precision={0: 1.0, 1: 0.5},
        per_class_recall={0: 0.5, 1: 1.0},
        confusion_matrix=[[1, 1], [0, 2]],
    )

    text = report(result)

    assert "0.75" in text
    assert "accuracy" in text.lower()
