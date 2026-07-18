"""CLI evaluation harness — the file-I/O edge around `eval.py`.

Points a trained checkpoint at a local evaluation set and reports false
positives and false negatives, a threshold sweep, and the worst individual
misclassifications so they can be inspected by hand.

The evaluation set is never committed. Point `--data-dir` at a local directory:

    data/eval/safe/<image>
    data/eval/explicit/<image>

Typical use with a public NSFW benchmark:

    holy-blocker-eval --checkpoint artifacts/baseline-v0.pt --data-dir data/eval
"""

import argparse
import warnings
from pathlib import Path

import torch
from torch import nn

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.dataset import load_dataset
from holy_blocker_ml.eval import (
    DEFAULT_SWEEP,
    DEFAULT_THRESHOLD,
    EvalResult,
    Predictions,
    collect_predictions,
    misclassified,
    report,
    report_sweep,
    score,
    sweep_thresholds,
)
from holy_blocker_ml.labels import BINARY_LABELS
from holy_blocker_ml.model import create_classifier


def load_checkpoint(checkpoint_path: Path) -> nn.Module:
    """Rebuild the classifier from a saved checkpoint.

    Refuses a checkpoint whose recorded label order disagrees with the current
    `BINARY_LABELS` — that mismatch would silently swap every FP and FN.
    """
    checkpoint = torch.load(checkpoint_path, map_location="cpu", weights_only=False)

    saved_labels = checkpoint.get("labels")
    if saved_labels is not None and list(saved_labels) != list(BINARY_LABELS):
        raise ValueError(
            f"checkpoint label order {list(saved_labels)} does not match "
            f"{list(BINARY_LABELS)}; false-positive/negative counts would be inverted"
        )

    model = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
    model.load_state_dict(checkpoint["model_state"])
    model.eval()
    return model


def evaluate_checkpoint(
    checkpoint_path: Path,
    data_dir: Path,
    config: TrainingConfig,
    threshold: float = DEFAULT_THRESHOLD,
) -> tuple[EvalResult, Predictions]:
    """Score a checkpoint against a local evaluation directory."""
    model = load_checkpoint(checkpoint_path)
    loader = load_dataset(
        data_dir,
        image_size=config.image_size,
        augment=False,
        batch_size=config.batch_size,
    )
    predictions = collect_predictions(model, loader)
    return score(predictions, threshold=threshold), predictions


def evaluate_feature_set(
    checkpoint_path: Path,
    features_path: Path,
    threshold: float = DEFAULT_THRESHOLD,
    batch_size: int = 256,
) -> tuple[EvalResult, Predictions]:
    """Score a checkpoint against cached feature vectors — no images involved.

    Only the classifier head runs: the stored vectors are already the backbone's
    output. `Predictions.paths` stays empty, so no file path can be surfaced for
    inspection.
    """
    import numpy as np
    from torch.utils.data import DataLoader, TensorDataset

    from holy_blocker_ml.features import load_feature_set
    from holy_blocker_ml.model import BACKBONE_FEATURE_DIM

    feature_set = load_feature_set(features_path)

    stored_labels = feature_set.metadata.get("labels")
    if stored_labels is not None and list(stored_labels) != list(BINARY_LABELS):
        raise ValueError(
            f"feature artifact label order {list(stored_labels)} does not match "
            f"{list(BINARY_LABELS)}; false-positive/negative counts would be inverted"
        )

    width = feature_set.features.shape[1] if len(feature_set) else BACKBONE_FEATURE_DIM
    if width != BACKBONE_FEATURE_DIM:
        raise ValueError(
            f"feature artifact has width {width}, but this backbone emits "
            f"{BACKBONE_FEATURE_DIM}; re-run extraction against the source corpus"
        )

    # A fine-tuned backbone no longer matches the one that produced the cached
    # vectors, so the head would be scoring features it never saw. This is a
    # warning rather than an error: the checkpoint may predate the flag.
    checkpoint = torch.load(checkpoint_path, map_location="cpu", weights_only=False)
    if checkpoint.get("frozen_backbone") is not True:
        warnings.warn(
            f"{checkpoint_path} was not trained with a frozen backbone, so its "
            "backbone differs from the one that produced these features; the "
            "resulting rates are not trustworthy. Retrain with "
            "frozen_backbone=True, or re-extract features from this checkpoint.",
            RuntimeWarning,
            stacklevel=2,
        )

    head = load_checkpoint(checkpoint_path).classifier
    loader = DataLoader(
        TensorDataset(
            torch.from_numpy(np.ascontiguousarray(feature_set.features)),
            torch.from_numpy(np.ascontiguousarray(feature_set.labels)),
        ),
        batch_size=batch_size,
    )

    predictions = collect_predictions(head, loader)
    return score(predictions, threshold=threshold), predictions


def format_examples(
    predictions: Predictions,
    threshold: float,
    limit: int,
) -> str:
    """List the most confident mistakes of each kind, for manual triage."""
    false_positives, false_negatives = misclassified(predictions, threshold=threshold)
    if not false_positives and not false_negatives:
        return ""

    lines = ["", f"worst false positives (safe scored as explicit), top {limit}:"]
    lines += [f"  {s:.4f}  {p}" for p, s in false_positives[:limit]] or ["  none"]
    lines += ["", f"worst false negatives (explicit scored as safe), top {limit}:"]
    lines += [f"  {s:.4f}  {p}" for p, s in false_negatives[:limit]] or ["  none"]
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description="Evaluate a checkpoint for false positives/negatives.")
    parser.add_argument("--checkpoint", type=Path, default=Path("artifacts/baseline-v0.pt"))
    parser.add_argument(
        "--data-dir",
        type=Path,
        default=Path("data/eval"),
        help="directory containing one subdirectory per label (gitignored)",
    )
    parser.add_argument(
        "--features",
        type=Path,
        help="evaluate cached feature vectors instead of images; takes precedence "
        "over --data-dir and never surfaces file paths",
    )
    parser.add_argument("--threshold", type=float, default=DEFAULT_THRESHOLD)
    parser.add_argument("--image-size", type=int, default=TrainingConfig.image_size)
    parser.add_argument("--batch-size", type=int, default=TrainingConfig.batch_size)
    parser.add_argument(
        "--examples",
        type=int,
        default=0,
        help="list this many misclassified files per kind. Defaults to 0: the "
        "listing exists for manual inspection, which is not always wanted",
    )
    parser.add_argument("--no-sweep", action="store_true", help="skip the threshold sweep table")
    args = parser.parse_args()

    config = TrainingConfig(image_size=args.image_size, batch_size=args.batch_size)
    if args.features:
        result, predictions = evaluate_feature_set(
            args.checkpoint, args.features, threshold=args.threshold
        )
    else:
        result, predictions = evaluate_checkpoint(
            args.checkpoint, args.data_dir, config, threshold=args.threshold
        )

    print(report(result))

    from holy_blocker_ml.metrics import report_metrics

    print()
    print(report_metrics(predictions))

    if not args.no_sweep:
        print("\nthreshold sweep")
        print(report_sweep(sweep_thresholds(predictions, DEFAULT_SWEEP)))
    print(format_examples(predictions, args.threshold, args.examples))
