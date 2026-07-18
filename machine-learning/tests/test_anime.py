"""Tests for the anime-subsampling substitution plan.

The [experiment](../../docs/components/machine-learning/experiments/anime-subsampling.md)
is judged against baselines fixed on `stratified_split(seed=0, val_fraction=0.2)`
over the original archive. Everything here exists to keep that comparison valid:

- the validation half must come back **byte-identical** to the unmodified split,
  or the run is scored against samples the baselines never covered;
- substitution must be **volume-neutral**, or a result attributed to label
  quality is really a result about data volume;
- `questionable` must never reach training, because assigning it a side
  re-imports the arbitrariness the experiment is trying to remove.

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
    QUESTIONABLE,
    RATING_ARCHIVES,
    build_supplement,
    select_members,
    substitution_plan,
)
from holy_blocker_ml.features import map_source_label
from holy_blocker_ml.finetune import stratified_split
from holy_blocker_ml.labels import EXPLICIT, SAFE
from holy_blocker_ml.medium import medium_of

#: The real archive: five classes, 5,600 each.
CORPUS = ["drawings"] * 5600 + ["hentai"] * 5600 + ["neutral"] * 5600 + ["porn"] * 5600 + [
    "sexy"
] * 5600


@pytest.fixture
def plan():
    return substitution_plan(CORPUS, replace_fraction=0.5)


class TestRatingMapping:
    def test_general_and_sensitive_are_safe(self):
        assert ANIME_LABEL_POLICY["anime_general"] == SAFE
        assert ANIME_LABEL_POLICY["anime_sensitive"] == SAFE

    def test_explicit_is_explicit(self):
        assert ANIME_LABEL_POLICY["anime_explicit"] == EXPLICIT

    def test_questionable_is_absent_from_the_policy(self):
        # Absence is the mechanism, not an oversight: map_source_label returns
        # None for unmapped classes and the dataset drops them, so holding
        # `questionable` out of training is enforced by the pipeline rather
        # than by remembering to filter it.
        assert QUESTIONABLE not in ANIME_LABEL_POLICY

    def test_questionable_maps_to_no_training_label(self):
        assert map_source_label(QUESTIONABLE, ANIME_LABEL_POLICY) is None

    def test_every_anime_source_is_drawn(self):
        for source in ANIME_DRAWN_SOURCES:
            assert medium_of(source) == "drawn"

    def test_a_rating_archive_is_known_for_every_rating(self):
        for source in (*ANIME_DRAWN_SOURCES, QUESTIONABLE):
            assert source in RATING_ARCHIVES


class TestValidationHalfIsUntouched:
    def test_validation_indices_match_the_unmodified_split(self, plan):
        _, expected_val = stratified_split(CORPUS, val_fraction=0.2, seed=0)
        assert plan.val_indices == expected_val

    def test_no_validation_index_is_dropped(self, plan):
        assert not set(plan.dropped_indices) & set(plan.val_indices)

    def test_no_validation_index_survives_into_the_training_half(self, plan):
        assert not set(plan.kept_train_indices) & set(plan.val_indices)


class TestSubstitutionIsVolumeNeutral:
    def test_training_size_is_unchanged(self, plan):
        expected_train, _ = stratified_split(CORPUS, val_fraction=0.2, seed=0)
        assert len(plan.kept_train_indices) + plan.anime_total == len(expected_train)

    def test_drawn_training_volume_is_preserved(self, plan):
        kept_drawn = sum(1 for i in plan.kept_train_indices if medium_of(CORPUS[i]) == "drawn")
        assert kept_drawn + plan.anime_total == 8960

    def test_photographic_training_data_is_untouched(self, plan):
        kept_photo = sorted(
            i for i in plan.kept_train_indices if medium_of(CORPUS[i]) == "photographic"
        )
        train, _ = stratified_split(CORPUS, val_fraction=0.2, seed=0)
        original_photo = sorted(i for i in train if medium_of(CORPUS[i]) == "photographic")
        assert kept_photo == original_photo

    def test_only_drawn_samples_are_dropped(self, plan):
        assert {medium_of(CORPUS[i]) for i in plan.dropped_indices} == {"drawn"}

    def test_dropped_samples_all_come_from_the_training_half(self, plan):
        train, _ = stratified_split(CORPUS, val_fraction=0.2, seed=0)
        assert set(plan.dropped_indices) <= set(train)


class TestClassBalanceIsPreserved:
    def test_equal_numbers_are_dropped_from_each_drawn_class(self, plan):
        dropped = [CORPUS[i] for i in plan.dropped_indices]
        assert dropped.count("drawings") == dropped.count("hentai") == 2240

    def test_safe_and_explicit_anime_counts_match_what_they_replace(self, plan):
        safe = sum(plan.anime_counts[s] for s in ANIME_SAFE_SOURCES)
        explicit = sum(plan.anime_counts[s] for s in ANIME_EXPLICIT_SOURCES)
        assert safe == 2240  # replaces the dropped `drawings`
        assert explicit == 2240  # replaces the dropped `hentai`

    def test_the_safe_allocation_splits_evenly_across_its_two_ratings(self, plan):
        assert plan.anime_counts["anime_general"] == plan.anime_counts["anime_sensitive"] == 1120

    def test_questionable_is_never_allocated_training_samples(self, plan):
        assert QUESTIONABLE not in plan.anime_counts


class TestReplaceFraction:
    def test_a_zero_fraction_changes_nothing(self):
        empty = substitution_plan(CORPUS, replace_fraction=0.0)
        train, _ = stratified_split(CORPUS, val_fraction=0.2, seed=0)
        assert empty.dropped_indices == []
        assert empty.kept_train_indices == train
        assert empty.anime_total == 0

    def test_a_full_fraction_replaces_every_drawn_training_sample(self):
        full = substitution_plan(CORPUS, replace_fraction=1.0)
        kept_drawn = [i for i in full.kept_train_indices if medium_of(CORPUS[i]) == "drawn"]
        assert kept_drawn == []
        assert full.anime_total == 8960

    @pytest.mark.parametrize("fraction", [-0.1, 1.1])
    def test_a_fraction_outside_the_unit_interval_is_rejected(self, fraction):
        with pytest.raises(ValueError):
            substitution_plan(CORPUS, replace_fraction=fraction)


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


def make_rating_zip(path, count):
    """A stand-in for one remote rating archive, laid out flat like the real one."""
    with zipfile.ZipFile(path, "w") as archive:
        for i in range(count):
            buffer = io.BytesIO()
            Image.new("RGB", (8, 8), (i % 256, 0, 0)).save(buffer, format="WEBP")
            archive.writestr(f"danbooru_{i}.webp", buffer.getvalue())


class TestBuildSupplement:
    @pytest.fixture
    def sources(self, tmp_path):
        """Local stand-ins for the four remote rating archives."""
        root = tmp_path / "remote"
        root.mkdir()
        for source, filename in RATING_ARCHIVES.items():
            make_rating_zip(root / filename, count=100)
        return lambda filename: zipfile.ZipFile(root / filename)

    def test_it_writes_the_planned_number_of_each_source(self, tmp_path, sources):
        out = tmp_path / "supplement.zip"
        counts = {"anime_general": 5, "anime_sensitive": 5, "anime_explicit": 10}
        build_supplement(counts, out, open_archive=sources, seed=0)

        with zipfile.ZipFile(out) as archive:
            written = [n for n in archive.namelist() if n.endswith(".webp")]
        for source, expected in counts.items():
            assert sum(1 for n in written if n.startswith(f"{source}/")) == expected

    def test_the_layout_is_readable_as_a_training_dataset(self, tmp_path, sources):
        from holy_blocker_ml.dataset import ZipImageDataset

        out = tmp_path / "supplement.zip"
        counts = {"anime_general": 4, "anime_explicit": 6}
        build_supplement(counts, out, open_archive=sources, seed=0)

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

    def test_safe_and_explicit_ratings_get_the_right_binary_labels(self, tmp_path, sources):
        from holy_blocker_ml.dataset import ZipImageDataset
        from holy_blocker_ml.labels import BINARY_LABELS

        out = tmp_path / "supplement.zip"
        build_supplement(
            {"anime_general": 3, "anime_explicit": 3}, out, open_archive=sources, seed=0
        )
        dataset = ZipImageDataset(
            out, image_size=32, augment=False, policy=ANIME_LABEL_POLICY,
            classes=ANIME_SOURCE_CLASSES,
        )
        by_source = dict(zip(dataset.source_labels, [lbl for _, lbl in dataset.samples]))
        assert BINARY_LABELS[by_source["anime_general"]] == SAFE
        assert BINARY_LABELS[by_source["anime_explicit"]] == EXPLICIT

    def test_a_questionable_supplement_yields_no_trainable_samples(self, tmp_path, sources):
        from holy_blocker_ml.dataset import ZipImageDataset

        # The boundary set is buildable, but the training policy must refuse to
        # give it a label — that is what keeps it out of gradient updates.
        out = tmp_path / "boundary.zip"
        build_supplement({QUESTIONABLE: 8}, out, open_archive=sources, seed=0)

        from holy_blocker_ml.features import ArchiveLayoutError

        with pytest.raises((ArchiveLayoutError, ValueError)):
            ZipImageDataset(
                out, image_size=32, augment=False, policy=ANIME_LABEL_POLICY,
                classes=ANIME_SOURCE_CLASSES,
            )

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
        build_supplement(counts, first, open_archive=sources, seed=11)
        build_supplement(counts, second, open_archive=sources, seed=11)
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

        plan = substitution_plan(labels, replace_fraction=0.5)
        _, val_set = build_training_sets(corpus, None, plan, image_size=32)

        _, expected_val = stratified_split(labels, val_fraction=0.2, seed=0)
        assert len(val_set) == len(expected_val)
        assert val_set.source_labels == [labels[i] for i in expected_val]

    def test_the_training_half_holds_its_planned_volume(self, tmp_path, corpus, labels, sources_for):
        from holy_blocker_ml.anime import build_training_sets

        plan = substitution_plan(labels, replace_fraction=0.5)
        supplement = tmp_path / "supp.zip"
        build_supplement(plan.anime_counts, supplement, open_archive=sources_for, seed=0)

        train_set, _ = build_training_sets(corpus, supplement, plan, image_size=32)
        original_train, _ = stratified_split(labels, val_fraction=0.2, seed=0)
        assert len(train_set) == len(original_train)

    def test_a_supplement_of_the_wrong_size_is_refused(
        self, tmp_path, corpus, labels, sources_for
    ):
        from holy_blocker_ml.anime import build_training_sets

        plan = substitution_plan(labels, replace_fraction=0.5)
        wrong = tmp_path / "wrong.zip"
        build_supplement({"anime_general": 3}, wrong, open_archive=sources_for, seed=0)

        with pytest.raises(ValueError, match="volume-neutral"):
            build_training_sets(corpus, wrong, plan, image_size=32)


@pytest.fixture
def sources_for(tmp_path):
    root = tmp_path / "remote_shared"
    root.mkdir()
    for filename in RATING_ARCHIVES.values():
        make_rating_zip(root / filename, count=100)
    return lambda filename: zipfile.ZipFile(root / filename)


class TestDeterminism:
    def test_the_same_seed_gives_the_same_plan(self):
        first = substitution_plan(CORPUS, replace_fraction=0.5, seed=0)
        second = substitution_plan(CORPUS, replace_fraction=0.5, seed=0)
        assert first.dropped_indices == second.dropped_indices

    def test_a_different_drop_seed_drops_different_samples(self):
        first = substitution_plan(CORPUS, replace_fraction=0.5, drop_seed=1)
        second = substitution_plan(CORPUS, replace_fraction=0.5, drop_seed=2)
        assert first.dropped_indices != second.dropped_indices

    def test_the_drop_seed_does_not_disturb_the_validation_split(self):
        first = substitution_plan(CORPUS, replace_fraction=0.5, drop_seed=1)
        second = substitution_plan(CORPUS, replace_fraction=0.5, drop_seed=2)
        assert first.val_indices == second.val_indices
