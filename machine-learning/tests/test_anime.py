"""Tests for the anime-subsampling mixture plan.

The [experiment](../../docs/components/machine-learning/experiments/anime-subsampling.md)
is judged against baselines fixed on `stratified_split(seed=0, val_fraction=0.2)`
over the original archive, across three arms: a baseline, an **addition** arm
that adds anime data, and an **ablation** arm that removes drawn data and adds
nothing. Everything here exists to keep those comparable:

- the validation half must come back **byte-identical** to the unmodified split
  in every arm, or a run is scored against samples the baselines never covered;
- photographic training data must never move, because the decision rule rejects
  on a photographic regression and that is only readable if the half is fixed;
- the anime budget must stay balanced across safe and explicit, or an arm moves
  the class prior as well as the data and the delta cannot be attributed.

A defect in any of these changes the verdict silently rather than failing, which
is the failure mode pre-registration exists to prevent.
"""

import io
import zipfile

import pytest
from PIL import Image

from holy_blocker_ml.anime import (
    ANIME_DRAWN_SOURCES,
    ANIME_EXPLICIT_SOURCES,
    ANIME_LABEL_POLICY,
    ANIME_SAFE_SOURCES,
    ANIME_SOURCE_CLASSES,
    DEFAULT_ANIME_COUNT,
    QUESTIONABLE,
    RATING_ARCHIVES,
    build_supplement,
    mixture_plan,
    select_members,
)
from holy_blocker_ml.finetune import stratified_split
from holy_blocker_ml.labels import EXPLICIT, SAFE
from holy_blocker_ml.medium import medium_of

#: The real archive: five classes, 5,600 each.
CORPUS = (
    ["drawings"] * 5600
    + ["hentai"] * 5600
    + ["neutral"] * 5600
    + ["porn"] * 5600
    + ["sexy"] * 5600
)

#: Drawn samples in the training half: 11,200 drawn x 0.8.
DRAWN_TRAIN = 8960


def split_of(labels=CORPUS):
    return stratified_split(labels, val_fraction=0.2, seed=0)


@pytest.fixture
def addition():
    """The arm that answers the pre-registered question."""
    return mixture_plan(CORPUS, drop_fraction=0.0, anime_count=DEFAULT_ANIME_COUNT)


@pytest.fixture
def ablation():
    """The control arm: remove drawn data, add nothing."""
    return mixture_plan(CORPUS, drop_fraction=0.5, anime_count=0)


@pytest.fixture
def baseline():
    return mixture_plan(CORPUS)


class TestRatingMapping:
    def test_general_and_sensitive_are_safe(self):
        assert ANIME_LABEL_POLICY["anime_general"] == SAFE
        assert ANIME_LABEL_POLICY["anime_sensitive"] == SAFE

    def test_explicit_is_explicit(self):
        assert ANIME_LABEL_POLICY["anime_explicit"] == EXPLICIT

    def test_questionable_is_safe(self):
        # `drawings` is a residual class covering every drawn image that is not
        # `hentai`, which on the Danbooru scale includes much of questionable.
        # Holding it out would truncate the safe class's borderline support on
        # the training side only, moving drawn AUC for reasons unrelated to
        # label quality. Safe is also the side `sexy` already sits on.
        assert ANIME_LABEL_POLICY[QUESTIONABLE] == SAFE

    def test_every_anime_source_has_a_training_label(self):
        for source in ANIME_DRAWN_SOURCES:
            assert source in ANIME_LABEL_POLICY

    def test_every_anime_source_is_drawn(self):
        for source in ANIME_SOURCE_CLASSES:
            assert medium_of(source) == "drawn"

    def test_a_rating_archive_is_known_for_every_rating(self):
        for source in ANIME_SOURCE_CLASSES:
            assert source in RATING_ARCHIVES

    def test_the_source_order_is_frozen(self):
        # `source_seed` indexes into this tuple, so reordering it silently
        # reselects every supplement while still reporting the same seed.
        assert ANIME_SOURCE_CLASSES == (
            "anime_general",
            "anime_sensitive",
            "anime_questionable",
            "anime_explicit",
        )


