"""CLI: turn a gated NSFW corpus into a feature artifact, then delete the source.

    holy-blocker-extract --out data/eval/nsfw_detect.npz

`deepghs/nsfw_detect` ships as a single zip rather than parquet shards, so it
cannot be streamed — the archive is downloaded whole. It is never unpacked:
members are decoded in memory, and the archive is deleted once extraction
finishes (`--keep-archive` opts out). That download window is the only moment
the material exists on disk in a decodable form.

Access is gated with automatic approval:

1. Open https://huggingface.co/datasets/deepghs/nsfw_detect and accept the terms.
2. Create a read token at https://huggingface.co/settings/tokens.
3. `export HF_TOKEN=hf_...`
"""

import argparse
from pathlib import Path

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.dataset import build_transform
from holy_blocker_ml.env import load_dotenv, require_env
from holy_blocker_ml.features import (
    DEFAULT_LABEL_POLICY,
    ArchiveLayoutError,
    FeatureSet,
    extract_features,
    inspect_archive,
    iter_zip_images,
    save_feature_set,
)
from holy_blocker_ml.labels import EXPLICIT
from holy_blocker_ml.model import create_feature_extractor

DATASET_REPO = "deepghs/nsfw_detect"
ARCHIVE_NAME = "nsfw_dataset_v1.zip"


GATE_HINT = (
    "This dataset is gated (automatic approval):\n"
    "  1. Accept the terms at https://huggingface.co/datasets/deepghs/nsfw_detect\n"
    "  2. Create a read token at https://huggingface.co/settings/tokens\n"
    "  3. Put it in .env as HF_TOKEN=hf_..."
)


def download_archive(repo_id: str, filename: str, destination: Path) -> Path:
    """Fetch the dataset archive from the Hub, failing loudly if inaccessible."""
    from huggingface_hub import hf_hub_download
    from huggingface_hub.errors import GatedRepoError, RepositoryNotFoundError

    token = require_env("HF_TOKEN", hint=GATE_HINT)
    destination.mkdir(parents=True, exist_ok=True)
    try:
        return Path(
            hf_hub_download(
                repo_id=repo_id,
                filename=filename,
                repo_type="dataset",
                local_dir=destination,
                token=token,
            )
        )
    except GatedRepoError as error:
        raise SystemExit(
            f"{repo_id} is gated and this token has not been granted access.\n\n{GATE_HINT}"
        ) from error
    except RepositoryNotFoundError as error:
        raise SystemExit(
            f"{repo_id} not found, or the token lacks read access to it.\n\n{GATE_HINT}"
        ) from error


def extract_hf_dataset(
    output_path: Path,
    config: TrainingConfig,
    repo_id: str = DATASET_REPO,
    filename: str = ARCHIVE_NAME,
    work_dir: Path | None = None,
    policy=DEFAULT_LABEL_POLICY,
    keep_archive: bool = False,
    pretrained: bool = True,
    archive: Path | None = None,
    from_checkpoint: Path | None = None,
) -> FeatureSet:
    """Download, extract features in memory, save, and remove the archive.

    `archive` skips the download and uses a local zip — for when the file was
    fetched manually. A supplied archive is never deleted.
    """
    if archive is not None:
        archive_path, keep_archive = Path(archive), True
    else:
        work_dir = work_dir or output_path.parent / ".archive"
        archive_path = download_archive(repo_id, filename, work_dir)

    try:
        summary = inspect_archive(archive_path)
        print(f"archive layout:\n{summary.describe()}\n")

        if from_checkpoint is not None:
            # Regenerate vectors with a fine-tuned backbone: features are only
            # valid against the backbone that produced them.
            import torch

            from holy_blocker_ml.labels import BINARY_LABELS
            from holy_blocker_ml.model import BackboneFeatures, create_classifier

            state = torch.load(from_checkpoint, map_location="cpu", weights_only=False)
            source = create_classifier(class_count=len(BINARY_LABELS), pretrained=False)
            source.load_state_dict(state["model_state"])
            backbone = BackboneFeatures(source).eval()
        else:
            backbone = create_feature_extractor(pretrained=pretrained)
        transform = build_transform(config.image_size, augment=False)
        result = extract_features(
            iter_zip_images(archive_path),
            backbone,
            transform,
            policy=policy,
            batch_size=config.batch_size,
        )
        result.metadata |= {
            "source_repo": repo_id,
            "source_file": filename,
            "image_size": config.image_size,
            "pretrained_backbone": pretrained,
            "from_checkpoint": str(from_checkpoint) if from_checkpoint else None,
        }
        save_feature_set(result, output_path)
    finally:
        if not keep_archive and archive_path.exists():
            archive_path.unlink()

    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--out", type=Path, default=Path("data/eval/nsfw_detect.npz"))
    parser.add_argument("--repo", default=DATASET_REPO)
    parser.add_argument("--file", default=ARCHIVE_NAME)
    parser.add_argument(
        "--archive",
        type=Path,
        help="use an already-downloaded zip instead of fetching it (never deleted)",
    )
    parser.add_argument(
        "--from-checkpoint",
        type=Path,
        help="extract using this checkpoint's backbone instead of pristine ImageNet "
        "weights; required after fine-tuning",
    )
    parser.add_argument("--image-size", type=int, default=TrainingConfig.image_size)
    parser.add_argument("--batch-size", type=int, default=TrainingConfig.batch_size)
    parser.add_argument(
        "--strict-sexy",
        action="store_true",
        help="treat the 'sexy' class as explicit instead of a hard negative",
    )
    parser.add_argument(
        "--keep-archive",
        action="store_true",
        help="do not delete the downloaded archive afterwards",
    )
    parser.add_argument(
        "--inspect",
        action="store_true",
        help="report the archive layout and exit without extracting",
    )
    args = parser.parse_args()

    load_dotenv()

    if args.inspect:
        archive = args.archive or download_archive(
            args.repo, args.file, args.out.parent / ".archive"
        )
        print(inspect_archive(archive).describe())
        return

    policy = dict(DEFAULT_LABEL_POLICY)
    if args.strict_sexy:
        policy["sexy"] = EXPLICIT

    config = TrainingConfig(image_size=args.image_size, batch_size=args.batch_size)
    try:
        result = extract_hf_dataset(
            args.out,
            config,
            repo_id=args.repo,
            filename=args.file,
            policy=policy,
            keep_archive=args.keep_archive,
            archive=args.archive,
            from_checkpoint=args.from_checkpoint,
        )
    except (ArchiveLayoutError, ValueError) as error:
        # A layout mismatch is a setup problem, not a crash: report it as one
        # so the diagnostic is the first thing seen rather than a traceback.
        raise SystemExit(f"extraction failed: {error}") from error

    print(f"wrote {args.out}  ({len(result)} samples, dim {result.metadata['feature_dim']})")
    for name, count in sorted(result.metadata["source_counts"].items()):
        print(f"  {name:<10}{count:>8}  -> {policy[name]}")
    if result.metadata["dropped"]:
        print(f"  dropped (unmapped classes): {result.metadata['dropped']}")
