from pathlib import Path

import torch
from torch import nn
from torch.utils.data import DataLoader, TensorDataset

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.train import CHECKPOINT_NAME, run_epoch, select_device, train

from tests.conftest import write_image


def dataset_at(root: Path, per_class: int = 2) -> Path:
    for index in range(per_class):
        write_image(root / "safe" / f"{index}.png", (0, 128 + index, 0))
        write_image(root / "explicit" / f"{index}.png", (128 + index, 0, 0))
    return root


def test_run_epoch_reduces_loss_on_a_trivially_separable_batch() -> None:
    torch.manual_seed(0)
    features = torch.tensor([[1.0, 0.0], [0.0, 1.0]] * 8)
    targets = torch.tensor([0, 1] * 8)
    loader = DataLoader(TensorDataset(features, targets), batch_size=4)

    model = nn.Linear(2, len(BINARY_LABELS))
    criterion = nn.CrossEntropyLoss()
    optimizer = torch.optim.AdamW(model.parameters(), lr=0.1)
    device = torch.device("cpu")

    first = run_epoch(model, loader, criterion, optimizer, device)
    for _ in range(10):
        last = run_epoch(model, loader, criterion, optimizer, device)

    assert last < first


def test_run_epoch_leaves_the_model_in_train_mode() -> None:
    model = nn.Linear(2, 2)
    model.eval()
    loader = DataLoader(TensorDataset(torch.randn(2, 2), torch.tensor([0, 1])), batch_size=2)

    run_epoch(model, loader, nn.CrossEntropyLoss(), torch.optim.AdamW(model.parameters()), torch.device("cpu"))

    assert model.training


def test_run_epoch_on_an_empty_loader_returns_zero_rather_than_dividing_by_zero() -> None:
    model = nn.Linear(2, 2)
    empty = DataLoader(TensorDataset(torch.empty(0, 2), torch.empty(0, dtype=torch.long)), batch_size=2)

    loss = run_epoch(model, empty, nn.CrossEntropyLoss(), torch.optim.AdamW(model.parameters()), torch.device("cpu"))

    assert loss == 0.0


def test_train_writes_a_checkpoint_carrying_the_label_order(tmp_path: Path) -> None:
    dataset_at(tmp_path / "data" / "train")
    dataset_at(tmp_path / "data" / "val")
    config = TrainingConfig(
        data_dir=tmp_path / "data",
        output_dir=tmp_path / "artifacts",
        image_size=32,
        batch_size=2,
        epochs=1,
    )

    checkpoint_path = train(config, pretrained=False)

    assert checkpoint_path == tmp_path / "artifacts" / CHECKPOINT_NAME
    checkpoint = torch.load(checkpoint_path, map_location="cpu", weights_only=False)
    assert checkpoint["labels"] == list(BINARY_LABELS)
    assert "model_state" in checkpoint


def test_train_runs_without_a_validation_split(tmp_path: Path) -> None:
    dataset_at(tmp_path / "data" / "train")
    config = TrainingConfig(
        data_dir=tmp_path / "data",
        output_dir=tmp_path / "artifacts",
        image_size=32,
        batch_size=2,
        epochs=1,
    )

    assert train(config, pretrained=False).exists()


def test_select_device_returns_a_usable_device() -> None:
    device = select_device()

    assert device.type in {"cpu", "cuda", "mps"}
    torch.zeros(1, device=device)