class TestValidationHalfIsUntouched:
    """The invariant every arm depends on."""

    @pytest.mark.parametrize("arm", ["addition", "ablation", "baseline"])
    def test_validation_indices_match_the_unmodified_split(self, arm, request):
        plan = request.getfixturevalue(arm)
        _, expected_val = split_of()
        assert plan.val_indices == expected_val

    @pytest.mark.parametrize("arm", ["addition", "ablation", "baseline"])
    def test_no_validation_index_leaks_into_training(self, arm, request):
        plan = request.getfixturevalue(arm)
        assert not set(plan.kept_train_indices) & set(plan.val_indices)

    @pytest.mark.parametrize("arm", ["addition", "ablation", "baseline"])
    def test_photographic_training_data_never_moves(self, arm, request):
        plan = request.getfixturevalue(arm)
        train, _ = split_of()
        expected = sorted(i for i in train if medium_of(CORPUS[i]) == "photographic")
        actual = sorted(
            i for i in plan.kept_train_indices if medium_of(CORPUS[i]) == "photographic"
        )
        assert actual == expected


class TestAdditionArm:
    def test_nothing_is_dropped(self, addition):
        assert addition.dropped_indices == []

    def test_the_whole_original_training_half_is_kept(self, addition):
        train, _ = split_of()
        assert addition.kept_train_indices == train

    def test_the_training_half_grows_by_the_anime_budget(self, addition):
        train, _ = split_of()
        total = len(addition.kept_train_indices) + addition.anime_total
        assert total == len(train) + DEFAULT_ANIME_COUNT

    def test_the_default_budget_takes_drawn_to_exactly_half(self, addition):
        # The bound this experiment adopts in place of the unsatisfiable
        # "hold 40%": drawn may grow but must not exceed photographic.
        drawn = DRAWN_TRAIN + addition.anime_total
        total = len(addition.kept_train_indices) + addition.anime_total
        assert drawn / total == 0.5

    def test_drawn_never_exceeds_photographic_at_the_default_budget(self, addition):
        train, _ = split_of()
        photographic = sum(1 for i in train if medium_of(CORPUS[i]) == "photographic")
        assert DRAWN_TRAIN + addition.anime_total <= photographic


class TestAblationArm:
    def test_it_adds_no_anime_data(self, ablation):
        assert ablation.anime_counts == {}
        assert ablation.anime_total == 0

    def test_it_removes_half_the_drawn_training_half(self, ablation):
        assert len(ablation.dropped_indices) == DRAWN_TRAIN // 2

    def test_only_drawn_samples_are_dropped(self, ablation):
        assert {medium_of(CORPUS[i]) for i in ablation.dropped_indices} == {"drawn"}

    def test_dropped_samples_all_come_from_the_training_half(self, ablation):
        train, _ = split_of()
        assert set(ablation.dropped_indices) <= set(train)

    def test_equal_numbers_are_dropped_from_each_drawn_class(self, ablation):
        dropped = [CORPUS[i] for i in ablation.dropped_indices]
        assert dropped.count("drawings") == dropped.count("hentai") == 2240

    def test_its_removal_matches_the_addition_arm_s_budget(self, ablation):
        # The two arms are symmetric around the baseline so their deltas share
        # a scale; that is what lets the addition arm's magnitude be read.
        assert len(ablation.dropped_indices) == DEFAULT_ANIME_COUNT


class TestBaselineArm:
    def test_it_is_the_plain_stratified_split(self, baseline):
        train, val = split_of()
        assert baseline.kept_train_indices == train
        assert baseline.val_indices == val
        assert baseline.dropped_indices == []
        assert baseline.anime_total == 0


class TestAnimeAllocation:
    def test_the_budget_splits_evenly_between_safe_and_explicit(self, addition):
        safe = sum(addition.anime_counts.get(s, 0) for s in ANIME_SAFE_SOURCES)
        explicit = sum(addition.anime_counts.get(s, 0) for s in ANIME_EXPLICIT_SOURCES)
        assert safe == explicit == DEFAULT_ANIME_COUNT // 2

    def test_the_safe_budget_spreads_across_all_three_safe_ratings(self, addition):
        allocated = [addition.anime_counts.get(s, 0) for s in ANIME_SAFE_SOURCES]
        assert all(count > 0 for count in allocated)
        assert max(allocated) - min(allocated) <= 1

    def test_questionable_receives_training_samples(self, addition):
        assert addition.anime_counts.get(QUESTIONABLE, 0) > 0

    def test_the_allocation_sums_to_the_requested_budget(self, addition):
        assert addition.anime_total == DEFAULT_ANIME_COUNT

    @pytest.mark.parametrize("budget", [1, 2, 3, 7, 99, 4481])
    def test_an_odd_budget_is_allocated_without_loss(self, budget):
        plan = mixture_plan(CORPUS, anime_count=budget)
        assert plan.anime_total == budget

    def test_a_negative_budget_is_rejected(self):
        with pytest.raises(ValueError, match="non-negative"):
            mixture_plan(CORPUS, anime_count=-1)


