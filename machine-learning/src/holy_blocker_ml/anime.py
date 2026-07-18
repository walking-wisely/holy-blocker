"""Subsample `deepghs/anime_dbrating` into a drawn-medium training supplement.

Implements the [anime subsampling experiment](../../../docs/components/machine-learning/experiments/anime-subsampling.md):
does drawn training data from a better-labelled source close the drawn/photographic
gap, without degrading photographic performance?

## The ratio rule, and what replaced it

Method step 1 said to subsample "so the combined set holds that ratio" (40%
drawn / 60% photographic). The corpus is *already* exactly 40% drawn — 11,200
of 28,000 — so the rule as written is satisfied only by adding nothing.

Its purpose, though, is stated in the risk table: "volume imbalance swamps
photographic content." That is a guard against the naive 40:1 mix, not a
conservation law. So the bound adopted here is **drawn may grow but must not
exceed photographic**, which `DEFAULT_ANIME_COUNT` sets at exactly 50%.

An earlier revision of this module held drawn volume fixed and *substituted*
anime data for original drawn data. That was withdrawn: it confounds label
quality with domain shift plus a cut in in-distribution data, and since the
validation half is original `drawings`/`hentai`, it biases the run toward a
decline — answering a different question than the one pre-registered.

## Three arms

Addition alone cannot separate "better labels helped" from "more data helped",
so the experiment runs an ablation control beside it (see `MixturePlan`). The
two are symmetric around the baseline — `DEFAULT_ANIME_COUNT` added, the same
number removed — so the ablation measures how sensitive drawn AUC is to drawn
volume at all, which is the scale the addition arm's delta has to be read
against.

## What must not move

Baselines are fixed on `stratified_split(seed=0, val_fraction=0.2)` over the
original archive. So:

- **The validation half is never touched** in any arm, so `val_indices` comes
  back identical to the unmodified split and the frozen holdouts stay
  comparable.
- **Photographic training data is never touched.** The decision rule rejects on
  a photographic regression, which is only interpretable if that half is held
  fixed.
- **The anime budget stays balanced across safe and explicit**, so an arm does
  not move the class prior as well as the data.
"""

import random
import zipfile
from collections.abc import Callable, Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path

from holy_blocker_ml.dataset import IMAGE_SUFFIXES
from holy_blocker_ml.finetune import stratified_split
from holy_blocker_ml.labels import EXPLICIT, SAFE

#: Danbooru's four ordinal ratings, prefixed so they cannot collide with the
#: five `nsfw_detect` classes in a shared source-label column.
GENERAL = "anime_general"
SENSITIVE = "anime_sensitive"
QUESTIONABLE = "anime_questionable"
ANIME_EXPLICIT = "anime_explicit"

#: Ratings that map onto safe drawn content, i.e. onto `drawings`.
#:
#: `questionable` is here rather than held out, reversing the original method.
#: Two reasons, both about matching what `drawings` actually contains:
#:
#: - `drawings` is a *residual* class — every drawn image that is not `hentai`.
#:   On the Danbooru scale that spans general, sensitive, and much of
#:   questionable. Excluding questionable from the anime safe class while the
#:   validation set keeps such content inside `drawings` truncates the safe
#:   class's borderline support on one side only, which drives false positives
#:   up on exactly the images the experiment cares about — a drawn AUC drop by
#:   construction, unrelated to label quality.
#: - It is the mapping consistent with the established policy, which already
#:   puts `sexy` (photographic suggestive) on the safe side.
#:
#: The original rationale for holding it out — that assigning it a side
#: re-imports arbitrariness — mistakes the level: the arbitrariness being fixed
#: is per-image, and `drawings` already commits this content to the safe side.
ANIME_SAFE_SOURCES: tuple[str, ...] = (GENERAL, SENSITIVE, QUESTIONABLE)
#: Ratings that map onto explicit drawn content, i.e. onto `hentai`.
ANIME_EXPLICIT_SOURCES: tuple[str, ...] = (ANIME_EXPLICIT,)
#: Everything that may enter training.
ANIME_DRAWN_SOURCES: tuple[str, ...] = (*ANIME_SAFE_SOURCES, *ANIME_EXPLICIT_SOURCES)

#: Every anime source class. Order is load-bearing: `source_seed` indexes into
#: it, so reordering silently reselects every supplement.
ANIME_SOURCE_CLASSES: tuple[str, ...] = (GENERAL, SENSITIVE, QUESTIONABLE, ANIME_EXPLICIT)

