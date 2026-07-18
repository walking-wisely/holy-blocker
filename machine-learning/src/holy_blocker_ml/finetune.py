"""Fine-tune the backbone against the source corpus.

The linear probe over frozen ImageNet features tops out around 90% because its
errors concentrate on one axis: it cannot separate illustrated *safe* content
from illustrated *explicit* content. `drawings` and `hentai` account for most
false positives and false negatives respectively, while photographic `neutral`
sits near 3%. ImageNet features encode object categories, not that distinction,
so no amount of head training or class reweighting adds the missing capacity —
reweighting only slides the boundary along the same axis, trading one error kind
for the other. The backbone itself has to learn it.

That has a cost. Backbone gradients need pixels, so the archive must stay
readable for the whole run rather than the few minutes extraction takes. It is
still never unpacked, and `--delete-archive` removes it afterwards.

Fine-tuning also invalidates any cached feature artifact: the vectors were
produced by the old backbone. Re-run `holy-blocker-extract --from-checkpoint`
against the new checkpoint to regenerate them.
"""

import argparse
import random
from collections.abc import Sequence
from pathlib import Path

import torch
from torch import nn
from torch.utils.data import DataLoader

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.dataset import ZipImageDataset
from holy_blocker_ml.eval import collect_predictions, report, score
from holy_blocker_ml.labels import BINARY_LABELS, NEGATIVE_INDEX, POSITIVE_INDEX
from holy_blocker_ml.model import create_classifier, trainable_parameter_names, unfreeze_last_blocks
from holy_blocker_ml.train import select_device

CHECKPOINT_NAME = "finetuned-v0.pt"


def stratified_split(
    source_labels: Sequence[str],
    val_fraction: float = 0.2,
    seed: int = 0,
) -> tuple[list[int], list[int]]:
    """Split indices so every source class keeps its proportion in both halves.

    Stratifying on the *source* class rather than the binary label matters here:
    the five classes are what the errors separate along, and a split that
    happened to under-represent `hentai` in validation would hide exactly the
    weakness this run is meant to fix.
    """
    buckets: dict[str, list[int]] = {}
    for index, name in enumerate(source_labels):
        buckets.setdefault(name, []).append(index)

    rng = random.Random(seed)
    train: list[int] = []
    val: list[int] = []
    for name in sorted(buckets):
        members = buckets[name][:]
        rng.shuffle(members)
        cut = round(len(members) * val_fraction)
        val.extend(members[:cut])
        train.extend(members[cut:])
    return sorted(train), sorted(val)


def parameter_groups(
    model: nn.Module,
    backbone_lr: float,
    head_lr: float,
) -> list[dict]:
    """Discriminative learning rates: gentle on the backbone, faster on the head.

    A randomly-initialised head produces large early gradients. Applying those
    at full rate to pretrained convolutions destroys the representation before
    the head has learned anything useful to backpropagate.
    """
    backbone, head = [], []
    for name, parameter in model.named_parameters():
        if not parameter.requires_grad:
            continue
        (head if name.startswith("classifier.") else backbone).append(parameter)

    groups = []
    if backbone:
        groups.append({"name": "backbone", "params": backbone, "lr": backbone_lr})
    if head:
        groups.append({"name": "head", "params": head, "lr": head_lr})
    return groups


def per_source_errors(
    predictions,
    source_labels: Sequence[str],
    threshold: float = 0.5,
) -> dict[str, tuple[int, int]]:
    """Error count and total per source class — the breakdown that shows the axis."""
    stats: dict[str, list[int]] = {}
    for source, target, positive_score in zip(
        source_labels, predictions.targets, predictions.positive_scores, strict=True
    ):
        predicted = POSITIVE_INDEX if positive_score >= threshold else NEGATIVE_INDEX
        entry = stats.setdefault(source, [0, 0])
        entry[1] += 1
        if predicted != target:
            entry[0] += 1
    return {name: (wrong, total) for name, (wrong, total) in stats.items()}