class TestDropFraction:
    def test_a_full_fraction_removes_every_drawn_training_sample(self):
        full = mixture_plan(CORPUS, drop_fraction=1.0)
        kept_drawn = [i for i in full.kept_train_indices if medium_of(CORPUS[i]) == "drawn"]
        assert kept_drawn == []

    @pytest.mark.parametrize("fraction", [-0.1, 1.1])
    def test_a_fraction_outside_the_unit_interval_is_rejected(self, fraction):
        with pytest.raises(ValueError, match=r"\[0, 1\]"):
            mixture_plan(CORPUS, drop_fraction=fraction)


class TestSelectMembers:
    NAMES = [f"danbooru_{i}.webp" for i in range(1000)]

    def test_it_returns_the_requested_count(self):
        assert len(select_members(self.NAMES, 50, seed=0)) == 50

    def test_the_same_seed_selects_the_same_members(self):
        assert select_members(self.NAMES, 50, seed=7) == select_members(self.NAMES, 50, seed=7)

    def test_a_different_seed_selects_different_members(self):
        assert select_members(self.NAMES, 50, seed=1) != select_members(self.NAMES, 50, seed=2)

    def test_selection_does_not_depend_on_the_order_names_arrive_in(self):
        # Remote zip listings are not order-stable across reads; a selection
        # that depended on arrival order would silently change the training set
        # between runs that claim the same seed.
        shuffled = list(reversed(self.NAMES))
        assert select_members(self.NAMES, 50, seed=3) == select_members(shuffled, 50, seed=3)

    def test_members_are_not_repeated(self):
        chosen = select_members(self.NAMES, 200, seed=0)
        assert len(set(chosen)) == len(chosen)

    def test_asking_for_more_than_exist_is_rejected(self):
        with pytest.raises(ValueError, match="only 1000"):
            select_members(self.NAMES, 5000, seed=0)


def make_rating_zip(path, count, extras=()):
    """A stand-in for one remote rating archive, laid out flat like the real one."""
    with zipfile.ZipFile(path, "w") as archive:
        for i in range(count):
            buffer = io.BytesIO()
            Image.new("RGB", (8, 8), (i % 256, 0, 0)).save(buffer, format="WEBP")
            archive.writestr(f"danbooru_{i}.webp", buffer.getvalue())
        for name in extras:
            archive.writestr(name, b"not an image")


class LocalHub:
    """Stand-in for the Hub: an index opener and a raw-stream opener."""

    def __init__(self, root):
        self.root = root

    def __call__(self, filename):  # open_archive
        return zipfile.ZipFile(self.root / filename)

    def stream(self, filename):  # open_stream
        return open(self.root / filename, "rb")


@pytest.fixture
def sources(tmp_path):
    """Local stand-ins for the four remote rating archives."""
    root = tmp_path / "remote"
    root.mkdir()
    for filename in RATING_ARCHIVES.values():
        make_rating_zip(root / filename, count=100)
    return LocalHub(root)


@pytest.fixture
def noisy_sources(tmp_path):
    """Rating archives that also carry metadata and a gif, as the real ones do."""
    root = tmp_path / "noisy"
    root.mkdir()
    for filename in RATING_ARCHIVES.values():
        make_rating_zip(
            root / filename,
            count=20,
            extras=("meta.json", "readme.txt", "danbooru_anim.gif"),
        )
    return LocalHub(root)