#: Rating -> binary policy.
ANIME_LABEL_POLICY: dict[str, str] = {
    GENERAL: SAFE,
    SENSITIVE: SAFE,
    QUESTIONABLE: SAFE,
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

#: Anime images added in the addition arm, and removed in the ablation arm, so
#: the two are symmetric around the baseline and their deltas share a scale.
#:
#: 4,480 takes the drawn share of the training half from 40% to exactly 50%.
#: That is the bound this experiment adopts in place of method step 1's
#: unsatisfiable "hold 40%": drawn may grow but must not exceed photographic,
#: which is what step 1's risk-table rationale ("volume imbalance swamps
#: photographic content") actually asks for.
DEFAULT_ANIME_COUNT = 4480

#: The drawn source each group of anime ratings stands alongside. Keeping the
#: safe/explicit balance of the drawn half intact means a shift in the decision
#: threshold cannot be mistaken for a modelling gain.
MIRRORS: dict[str, tuple[str, ...]] = {
    "drawings": ANIME_SAFE_SOURCES,
    "hentai": ANIME_EXPLICIT_SOURCES,
}

#: The drawn classes in the source corpus. Local alias so this module does not
#: depend on `medium.DRAWN`, which names the *scored* holdout and must not drift.
DRAWN_SOURCES: tuple[str, ...] = ("drawings", "hentai")


@dataclass(frozen=True)
class MixturePlan:
    """Which original samples to drop, and how many anime samples to add.

    Three arms are expressible, and the experiment runs all three:

    - `drop_fraction=0, anime_count=0` — the baseline, i.e. the existing
      full-unfreeze run.
    - `drop_fraction=0, anime_count=N` — **addition**, the pre-registered
      question: does more/better drawn data help?
    - `drop_fraction=f, anime_count=0` — **ablation**, the control: how much
      does drawn AUC depend on drawn training volume at all? Without this the
      addition arm's magnitude has no scale to be read against.
    """

    val_indices: list[int]
    kept_train_indices: list[int]
    dropped_indices: list[int]
    anime_counts: dict[str, int]

    @property
    def anime_total(self) -> int:
        return sum(self.anime_counts.values())

    def describe(self, source_labels: Sequence[str] | None = None) -> str:
        total = len(self.kept_train_indices) + self.anime_total
        lines = [
            f"validation half (frozen): {len(self.val_indices)}",
            f"training half:            {len(self.kept_train_indices)} kept "
            f"+ {self.anime_total} anime = {total}",
            f"dropped from training:    {len(self.dropped_indices)}",
        ]
        if source_labels is not None:
            # The drawn share is the quantity method step 1 constrains, so it is
            # reported rather than left to be inferred from the counts.
            drawn = sum(
                1 for i in self.kept_train_indices if source_labels[i] in DRAWN_SOURCES
            ) + self.anime_total
            lines.append(f"drawn share of training:  {drawn}/{total} = {drawn / total:.1%}")
        if self.anime_counts:
            lines += ["", f"{'anime source':<20}{'n':>8}"]
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


def mixture_plan(
    source_labels: Sequence[str],
    drop_fraction: float = 0.0,
    anime_count: int = 0,
    val_fraction: float = 0.2,
    seed: int = 0,
    drop_seed: int = 0,
) -> MixturePlan:
    """Plan one arm: drop a share of the drawn training half, add anime data, or both.

    `seed` and `val_fraction` must match the values the baselines were fixed
    with — they determine the validation half, which this never modifies.
    `drop_seed` chooses *which* drawn training samples are dropped and is
    independent of the split, so it can be varied without disturbing the
    holdouts.

    `anime_count` is split evenly between safe and explicit, mirroring the
    50/50 safe/explicit balance of the drawn half, so the arm does not also
    move the class prior.
    """
    if not 0.0 <= drop_fraction <= 1.0:
        raise ValueError(
            f"drop_fraction must be in [0, 1], got {drop_fraction}. It is the share "
            "of the drawn training half to remove."
        )
    if anime_count < 0:
        raise ValueError(f"anime_count must be non-negative, got {anime_count}")

    train_indices, val_indices = stratified_split(source_labels, val_fraction, seed)

    # Bucket the training half by source so only drawn classes are disturbed.
    buckets: dict[str, list[int]] = {}
    for index in train_indices:
        buckets.setdefault(source_labels[index], []).append(index)

    rng = random.Random(drop_seed)
    dropped: list[int] = []
    # sorted() so the two drawn classes are always consumed in the same order:
    # the rng stream is shared, and dict order would otherwise decide the draw.
    for original in sorted(DRAWN_SOURCES):
        members = buckets.get(original, [])[:]
        if not members:
            continue
        rng.shuffle(members)
        dropped.extend(members[: round(len(members) * drop_fraction)])

    # Half the anime budget mirrors `drawings`, half mirrors `hentai`.
    safe_budget = anime_count // 2
    anime_counts: dict[str, int] = {}
    for source, allocated in _allocate(safe_budget, MIRRORS["drawings"]).items():
        anime_counts[source] = anime_counts.get(source, 0) + allocated
    for source, allocated in _allocate(anime_count - safe_budget, MIRRORS["hentai"]).items():
        anime_counts[source] = anime_counts.get(source, 0) + allocated

    dropped_set = set(dropped)
    return MixturePlan(
        val_indices=val_indices,
        kept_train_indices=[i for i in train_indices if i not in dropped_set],
        dropped_indices=sorted(dropped),
        anime_counts={s: n for s, n in anime_counts.items() if n},
    )


def build_training_sets(
    archive_path: Path,
    supplement_path: Path | None,
    plan: MixturePlan,
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
            f"{plan.anime_total}; the arm would train on a different volume than the "
            "one recorded, so its delta could not be attributed"
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

    return zipfile.ZipFile(open_remote_stream(filename))


def open_remote_stream(filename: str):
    """Open one rating archive as a raw seekable byte stream.

    `cache_type="none"` because the access pattern is thousands of small reads
    scattered across an 18 GB file. fsspec's default read-ahead would fetch a
    block around each one and discard nearly all of it.

    The fsspec instance cache is deliberately left on: it shares one httpx client
    across streams, which is why `_fetch_source` must not close them mid-build.
    """
    from huggingface_hub import HfFileSystem

    return HfFileSystem().open(
        f"datasets/{DATASET_REPO}/{filename}", "rb", cache_type="none"
    )


#: Concurrent member fetches. Each selected image is one ranged HTTP request, so
#: a sequential build runs at ~0.2 images/s — about seven hours for a full
#: supplement. The work is entirely latency-bound, so it parallelises almost
#: linearly; 32 is well inside what the Hub tolerates for ranged reads.
DEFAULT_FETCH_WORKERS = 32

#: Members held in memory at once. `ThreadPoolExecutor.map` would submit every
#: task upfront and buffer every result, which at ~200 KB an image is close to a
#: gigabyte of resident bytes. Batching bounds it.
FETCH_BATCH = 256


# Local file header layout, PKWARE APPNOTE.TXT §4.3.7. Field offsets are from
# the start of the header; the member's data begins after the variable-length
# name and extra fields, whose lengths live in the header rather than in the
# central directory (they are permitted to differ between the two).
LOCAL_HEADER_SIGNATURE = b"PK\x03\x04"  # APPNOTE §4.3.7, "local file header signature"
LOCAL_HEADER_FIXED_SIZE = 30  # APPNOTE §4.3.7, through "extra field length"
LOCAL_NAME_LENGTH_OFFSET = 26  # APPNOTE §4.3.7, "file name length"
LOCAL_EXTRA_LENGTH_OFFSET = 28  # APPNOTE §4.3.7, "extra field length"

#: Slack read past the fixed header so the name and extra fields almost always
#: arrive in the same request as the data. Extra fields are a few dozen bytes in
#: practice; a miss costs one extra range request, not a wrong result.
LOCAL_HEADER_SLACK = 4096

#: APPNOTE §4.4.5. Only these two appear in the rating archives — webp is
#: already compressed, so members are typically stored.
COMPRESSION_STORED = 0
COMPRESSION_DEFLATED = 8


@dataclass(frozen=True)
class _Member:
    """Where one member's bytes live, taken from the central directory."""

    name: str
    header_offset: int
    compress_size: int
    compress_type: int


def index_members(
    archive: zipfile.ZipFile,
    names: Sequence[str],
) -> list[_Member]:
    """Resolve `names` to byte offsets using an already-parsed central directory."""
    entries = {info.filename: info for info in archive.infolist()}
    missing = [name for name in names if name not in entries]
    if missing:
        raise ValueError(f"members absent from the archive index: {missing[:5]}")
    return [
        _Member(
            name=name,
            header_offset=entries[name].header_offset,
            compress_size=entries[name].compress_size,
            compress_type=entries[name].compress_type,
        )
        for name in names
    ]


def read_member(handle, member: _Member) -> bytes:
    """Read and decompress one member through raw ranged reads.

    Speculatively fetches the fixed header plus slack plus the compressed size in
    a single request, then re-reads only if the name and extra fields overflowed
    the slack.
    """
    import struct
    import zlib

    handle.seek(member.header_offset)
    block = handle.read(LOCAL_HEADER_FIXED_SIZE + LOCAL_HEADER_SLACK + member.compress_size)
    if not block.startswith(LOCAL_HEADER_SIGNATURE):
        raise ValueError(
            f"{member.name}: no local file header at offset {member.header_offset}; "
            "the archive index and the data disagree"
        )

    (name_length,) = struct.unpack_from("<H", block, LOCAL_NAME_LENGTH_OFFSET)
    (extra_length,) = struct.unpack_from("<H", block, LOCAL_EXTRA_LENGTH_OFFSET)
    start = LOCAL_HEADER_FIXED_SIZE + name_length + extra_length
    end = start + member.compress_size

    if end > len(block):
        # Slack was too small for this member's extra field.
        handle.seek(member.header_offset + start)
        payload = handle.read(member.compress_size)
    else:
        payload = block[start:end]

    if member.compress_type == COMPRESSION_STORED:
        return payload
    if member.compress_type == COMPRESSION_DEFLATED:
        # Negative window size selects a raw deflate stream with no zlib header,
        # which is what a zip member holds (APPNOTE §4.4.5, method 8).
        return zlib.decompressobj(-zlib.MAX_WBITS).decompress(payload)
    raise ValueError(f"{member.name}: unsupported compression method {member.compress_type}")


def _fetch_source(
    members: Sequence[_Member],
    open_raw: Callable[[], object],
    workers: int,
    keepalive: list,
    batch_size: int = FETCH_BATCH,
) -> Iterator[tuple[str, bytes]]:
    """Yield `(name, bytes)` for one rating, in order, fetched concurrently.

    Workers hold *raw* byte streams, not `ZipFile`s. Opening a `ZipFile` downloads
    and parses the archive's central directory — ~21 MB across ~341k members — so
    giving every worker its own cost 21 MB x workers x ratings, about 2.7 GB at 32
    workers against the 0.23 GB of images actually wanted. Raising the worker count
    multiplied that overhead rather than the throughput, and at 96 workers the Hub
    began closing connections mid-body.

    The directory is now parsed once per rating by the caller and passed in as
    `members`; workers only issue ranged reads for the byte spans it names.

    Each worker still needs its own stream, because seeking is stateful and threads
    sharing one would interleave seeks — the hazard `ZipImageDataset` handles for
    forked DataLoader workers. Streams go into `keepalive` and are not closed here:
    they share one httpx client, so closing or garbage-collecting one closes it for
    every later rating, which once truncated a build to 2,783 of 4,480 images while
    still exiting cleanly.
    """
    import threading
    from concurrent.futures import ThreadPoolExecutor

    local = threading.local()
    lock = threading.Lock()

    def read(member: _Member) -> tuple[str, bytes]:
        handle = getattr(local, "handle", None)
        if handle is None:
            handle = local.handle = open_raw()
            with lock:
                keepalive.append(handle)
        return member.name, read_member(handle, member)

    with ThreadPoolExecutor(max_workers=workers) as pool:
        for start in range(0, len(members), batch_size):
            yield from pool.map(read, members[start : start + batch_size])


def build_supplement(
    counts: dict[str, int],
    output_path: Path,
    open_archive: Callable[[str], zipfile.ZipFile] = open_remote_archive,
    seed: int = 0,
    progress: bool = False,
    workers: int = DEFAULT_FETCH_WORKERS,
    open_stream: Callable[[str], object] = open_remote_stream,
) -> Path:
    """Write a local zip holding `counts[source]` images per anime rating.

    Members are laid out as `<source>/<name>` so `ZipImageDataset` reads the
    result through the same code path as the main corpus.

    Each rating's central directory is parsed exactly once, via `open_archive`;
    the selected members are then pulled by `open_stream` as raw ranged reads.
    Both are injected so this is testable against local stand-ins rather than
    the Hub.
    """
    output_path = Path(output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    unknown = sorted(set(counts) - set(RATING_ARCHIVES))
    if unknown:
        raise ValueError(
            f"unknown anime source(s): {', '.join(unknown)}; "
            f"expected one of: {', '.join(sorted(RATING_ARCHIVES))}"
        )

    # Holds every worker handle open for the whole build; see `_fetch_source`.
    keepalive: list[zipfile.ZipFile] = []
    with zipfile.ZipFile(output_path, "w", compression=zipfile.ZIP_STORED) as out:
        for source in sorted(counts):
            wanted = counts[source]
            if wanted <= 0:
                continue
            with open_archive(RATING_ARCHIVES[source]) as remote:
                # Filter to decodable images. The rating archives carry
                # non-image members, and `.gif` is not in IMAGE_SUFFIXES —
                # selecting either produces a supplement that loads short and
                # aborts the run at the volume check, after the whole fetch.
                names = [
                    n
                    for n in remote.namelist()
                    if not n.endswith("/") and Path(n).suffix.lower() in IMAGE_SUFFIXES
                ]
                if len(names) < wanted:
                    raise ValueError(
                        f"{RATING_ARCHIVES[source]} holds {len(names)} decodable images "
                        f"but {wanted} were requested"
                    )
                # Seed per source so changing one rating's count does not
                # reshuffle the others' selections. Offset comes from a fixed
                # position, not hash() — str hashing is salted per process, so
                # that would reselect the training set on every invocation
                # while still reporting the same seed.
                chosen = select_members(names, wanted, seed=source_seed(source, seed))
                # Resolve offsets while the directory is still parsed. This is
                # the only time it is read; workers never open a ZipFile.
                members = index_members(remote, chosen)

            done = 0
            for name, payload in _fetch_source(
                members,
                lambda source=source: open_stream(RATING_ARCHIVES[source]),
                workers,
                keepalive,
            ):
                out.writestr(f"{source}/{Path(name).name}", payload)
                done += 1
                if progress and done % FETCH_BATCH == 0:
                    print(f"  {source}: {done}/{wanted}", flush=True)

            # A short rating means the fetch died partway. Without this the
            # build writes a truncated zip and exits cleanly, and the shortfall
            # only surfaces much later as a volume-check failure at training
            # start — or not at all, if the arm is scored without checking.
            if done != wanted:
                raise RuntimeError(
                    f"{source}: wrote {done} of {wanted} images; the supplement is "
                    "incomplete and would train a different mixture than planned"
                )
            if progress:
                print(f"  {source}: {done}/{wanted} done", flush=True)

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
        "--anime-count",
        type=int,
        default=DEFAULT_ANIME_COUNT,
        help="anime images to add to the drawn training half",
    )
    parser.add_argument(
        "--drop-fraction",
        type=float,
        default=0.0,
        help="share of the drawn training half to remove (the ablation control arm)",
    )
    parser.add_argument("--drop-seed", type=int, default=0)
    parser.add_argument("--seed", type=int, default=0, help="seed for member selection")
    parser.add_argument("--image-size", type=int, default=TrainingConfig.image_size)
    parser.add_argument(
        "--workers",
        type=int,
        default=DEFAULT_FETCH_WORKERS,
        help="concurrent member fetches; the build is latency-bound, not CPU-bound",
    )
    parser.add_argument("--dry-run", action="store_true", help="print the plan and exit")
    args = parser.parse_args()

    index = ZipImageDataset(args.archive, image_size=args.image_size, augment=False)
    plan = mixture_plan(
        index.source_labels,
        drop_fraction=args.drop_fraction,
        anime_count=args.anime_count,
        drop_seed=args.drop_seed,
    )
    print(plan.describe(index.source_labels))
    counts = plan.anime_counts

    if not counts:
        print("\nno anime images requested; this arm needs no supplement")
        return

    if args.dry_run:
        return

    print(f"\nfetching from {DATASET_REPO} (ranged reads; the corpus is never downloaded whole)")
    build_supplement(counts, args.out, seed=args.seed, progress=True, workers=args.workers)
    print(f"\nwrote {args.out} ({sum(counts.values())} images)")
