"""Tests for medium grouping — the instrument the anime-subsampling experiment
is scored with.

The pre-registered decision rule compares drawn and photographic AUC against
fixed baselines, so a defect here would silently change the verdict rather than
fail loudly. These tests pin the grouping and the subsetting.
"""

import pytest

from holy_blocker_ml.eval import Predictions
from holy_blocker_ml.medium import (
    DRAWN,
    PHOTOGRAPHIC,
    medium_of,
    medium_report,
    per_medium_auc,
    subset_by_source,
)


def make_predictions(targets, scores):
    return Predictions(targets=list(targets), positive_scores=list(scores))


class TestMediumMembership:
    def test_the_five_source_classes_split_into_two_media(self):
        assert set(PHOTOGRAPHIC) == {"neutral", "sexy", "porn"}
        assert set(DRAWN) == {"drawings", "hentai"}

    def test_the_two_media_are_disjoint_and_cover_every_source_class(self):
        assert not set(PHOTOGRAPHIC) & set(DRAWN)
        assert len(set(PHOTOGRAPHIC) | set(DRAWN)) == 5

    @pytest.mark.parametrize(
        ("source", "expected"),
        [
            ("neutral", "photographic"),
            ("sexy", "photographic"),
            ("porn", "photographic"),
            ("drawings", "drawn"),
            ("hentai", "drawn"),
        ],
    )
    def test_each_source_class_maps_to_its_medium(self, source, expected):
        assert medium_of(source) == expected

    def test_the_singular_drawing_spelling_is_rejected(self):
        """`drawing` vs `drawings` already cost a fifth of the dataset once.

        The corpus class is plural; the singular form must not silently pass.
        """
        with pytest.raises(KeyError):
            medium_of("drawing")

    def test_an_unknown_source_class_raises_rather_than_defaulting(self):
        with pytest.raises(KeyError):
            medium_of("anime_dbrating")


class TestSubsetBySource:
    def test_it_keeps_only_the_requested_sources(self):
        predictions = make_predictions([0, 1, 0, 1], [0.1, 0.9, 0.2, 0.8])
        sources = ["neutral", "porn", "drawings", "hentai"]

        subset = subset_by_source(predictions, sources, DRAWN)

        assert subset.targets == [0, 1]
        assert subset.positive_scores == [0.2, 0.8]

    def test_it_preserves_sample_order(self):
        predictions = make_predictions([0, 0, 1], [0.3, 0.1, 0.7])
        sources = ["hentai", "neutral", "drawings"]

        subset = subset_by_source(predictions, sources, DRAWN)

        assert subset.positive_scores == [0.3, 0.7]

    def test_it_rejects_a_source_list_of_the_wrong_length(self):
        """A mismatched zip would silently truncate and score a partial set."""
        predictions = make_predictions([0, 1, 0], [0.1, 0.9, 0.2])

        with pytest.raises(ValueError):
            subset_by_source(predictions, ["neutral", "porn"], PHOTOGRAPHIC)

    def test_it_returns_an_empty_prediction_set_when_nothing_matches(self):
        predictions = make_predictions([0, 1], [0.1, 0.9])

        subset = subset_by_source(predictions, ["neutral", "porn"], DRAWN)

        assert len(subset) == 0


class TestPerMediumAuc:
    def test_it_scores_each_medium_independently(self):
        # Photographic separates perfectly; drawn is inverted.
        targets = [0, 1, 0, 1]
        scores = [0.1, 0.9, 0.9, 0.1]
        sources = ["neutral", "porn", "drawings", "hentai"]

        result = per_medium_auc(make_predictions(targets, scores), sources)

        assert result["photographic"] == pytest.approx(1.0)
        assert result["drawn"] == pytest.approx(0.0)

    def test_a_mediums_score_ignores_the_other_mediums_samples(self):
        """The combined AUC is not a weighted average of the parts.

        Guards against an implementation that ranks across all samples and then
        filters, which would let photographic scores shift the drawn ranking.
        """
        targets = [0, 1, 0, 1]
        scores = [0.01, 0.02, 0.3, 0.7]
        sources = ["neutral", "porn", "drawings", "hentai"]

        result = per_medium_auc(make_predictions(targets, scores), sources)

        assert result["photographic"] == pytest.approx(1.0)
        assert result["drawn"] == pytest.approx(1.0)

    def test_it_reports_the_combined_auc_over_every_sample(self):
        targets = [0, 1, 0, 1]
        scores = [0.1, 0.9, 0.2, 0.8]
        sources = ["neutral", "porn", "drawings", "hentai"]

        result = per_medium_auc(make_predictions(targets, scores), sources)

        assert result["combined"] == pytest.approx(1.0)

    def test_it_reports_the_sample_count_backing_each_medium(self):
        targets = [0, 1, 0, 1, 0]
        scores = [0.1, 0.9, 0.2, 0.8, 0.3]
        sources = ["neutral", "porn", "sexy", "drawings", "hentai"]

        counts = per_medium_auc(
            make_predictions(targets, scores), sources, with_counts=True
        )["counts"]

        assert counts == {"photographic": 3, "drawn": 2, "combined": 5}

    def test_a_medium_with_only_one_label_reports_undefined_rather_than_a_number(self):
        """AUC needs both classes present; a single-label medium has no ranking.

        Reporting 0.0 or 0.5 here would look like a real measurement and could
        trip the decision rule.
        """
        targets = [0, 0, 0, 1]
        scores = [0.1, 0.2, 0.3, 0.9]
        sources = ["drawings", "hentai", "drawings", "porn"]

        result = per_medium_auc(make_predictions(targets, scores), sources)

        assert result["drawn"] is None


class TestMediumReport:
    def test_it_names_every_medium_and_its_sample_count(self):
        targets = [0, 1, 0, 1]
        scores = [0.1, 0.9, 0.2, 0.8]
        sources = ["neutral", "porn", "drawings", "hentai"]

        text = medium_report(make_predictions(targets, scores), sources)

        assert "photographic" in text
        assert "drawn" in text
        assert "combined" in text

    def test_undefined_media_render_without_crashing(self):
        targets = [0, 0, 1, 0]
        scores = [0.1, 0.2, 0.9, 0.3]
        sources = ["drawings", "hentai", "porn", "neutral"]

        text = medium_report(make_predictions(targets, scores), sources)

        assert "drawn" in text