class TestRawRangeReads:
    """The hand-rolled ZIP reader must agree with zipfile, byte for byte.

    Workers bypass `zipfile` and parse local file headers themselves, because
    opening a ZipFile downloads a ~21 MB central directory per worker. That
    trades a bandwidth problem for a correctness risk, so the two paths are
    compared directly.
    """

    @pytest.fixture
    def archive(self, tmp_path):
        path = tmp_path / "mixed.zip"
        # Both compression methods, and a name long enough to push the data
        # past the fixed 30-byte header.
        with zipfile.ZipFile(path, "w") as out:
            for i in range(12):
                buffer = io.BytesIO()
                Image.new("RGB", (16, 16), (i * 7 % 256, i, 0)).save(buffer, format="WEBP")
                out.writestr(
                    f"danbooru_{'x' * i}_{i}.webp",
                    buffer.getvalue(),
                    compress_type=zipfile.ZIP_STORED if i % 2 else zipfile.ZIP_DEFLATED,
                )
        return path

    def test_it_matches_zipfile_for_every_member(self, archive):
        from holy_blocker_ml.anime import index_members, read_member

        with zipfile.ZipFile(archive) as source:
            names = source.namelist()
            expected = {n: source.read(n) for n in names}
            members = index_members(source, names)

        with open(archive, "rb") as raw:
            for member in members:
                assert read_member(raw, member) == expected[member.name], member.name

    def test_it_handles_both_compression_methods(self, archive):
        from holy_blocker_ml.anime import index_members

        with zipfile.ZipFile(archive) as source:
            members = index_members(source, source.namelist())
        assert {m.compress_type for m in members} == {0, 8}

    def test_a_member_absent_from_the_index_is_rejected(self, archive):
        from holy_blocker_ml.anime import index_members

        with zipfile.ZipFile(archive) as source:
            with pytest.raises(ValueError, match="absent from the archive index"):
                index_members(source, ["nope.webp"])

    def test_a_wrong_offset_is_caught_rather_than_returning_garbage(self, archive):
        from holy_blocker_ml.anime import _Member, read_member

        bogus = _Member(name="x.webp", header_offset=7, compress_size=10, compress_type=0)
        with open(archive, "rb") as raw:
            with pytest.raises(ValueError, match="no local file header"):
                read_member(raw, bogus)

    def test_it_survives_an_extra_field_larger_than_the_slack(self, tmp_path, monkeypatch):
        # The single-request read is speculative; if the name and extra fields
        # overflow the slack it must re-read rather than truncate the payload.
        from holy_blocker_ml import anime
        from holy_blocker_ml.anime import index_members, read_member

        path = tmp_path / "slack.zip"
        with zipfile.ZipFile(path, "w") as out:
            out.writestr("danbooru_1.webp", b"payload-bytes" * 64)

        monkeypatch.setattr(anime, "LOCAL_HEADER_SLACK", 0)
        with zipfile.ZipFile(path) as source:
            expected = source.read("danbooru_1.webp")
            members = index_members(source, ["danbooru_1.webp"])
        with open(path, "rb") as raw:
            assert read_member(raw, members[0]) == expected


