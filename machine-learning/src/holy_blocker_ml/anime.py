"""Subsample `deepghs/anime_dbrating` into a drawn-medium training supplement.

Implements the [anime subsampling experiment](../../../docs/components/machine-learning/experiments/anime-subsampling.md):
does drawn training data from a better-labelled source close the drawn/photographic
gap, without degrading photographic performance?

## Why substitution rather than addition

The experiment's method said to subsample "so the combined set holds that ratio"
(40% drawn / 60% photographic). The corpus is *already* exactly 40% drawn —
11,200 of 28,000 — so adding any drawn data breaks the ratio and only n=0
satisfies the rule as written. The protocol was amended before the run to hold
drawn volume fixed and swap part of it for anime data instead.

That is also the stronger experiment. The case for this dataset is about label
*quality* — `questionable` exists as a boundary class, and labels come from
community moderation rather than subreddit provenance — not about volume. And
the pre-registered prediction is that capacity, not data volume, is binding.
Holding volume fixed isolates the variable actually being argued about.

## What must not move

Baselines are fixed on `stratified_split(seed=0, val_fraction=0.2)` over the
original archive. So:

- **The validation half is never touched.** Substitution only removes samples
  from the training half, so `val_indices` comes back identical to the
  unmodified split and the frozen holdouts stay comparable.
- **Photographic training data is never touched.** The decision rule rejects on
  a photographic regression, which is only interpretable if that half is held
  fixed.
- **`questionable` is held out of training** by being absent from
  `ANIME_LABEL_POLICY`: `map_source_label` returns None for unmapped classes and
  `ZipImageDataset` drops them. Enforced by the pipeline, not by convention.
"""

import random
import zipfile
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path

from holy_blocker_ml.finetune import stratified_split
from holy_blocker_ml.labels import EXPLICIT, SAFE

#: Danbooru's four ordinal ratings, prefixed so they cannot collide with the
#: five `nsfw_detect` classes in a shared source-label column.
GENERAL = "anime_general"
SENSITIVE = "anime_sensitive"
QUESTIONABLE = "anime_questionable"
ANIME_EXPLICIT = "anime_explicit"

#: Ratings that stand in for the dropped `drawings` (safe drawn content).
ANIME_SAFE_SOURCES: tuple[str, ...] = (GENERAL, SENSITIVE)
#: Ratings that stand in for the dropped `hentai` (explicit drawn content).
ANIME_EXPLICIT_SOURCES: tuple[str, ...] = (ANIME_EXPLICIT,)
#: Everything that may enter training. `questionable` is deliberately excluded.
ANIME_DRAWN_SOURCES: tuple[str, ...] = (*ANIME_SAFE_SOURCES, *ANIME_EXPLICIT_SOURCES)

#: Every anime source class, including the held-out boundary set.
ANIME_SOURCE_CLASSES: tuple[str, ...] = (*ANIME_DRAWN_SOURCES, QUESTIONABLE)

#: Rating -> binary policy. `questionable` is absent on purpose: assigning it a
#: side would re-import exactly the arbitrariness this dataset was chosen to fix.
ANIME_LABEL_POLICY: dict[str, str] = {
    GENERAL: SAFE,
    SENSITIVE: SAFE,
    ANIME_EXPLICIT: EXPLICIT,
}

#: Archive member per rating. The counts are part of the published filenames on
#: the Hub; they are not assertions about how many samples this package uses.
RATING_ARCHIVES: dict[str, str] = {
    GENERAL: "general_341506.zip",
    SENSITIVE: "sensitive_341769.zip",
    QUESTIONABLE: "questionable_320107.zip",
    ANIME_EXPLICIT: "explicit_278119.zip",
}

DATASET_REPO = "deepghs/anime_dbrating"

#: Which drawn source each group of anime ratings replaces. Substituting
#: like-for-like keeps the safe/explicit balance of the drawn half intact, so a
#: shift in the decision threshold cannot be mistaken for a modelling gain.
REPLACES: dict[str, tuple[str, ...]] = {
    "drawings": ANIME_SAFE_SOURCES,
    "hentai": ANIME_EXPLICIT_SOURCES,
}


@dataclass(frozen=True)
class SubstitutionPlan:
    """Which original samples to drop, and how many anime samples replace them."""

    val_indices: list[int]
    kept_train_indices: list[int]
    dropped_indices: list[int]
    anime_counts: dict[str, int]

    @property
    def anime_total(self) -> int:
        return sum(self.anime_counts.values())

    def describe(self) -> str:
        lines = [
            f"validation half (frozen): {len(self.val_indices)}",
            f"training half:            {len(self.kept_train_indices)} kept "
            f"+ {self.anime_total} anime = "
            f"{len(self.kept_train_indices) + self.anime_total}",
            f"dropped from training:    {len(self.dropped_indices)}",
            "",
            f"{'anime source':<20}{'n':>8}",
        ]
        for source in ANIME_DRAWN_SOURCES:
            if source in self.anime_counts:
                lines.append(f"{source:<20}{self.anime_counts[source]:>8}")
        return "\n".join(lines)


