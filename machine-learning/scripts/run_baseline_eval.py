#!/usr/bin/env python3
"""Generate the synthetic UI/text corpus (if missing) and evaluate the
baseline pretrained classifier (Falconsai/nsfw_image_detection by default)
against it, printing a score-distribution + threshold-sweep report.

Usage:
    .venv/bin/python scripts/run_baseline_eval.py [--count 300] [--seed 0]
        [--data-dir data/eval/synthetic-ui] [--model Falconsai/nsfw_image_detection]

Corpus data lands under the gitignored `data/` directory and is never
committed.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from holy_blocker_ml import eval as eval_mod
from holy_blocker_ml import model as model_mod
from holy_blocker_ml import synth_ui
from holy_blocker_ml.corpus import CorpusKind, CorpusSpec


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--count", type=int, default=300)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument(
        "--data-dir",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "data" / "eval" / "synthetic-ui",
    )
    parser.add_argument("--model", default=model_mod.DEFAULT_MODEL_ID)
    args = parser.parse_args()

    existing = sorted(args.data_dir.glob("*.png")) if args.data_dir.is_dir() else []
    if len(existing) < args.count:
        print(f"generating {args.count} synthetic UI images into {args.data_dir} ...")
        synth_ui.generate_corpus(args.data_dir, args.count, args.seed)
    else:
        print(f"reusing {len(existing)} existing images in {args.data_dir}")

    print(f"loading {args.model} ...")
    classifier = model_mod.load_classifier(args.model)
    print(
        f"resolved NSFW class index {classifier.nsfw_index} "
        f"from id2label={dict(classifier.model.config.id2label)}"
    )

    spec = CorpusSpec(name="synthetic-ui", root=args.data_dir, kind=CorpusKind.BENIGN)
    result = eval_mod.evaluate_corpus(
        spec, lambda p: model_mod.score_path(classifier, p)
    )
    print()
    print(eval_mod.report(result))


if __name__ == "__main__":
    main()
