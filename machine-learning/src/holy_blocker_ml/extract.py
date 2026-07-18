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
import os
from pathlib import Path

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.dataset import build_transform
from holy_blocker_ml.features import (
    DEFAULT_LABEL_POLICY,
    FeatureSet,
    extract_features,
    iter_zip_images,
    save_feature_set,
)
from holy_blocker_ml.labels import EXPLICIT
from holy_blocker_ml.model import create_feature_extractor

DATASET_REPO = "deepghs/nsfw_detect"
ARCHIVE_NAME = "nsfw_dataset_v1.zip"


def download_archive(repo_id: str, filename: str, destination: Path) -> Path:
    """Fetch the dataset archive from the Hub, failing loudly if ungated."""
    from huggingface_hub import hf_hub_download
    from huggingface_hub.errors import GatedRepoError

    destination.mkdir(parents=True, exist_ok=True)
    try:
        return Path(
            hf_hub_download(
                repo_id=repo_id,
                filename=filename,
                repo_type="dataset",
                local_dir=destination,
                token=os.environ.get("HF_TOKEN"),
            )
        )
    except GatedRepoError as error:
        raise SystemExit(
            f"{repo_id} is gated. Accept the terms at "
            f"https://huggingface.co/datasets/{repo_id}, create a read token at "
            "https://huggingface.co/settings/tokens, then set HF_TOKEN."
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
    args = parser.parse_args()

    policy = dict(DEFAULT_LABEL_POLICY)
    if args.strict_sexy:
        policy["sexy"] = EXPLICIT

    config = TrainingConfig(image_size=args.image_size, batch_size=args.batch_size)
    result = extract_hf_dataset(
        args.out,
        config,
        repo_id=args.repo,
        filename=args.file,
        policy=policy,
        keep_archive=args.keep_archive,
        archive=args.archive,
    )

    print(f"wrote {args.out}  ({len(result)} samples, dim {result.metadata['feature_dim']})")
    for name, count in sorted(result.metadata["source_counts"].items()):
        print(f"  {name:<10}{count:>8}  -> {policy[name]}")
    if result.metadata["dropped"]:
        print(f"  dropped (unmapped classes): {result.metadata['dropped']}")