class TestBuildSupplement:
    def test_it_writes_the_planned_number_of_each_source(self, tmp_path, sources):
        out = tmp_path / "supplement.zip"
        counts = {"anime_general": 5, "anime_sensitive": 5, "anime_explicit": 10}
        build_supplement(counts, out, open_archive=sources, open_stream=sources.stream, seed=0)

        with zipfile.ZipFile(out) as archive:
            written = [n for n in archive.namelist() if n.endswith(".webp")]
        for source, expected in counts.items():
            assert sum(1 for n in written if n.startswith(f"{source}/")) == expected

    def test_non_image_members_are_never_selected(self, tmp_path, noisy_sources):
        # The rating archives carry metadata alongside images, and `.gif` is not
        # in IMAGE_SUFFIXES. Selecting either yields a supplement that loads
        # short and aborts the run at the volume check, after the whole fetch.
        out = tmp_path / "supplement.zip"
        build_supplement({"anime_general": 20}, out, open_archive=noisy_sources, open_stream=noisy_sources.stream, seed=0)

        with zipfile.ZipFile(out) as archive:
            written = archive.namelist()
        assert len(written) == 20
        assert all(n.endswith(".webp") for n in written)

    def test_a_supplement_from_noisy_sources_still_loads_at_full_size(
        self, tmp_path, noisy_sources
    ):
        from holy_blocker_ml.dataset import ZipImageDataset

        out = tmp_path / "supplement.zip"
        build_supplement({"anime_general": 20}, out, open_archive=noisy_sources, open_stream=noisy_sources.stream, seed=0)
        dataset = ZipImageDataset(
            out,
            image_size=32,
            augment=False,
            policy=ANIME_LABEL_POLICY,
            classes=ANIME_SOURCE_CLASSES,
        )
        assert len(dataset) == 20

    def test_a_fetch_that_dies_partway_is_not_written_out_as_success(self, tmp_path, sources):
        # The real failure: HfFileSystem shares one httpx client across cached
        # instances, so closing a handle killed every later rating. The build
        # exited 0 with a zip holding 2,783 of 4,480 images.
        opened = {"n": 0}

        def flaky_stream(filename):
            opened["n"] += 1
            if opened["n"] > 1:
                raise RuntimeError("Cannot send a request, as the client has been closed.")
            return sources.stream(filename)

        with pytest.raises(RuntimeError, match="client has been closed|wrote \\d+ of"):
            build_supplement(
                {"anime_general": 10, "anime_explicit": 10},
                tmp_path / "partial.zip",
                open_archive=sources,
                open_stream=flaky_stream,
                seed=0,
                workers=1,
            )

    def test_asking_for_more_images_than_a_rating_holds_is_rejected(
        self, tmp_path, noisy_sources
    ):
        out = tmp_path / "supplement.zip"
        with pytest.raises(ValueError, match="decodable images"):
            build_supplement({"anime_general": 50}, out, open_archive=noisy_sources, open_stream=noisy_sources.stream, seed=0)

    def test_the_layout_is_readable_as_a_training_dataset(self, tmp_path, sources):
        from holy_blocker_ml.dataset import ZipImageDataset

        out = tmp_path / "supplement.zip"
        counts = {"anime_general": 4, "anime_explicit": 6}
        build_supplement(counts, out, open_archive=sources, open_stream=sources.stream, seed=0)

        dataset = ZipImageDataset(
            out,
            image_size=32,
            augment=False,
            policy=ANIME_LABEL_POLICY,
            classes=ANIME_SOURCE_CLASSES,
        )
        assert len(dataset) == 10
        image, label = dataset[0]
        assert image.shape == (3, 32, 32)
        assert label in (0, 1)

    def test_each_rating_gets_the_right_binary_label(self, tmp_path, sources):
        from holy_blocker_ml.dataset import ZipImageDataset
        from holy_blocker_ml.labels import BINARY_LABELS

        out = tmp_path / "supplement.zip"
        build_supplement(
            {s: 3 for s in ANIME_SOURCE_CLASSES}, out, open_archive=sources, open_stream=sources.stream, seed=0
        )
        dataset = ZipImageDataset(
            out,
            image_size=32,
            augment=False,
            policy=ANIME_LABEL_POLICY,
            classes=ANIME_SOURCE_CLASSES,
        )
        seen = dict(zip(dataset.source_labels, [label for _, label in dataset.samples]))
        for source in ANIME_SAFE_SOURCES:
            assert BINARY_LABELS[seen[source]] == SAFE
        assert BINARY_LABELS[seen["anime_explicit"]] == EXPLICIT

    def test_an_unknown_source_is_rejected(self, tmp_path, sources):
        with pytest.raises(ValueError, match="unknown anime source"):
            build_supplement({"anime_nope": 1}, tmp_path / "x.zip", open_archive=sources, open_stream=sources.stream)

    def test_the_per_source_seed_survives_a_fresh_interpreter(self):
        # PYTHONHASHSEED randomises str hashing per process. A per-source seed
        # derived from hash() would reselect the supplement on every run while
        # still reporting the same --seed, so this has to be checked in a
        # separate interpreter rather than in-process.
        import subprocess
        import sys

        script = (
            "from holy_blocker_ml.anime import source_seed, ANIME_SOURCE_CLASSES;"
            "print([source_seed(s, 5) for s in ANIME_SOURCE_CLASSES])"
        )
        runs = {
            subprocess.run(
                [sys.executable, "-c", script],
                capture_output=True,
                text=True,
                check=True,
                env={"PYTHONHASHSEED": str(salt), "PATH": "/usr/bin:/bin"},
            ).stdout.strip()
            for salt in (0, 1, 2)
        }
        assert len(runs) == 1, f"per-source seed varies with PYTHONHASHSEED: {runs}"

    def test_it_is_deterministic_under_a_seed(self, tmp_path, sources):
        counts = {"anime_general": 7}
        first, second = tmp_path / "a.zip", tmp_path / "b.zip"
        build_supplement(counts, first, open_archive=sources, open_stream=sources.stream, seed=11)
        build_supplement(counts, second, open_archive=sources, open_stream=sources.stream, seed=11)
        with zipfile.ZipFile(first) as a, zipfile.ZipFile(second) as b:
            assert sorted(a.namelist()) == sorted(b.namelist())


