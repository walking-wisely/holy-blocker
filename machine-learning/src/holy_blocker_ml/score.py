"""Score a checkpoint on the frozen holdouts the experiments are judged against.

`harness.py` evaluates a directory of labelled images or a cached feature set.
Neither fits here: the experiment holdouts are *slices of the source archive* —
the validation split produced by `stratified_split(seed=0, val_fraction=0.2)`,
and the 1,147-sample common holdout nested inside it. Reproducing a published
result therefore has to start from the archive and the same split parameters.

This exists because the per-medium numbers in
[results.md](../../../docs/components/machine-learning/results.md) were produced
by a slice that was never committed, which left the
[anime subsampling experiment](../../../docs/components/machine-learning/experiments/anime-subsampling.md)
resting on figures nothing in the package could regenerate.

Fine-tuning drifts the backbone away from any cached feature vectors, so scoring
a fine-tuned checkpoint runs from pixels rather than from a feature artifact.
"""

import argparse
from collections.abc import Sequence
from pathlib import Path

import torch
from torch.utils.data import DataLoader

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.dataset import ZipImageDataset
from holy_blocker_ml.eval import Predictions, collect_predictions, score
from holy_blocker_ml.medium import medium_report
from holy_blocker_ml.metrics import fpr_at_fnr, report_metrics
from holy_blocker_ml.model import create_classifier

#: Split parameters every published run shares. Changing either of these makes a
#: result incomparable to the pre-registered baselines, because the validation
#: samples themselves change.
BASELINE_SEED = 0
BASELINE_VAL_FRACTION = 0.2


def require_subset(
    inner: Sequence[int],
    outer: Sequence[int],
    inner_name: str,
    outer_name: str,
) -> None:
    """Assert that one index set is contained in another.

    The common holdout is only meaningful while it stays inside the validation
    split. If the split ever changes, indices leak into training data and the
    resulting score is inflated rather than wrong-looking — so this fails loudly
    instead of scoring whatever overlap remains.
    """
    stray = sorted(set(inner) - set(outer))
    if stray:
        raise ValueError(
            f"{inner_name} contains {len(stray)} indices outside the {outer_name} "
            f"({stray[:10]}{'...' if len(stray) > 10 else ''}); the split parameters "
            "no longer match the ones these holdouts were built from"
        )


def positions_within(wanted: Sequence[int], scored: Sequence[int]) -> list[int]:
    """Positions of `wanted` inside `scored`, in `scored`'s own order.

    Predictions come back in dataset order, so a caller holding archive indices
    needs their offsets rather than the indices themselves.
    """
    lookup = {index: position for position, index in enumerate(scored)}
    missing = sorted(set(wanted) - lookup.keys())
    if missing:
        raise ValueError(f"indices were never scored: {missing[:10]}")
    return sorted(lookup[index] for index in wanted)


def take(predictions: Predictions, positions: Sequence[int]) -> Predictions:
    """Restrict predictions to the given positions, preserving order."""
    return Predictions(
        targets=[predictions.targets[p] for p in positions],
        positive_scores=[predictions.positive_scores[p] for p in positions],
    )


def load_for_inference(checkpoint_path: Path) -> torch.nn.Module:
    """Load a checkpoint for scoring, without ImageNet weights it will overwrite."""
    state = torch.load(checkpoint_path, map_location="cpu", weights_only=False)
    model = create_classifier(pretrained=False)
    model.load_state_dict(state["model_state"])
    model.eval()
    return model


def report_holdout(
    name: str,
    predictions: Predictions,
    source_labels: Sequence[str],
    miss_budget: float,
) -> str:
    """Render one holdout: per-medium AUC, accuracy, and the miss-budget cost."""
    lines = [f"== {name} ({len(predictions)} samples)", ""]
    lines.append(medium_report(predictions, source_labels))
    lines.append("")
    lines.append(f"accuracy @ 0.5:              {score(predictions).accuracy:.4f}")
    try:
        cost = f"{fpr_at_fnr(predictions, miss_budget):.4f}"
    except ValueError:
        cost = "undefined"
    lines.append(f"FP rate at {miss_budget:.0%} miss budget:  {cost}")
    lines.append("")
    lines.append(report_metrics(predictions))
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Score a checkpoint on the frozen experiment holdouts."
    )
    parser.add_argument("--archive", type=Path, required=True, help="path to the corpus zip")
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument(
        "--common-idx",
        type=Path,
        help="npy file of archive indices held out from every model's training; "
        "reported as a second holdout when given",
    )
    parser.add_argument(
        "--split",
        choices=("val", "train"),
        default="val",
        help="which half to score. 'train' measures the fit the model achieved on "
        "data it saw, which is what distinguishes underfitting from overfitting",
    )
    parser.add_argument("--seed", type=int, default=BASELINE_SEED)
    parser.add_argument("--val-fraction", type=float, default=BASELINE_VAL_FRACTION)
    parser.add_argument("--miss-budget", type=float, default=0.05)
    parser.add_argument("--image-size", type=int, default=TrainingConfig.image_size)
    parser.add_argument("--batch-size", type=int, default=64)
    args = parser.parse_args()

    from holy_blocker_ml.finetune import stratified_split

    index = ZipImageDataset(args.archive, image_size=args.image_size, augment=False)
    train_idx, val_idx = stratified_split(index.source_labels, args.val_fraction, args.seed)
    scored_idx = train_idx if args.split == "train" else val_idx

    # augment=False on the training half too: this measures the fit that was
    # achieved, and augmentation would score a distribution the model never
    # settled on.
    scored_set = ZipImageDataset(
        args.archive, image_size=args.image_size, augment=False, indices=scored_idx
    )
    model = load_for_inference(args.checkpoint)
    loader = DataLoader(scored_set, batch_size=args.batch_size, shuffle=False, num_workers=0)
    predictions = collect_predictions(model, loader)
    sources = scored_set.source_labels

    print(f"checkpoint: {args.checkpoint}")
    print(f"split:      {args.split}  seed={args.seed} val_fraction={args.val_fraction}")
    print()
    print(report_holdout(f"{args.split} split", predictions, sources, args.miss_budget))

    if args.common_idx and args.split == "val":
        import numpy as np

        common = np.load(args.common_idx).tolist()
        require_subset(common, scored_idx, "common holdout", "validation split")
        positions = positions_within(common, scored_idx)
        print()
        print(
            report_holdout(
                "common holdout",
                take(predictions, positions),
                [sources[p] for p in positions],
                args.miss_budget,
            )
        )