def finetune(
    archive_path: Path,
    config: TrainingConfig,
    unfreeze: int | None = None,
    backbone_lr: float = 1e-4,
    val_fraction: float = 0.2,
    seed: int = 0,
    num_workers: int = 4,
    pretrained: bool = True,
) -> Path:
    """Fine-tune against the archive and save a checkpoint. Returns its path."""
    config.output_dir.mkdir(parents=True, exist_ok=True)
    device = select_device()

    index = ZipImageDataset(archive_path, image_size=config.image_size, augment=False)
    train_idx, val_idx = stratified_split(index.source_labels, val_fraction, seed)
    print(f"train {len(train_idx)}  val {len(val_idx)}")

    train_set = ZipImageDataset(
        archive_path, image_size=config.image_size, augment=True, indices=train_idx
    )
    val_set = ZipImageDataset(
        archive_path, image_size=config.image_size, augment=False, indices=val_idx
    )
    train_loader = DataLoader(
        train_set, batch_size=config.batch_size, shuffle=True, num_workers=num_workers
    )
    val_loader = DataLoader(
        val_set, batch_size=config.batch_size, shuffle=False, num_workers=num_workers
    )

    model = create_classifier(pretrained=pretrained)
    unfreeze_last_blocks(model, count=unfreeze)
    model.to(device)
    print(f"trainable tensors: {len(trainable_parameter_names(model))}")

    criterion = nn.CrossEntropyLoss()
    optimizer = torch.optim.AdamW(
        parameter_groups(model, backbone_lr=backbone_lr, head_lr=config.learning_rate),
        weight_decay=1e-4,
    )
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=config.epochs)

    output_path = config.output_dir / CHECKPOINT_NAME
    best_accuracy = 0.0

    for epoch in range(1, config.epochs + 1):
        model.train()
        total, batches = 0.0, 0
        for images, labels in train_loader:
            images, labels = images.to(device), labels.to(device)
            optimizer.zero_grad(set_to_none=True)
            loss = criterion(model(images), labels)
            loss.backward()
            optimizer.step()
            total += float(loss.detach())
            batches += 1
        scheduler.step()

        model.to("cpu")
        predictions = collect_predictions(model, val_loader)
        result = score(predictions)
        print(f"\nepoch {epoch}/{config.epochs}  train_loss={total / max(batches, 1):.4f}")
        print(report(result))

        breakdown = per_source_errors(predictions, val_set.source_labels)
        print(f"\n{'source':<10}{'n':>7}{'errors':>8}{'rate':>8}")
        for name in sorted(breakdown):
            wrong, count = breakdown[name]
            print(f"{name:<10}{count:>7}{wrong:>8}{wrong / count:>8.1%}")

        if result.accuracy > best_accuracy:
            best_accuracy = result.accuracy
            torch.save(
                {
                    "model_state": model.state_dict(),
                    "labels": list(BINARY_LABELS),
                    # The backbone moved, so cached vectors no longer apply.
                    "frozen_backbone": False,
                    "unfreeze": unfreeze,
                    "epoch": epoch,
                    "val_accuracy": result.accuracy,
                },
                output_path,
            )
            print(f"  saved (best so far: {best_accuracy:.4f})")
        model.to(device)

    return output_path


def main() -> None:
    parser = argparse.ArgumentParser(description="Fine-tune the backbone on the source corpus.")
    parser.add_argument("--archive", type=Path, required=True, help="path to the corpus zip")
    parser.add_argument("--output-dir", type=Path, default=Path("artifacts"))
    parser.add_argument("--image-size", type=int, default=TrainingConfig.image_size)
    parser.add_argument("--batch-size", type=int, default=TrainingConfig.batch_size)
    parser.add_argument("--epochs", type=int, default=4)
    parser.add_argument("--head-lr", type=float, default=1e-3)
    parser.add_argument("--backbone-lr", type=float, default=1e-4)
    parser.add_argument(
        "--unfreeze",
        type=int,
        default=None,
        help="feature blocks to unfreeze from the end; omit for the whole backbone",
    )
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument(
        "--delete-archive",
        action="store_true",
        help="remove the corpus once training finishes",
    )
    args = parser.parse_args()

    config = TrainingConfig(
        output_dir=args.output_dir,
        image_size=args.image_size,
        batch_size=args.batch_size,
        epochs=args.epochs,
        learning_rate=args.head_lr,
    )
    try:
        artifact = finetune(
            args.archive,
            config,
            unfreeze=args.unfreeze,
            backbone_lr=args.backbone_lr,
            num_workers=args.workers,
        )
        print(f"\nsaved {artifact}")
        print("cached feature artifacts are now stale — regenerate with")
        print(f"  holy-blocker-extract --from-checkpoint {artifact} --archive {args.archive}")
    finally:
        if args.delete_archive and args.archive.exists():
            args.archive.unlink()
            print(f"removed {args.archive}")