def _allocate(total: int, sources: Sequence[str]) -> dict[str, int]:
    """Split `total` as evenly as possible across `sources`, remainder first.

    Returned in `sources` order so the allocation is reproducible rather than
    dependent on dict iteration of a set.
    """
    if not sources:
        return {}
    share, remainder = divmod(total, len(sources))
    return {
        source: share + (1 if position < remainder else 0)
        for position, source in enumerate(sources)
    }


def substitution_plan(
    source_labels: Sequence[str],
    replace_fraction: float,
    val_fraction: float = 0.2,
    seed: int = 0,
    drop_seed: int = 0,
) -> SubstitutionPlan:
    """Plan a volume-neutral swap of drawn training data for anime data.

    `seed` and `val_fraction` must match the values the baselines were fixed
    with — they determine the validation half, which this never modifies.
    `drop_seed` chooses *which* drawn training samples are swapped out and is
    independent of the split, so it can be varied without disturbing the
    holdouts.
    """
    if not 0.0 <= replace_fraction <= 1.0:
        raise ValueError(
            f"replace_fraction must be in [0, 1], got {replace_fraction}. It is the "
            "share of the drawn training half that anime data stands in for."
        )

    train_indices, val_indices = stratified_split(source_labels, val_fraction, seed)

    # Bucket the training half by source so only drawn classes are disturbed.
    buckets: dict[str, list[int]] = {}
    for index in train_indices:
        buckets.setdefault(source_labels[index], []).append(index)

    rng = random.Random(drop_seed)
    dropped: list[int] = []
    anime_counts: dict[str, int] = {}

    # sorted() so the two drawn classes are always consumed in the same order:
    # the rng stream is shared, and dict order would otherwise decide the draw.
    for original in sorted(REPLACES):
        members = buckets.get(original, [])[:]
        if not members:
            continue
        rng.shuffle(members)
        count = round(len(members) * replace_fraction)
        dropped.extend(members[:count])
        for source, allocated in _allocate(count, REPLACES[original]).items():
            anime_counts[source] = anime_counts.get(source, 0) + allocated

    dropped_set = set(dropped)
    return SubstitutionPlan(
        val_indices=val_indices,
        kept_train_indices=[i for i in train_indices if i not in dropped_set],
        dropped_indices=sorted(dropped),
        anime_counts={s: n for s, n in anime_counts.items() if n},
    )


def build_training_sets(
    archive_path: Path,
    supplement_path: Path | None,
    plan: SubstitutionPlan,
    image_size: int,
):
    """Compose the substituted training half and the untouched validation half.

    Returns `(train_set, val_set)`. The validation set is built from
    `plan.val_indices` alone, so it is the same set of samples the baselines
    were measured on regardless of what happened to the training half.
    """
    from torch.utils.data import ConcatDataset

    from holy_blocker_ml.dataset import ZipImageDataset

    original_train = ZipImageDataset(
        archive_path, image_size=image_size, augment=True, indices=plan.kept_train_indices
    )
    val_set = ZipImageDataset(
        archive_path, image_size=image_size, augment=False, indices=plan.val_indices
    )

    if supplement_path is None:
        return original_train, val_set

    supplement = ZipImageDataset(
        supplement_path,
        image_size=image_size,
        augment=True,
        policy=ANIME_LABEL_POLICY,
        classes=ANIME_SOURCE_CLASSES,
    )
    if len(supplement) != plan.anime_total:
        raise ValueError(
            f"supplement holds {len(supplement)} samples but the plan calls for "
            f"{plan.anime_total}; training on a different volume than planned would "
            "make the run non-volume-neutral and its comparison to the baselines invalid"
        )

    return ConcatDataset([original_train, supplement]), val_set


def select_members(names: Sequence[str], count: int, seed: int) -> list[str]:
    """Deterministically pick `count` distinct members from `names`.

    Sorted before shuffling because a remote zip's listing order is not stable
    across reads — sampling the arrival order would silently change the training
    set between two runs that claim the same seed.
    """
    pool = sorted(names)
    if count > len(pool):
        raise ValueError(f"asked for {count} members but only {len(pool)} are available")
    rng = random.Random(seed)
    rng.shuffle(pool)
    return pool[:count]


