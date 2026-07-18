"""Fine-tuning loop for the binary baseline classifier.

Expects the layout described in `docs/components/machine-learning/plan.md`:

    <data_dir>/train/<label>/<image>
    <data_dir>/val/<label>/<image>

Both directories are gitignored — no imagery lives in the repo.
"""

from pathlib import Path

import torch
from torch import nn
from torch.utils.data import DataLoader

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.dataset import load_dataset
from holy_blocker_ml.eval import EvalResult, evaluate, report
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.model import create_classifier, freeze_backbone

CHECKPOINT_NAME = "baseline-v0.pt"


def select_device() -> torch.device:
    """Prefer Apple Silicon's MPS backend, then CUDA, then CPU."""
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def run_epoch(
    model: nn.Module,
    loader: DataLoader,
    criterion: nn.Module,
    optimizer: torch.optim.Optimizer,
    device: torch.device,
) -> float:
    """Train for one pass over `loader`; return the mean batch loss."""
    model.train()
    total_loss = 0.0
    batches = 0

    for images, labels in loader:
        images, labels = images.to(device), labels.to(device)
        optimizer.zero_grad(set_to_none=True)
        loss = criterion(model(images), labels)
        loss.backward()
        optimizer.step()
        total_loss += float(loss.detach())
        batches += 1

    return total_loss / batches if batches else 0.0


def train(config: TrainingConfig, pretrained: bool = True, frozen_backbone: bool = False) -> Path:
    """Fine-tune the classifier and save a checkpoint. Returns the checkpoint path.

    `frozen_backbone=True` trains only the head, which keeps the backbone
    identical to the one that produced any cached feature artifacts. Use it when
    evaluating against `.npz` features; leave it off for best accuracy.
    """
    config.output_dir.mkdir(parents=True, exist_ok=True)
    device = select_device()

    train_loader = load_dataset(
        config.data_dir / "train",
        image_size=config.image_size,
        augment=True,
        batch_size=config.batch_size,
    )
    val_dir = config.data_dir / "val"
    val_loader = (
        load_dataset(
            val_dir,
            image_size=config.image_size,
            augment=False,
            batch_size=config.batch_size,
        )
        if val_dir.is_dir()
        else None
    )

    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=pretrained).to(device)
    if frozen_backbone:
        freeze_backbone(model)
    criterion = nn.CrossEntropyLoss()
    optimizer = torch.optim.AdamW(
        [parameter for parameter in model.parameters() if parameter.requires_grad],
        lr=config.learning_rate,
    )

    result: EvalResult | None = None
    for epoch in range(1, config.epochs + 1):
        loss = run_epoch(model, train_loader, criterion, optimizer, device)
        print(f"epoch {epoch}/{config.epochs}  train_loss={loss:.4f}")
        if val_loader is not None:
            # Evaluation runs on CPU so metrics are identical to what the
            # export/eval harness reports on a machine without an accelerator.
            result = evaluate(model.to("cpu"), val_loader)
            print(report(result))
            model.to(device)

    output_path = config.output_dir / CHECKPOINT_NAME
    torch.save(
        {
            "model_state": model.to("cpu").state_dict(),
            "config": {key: str(value) for key, value in config.__dict__.items()},
            # Baked in so an artifact can never be read back with the class
            # order inverted; `harness.py` checks this against BINARY_LABELS.
            "labels": list(BINARY_LABELS),
            # Cached feature artifacts are only valid against a frozen backbone.
            "frozen_backbone": frozen_backbone,
        },
        output_path,
    )
    return output_path


def main() -> None:
    artifact = train(TrainingConfig())
    print(f"saved training artifact: {artifact}")