def make_corpus_zip(path, per_class=20):
    """A stand-in for the nsfw_detect archive, laid out `<class>/<name>.png`."""
    from holy_blocker_ml.features import SOURCE_CLASSES

    with zipfile.ZipFile(path, "w") as archive:
        for name in SOURCE_CLASSES:
            for i in range(per_class):
                buffer = io.BytesIO()
                Image.new("RGB", (8, 8), (i % 256, 0, 0)).save(buffer, format="PNG")
                archive.writestr(f"nsfw_dataset_v1/{name}/{i}.png", buffer.getvalue())


class TestBuildTrainingSets:
    @pytest.fixture
    def corpus(self, tmp_path):
        path = tmp_path / "corpus.zip"
        make_corpus_zip(path)
        return path

    @pytest.fixture
    def labels(self):
        from holy_blocker_ml.features import SOURCE_CLASSES

        return [name for name in SOURCE_CLASSES for _ in range(20)]

    def test_the_validation_half_is_the_unmodified_split(self, corpus, labels):
        from holy_blocker_ml.anime import build_training_sets

        plan = mixture_plan(labels, drop_fraction=0.5)
        _, val_set = build_training_sets(corpus, None, plan, image_size=32)

        _, expected_val = stratified_split(labels, val_fraction=0.2, seed=0)
        assert len(val_set) == len(expected_val)
        assert val_set.source_labels == [labels[i] for i in expected_val]

    def test_the_ablation_arm_needs_no_supplement(self, corpus, labels):
        from holy_blocker_ml.anime import build_training_sets

        plan = mixture_plan(labels, drop_fraction=0.5)
        train_set, _ = build_training_sets(corpus, None, plan, image_size=32)
        assert len(train_set) == len(plan.kept_train_indices)

    def test_the_addition_arm_concatenates_the_supplement(
        self, tmp_path, corpus, labels, sources
    ):
        from holy_blocker_ml.anime import build_training_sets

        plan = mixture_plan(labels, anime_count=8)
        supplement = tmp_path / "supp.zip"
        build_supplement(plan.anime_counts, supplement, open_archive=sources, open_stream=sources.stream, seed=0)

        train_set, _ = build_training_sets(corpus, supplement, plan, image_size=32)
        original_train, _ = stratified_split(labels, val_fraction=0.2, seed=0)
        assert len(train_set) == len(original_train) + 8

    def test_a_supplement_of_the_wrong_size_is_refused(self, tmp_path, corpus, labels, sources):
        from holy_blocker_ml.anime import build_training_sets

        plan = mixture_plan(labels, anime_count=8)
        wrong = tmp_path / "wrong.zip"
        build_supplement({"anime_general": 3}, wrong, open_archive=sources, open_stream=sources.stream, seed=0)

        with pytest.raises(ValueError, match="could not be attributed"):
            build_training_sets(corpus, wrong, plan, image_size=32)


class TestDeterminism:
    def test_the_same_seed_gives_the_same_plan(self):
        first = mixture_plan(CORPUS, drop_fraction=0.5, seed=0)
        second = mixture_plan(CORPUS, drop_fraction=0.5, seed=0)
        assert first.dropped_indices == second.dropped_indices

    def test_a_different_drop_seed_drops_different_samples(self):
        first = mixture_plan(CORPUS, drop_fraction=0.5, drop_seed=1)
        second = mixture_plan(CORPUS, drop_fraction=0.5, drop_seed=2)
        assert first.dropped_indices != second.dropped_indices

    def test_the_drop_seed_does_not_disturb_the_validation_split(self):
        first = mixture_plan(CORPUS, drop_fraction=0.5, drop_seed=1)
        second = mixture_plan(CORPUS, drop_fraction=0.5, drop_seed=2)
        assert first.val_indices == second.val_indices