def source_seed(source: str, seed: int) -> int:
    """A per-source seed that is stable across processes.

    Derived from the source's fixed position in `ANIME_SOURCE_CLASSES` rather
    than `hash()`, which is salted per interpreter run and would reselect the
    supplement on every invocation while still reporting the same seed.
    """
    return seed * len(ANIME_SOURCE_CLASSES) + ANIME_SOURCE_CLASSES.index(source)


def open_remote_archive(filename: str) -> zipfile.ZipFile:
    """Open one rating archive on the Hub for random access, without downloading it.

    The archives total 68 GB, and the experiment's risk table calls for a
    stratified subset rather than materialising the corpus. Zip keeps its
    central directory at the end of the file, so `HfFileSystem`'s ranged reads
    let individual members be pulled without fetching the rest.
    """
    from huggingface_hub import HfFileSystem

    handle = HfFileSystem().open(f"datasets/{DATASET_REPO}/{filename}", "rb")
    return zipfile.ZipFile(handle)


def build_supplement(
    counts: dict[str, int],
    output_path: Path,
    open_archive: Callable[[str], zipfile.ZipFile] = open_remote_archive,
    seed: int = 0,
    progress: bool = False,
) -> Path:
    """Write a local zip holding `counts[source]` images per anime rating.

    Members are copied as bytes without decoding, and laid out as
    `<source>/<name>` so `ZipImageDataset` reads it with the same code path as
    the main corpus. `open_archive` is injected so this is testable against
    local stand-ins rather than the Hub.
    """
    output_path = Path(output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    unknown = sorted(set(counts) - set(RATING_ARCHIVES))
    if unknown:
        raise ValueError(
            f"unknown anime source(s): {', '.join(unknown)}; "
            f"expected one of: {', '.join(sorted(RATING_ARCHIVES))}"
        )

    with zipfile.ZipFile(output_path, "w", compression=zipfile.ZIP_STORED) as out:
        for source in sorted(counts):
            wanted = counts[source]
            if wanted <= 0:
                continue
            with open_archive(RATING_ARCHIVES[source]) as remote:
                names = [n for n in remote.namelist() if not n.endswith("/")]
                # Seed per source so changing one rating's count does not
                # reshuffle the others' selections. Offset comes from a fixed
                # position, not hash() — str hashing is salted per process, so
                # that would reselect the training set on every invocation
                # while still reporting the same seed.
                chosen = select_members(names, wanted, seed=source_seed(source, seed))
                for position, name in enumerate(chosen, start=1):
                    out.writestr(f"{source}/{Path(name).name}", remote.read(name))
                    if progress and position % 250 == 0:
                        print(f"  {source}: {position}/{wanted}", flush=True)
            if progress:
                print(f"  {source}: {wanted}/{wanted} done", flush=True)

    return output_path


def main() -> None:
    """CLI: build the anime training supplement, or the questionable boundary set."""
    import argparse

    from holy_blocker_ml.config import TrainingConfig
    from holy_blocker_ml.dataset import ZipImageDataset

    parser = argparse.ArgumentParser(
        description="Subsample deepghs/anime_dbrating into a drawn training supplement."
    )
    parser.add_argument("--archive", type=Path, required=True, help="path to the corpus zip")
    parser.add_argument("--out", type=Path, default=Path("data/eval/anime_supplement.zip"))
    parser.add_argument(
        "--replace-fraction",
        type=float,
        default=0.5,
        help="share of the drawn training half to swap for anime data",
    )
    parser.add_argument("--drop-seed", type=int, default=0)
    parser.add_argument("--seed", type=int, default=0, help="seed for member selection")
    parser.add_argument(
        "--questionable",
        type=int,
        default=0,
        help="instead build a boundary evaluation set of this many `questionable` "
        "images; these are never trained on",
    )
    parser.add_argument("--image-size", type=int, default=TrainingConfig.image_size)
    parser.add_argument("--dry-run", action="store_true", help="print the plan and exit")
    args = parser.parse_args()

    if args.questionable:
        counts = {QUESTIONABLE: args.questionable}
        print(f"boundary set: {args.questionable} `questionable` images (never trained on)")
    else:
        index = ZipImageDataset(args.archive, image_size=args.image_size, augment=False)
        plan = substitution_plan(
            index.source_labels,
            replace_fraction=args.replace_fraction,
            drop_seed=args.drop_seed,
        )
        print(plan.describe())
        counts = plan.anime_counts

    if args.dry_run:
        return

    print(f"\nfetching from {DATASET_REPO} (ranged reads; the corpus is never downloaded whole)")
    build_supplement(counts, args.out, seed=args.seed, progress=True)
    print(f"\nwrote {args.out} ({sum(counts.values())} images)")
