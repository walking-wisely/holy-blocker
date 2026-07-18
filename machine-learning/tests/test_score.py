"""Tests for the holdout selection behind the experiment scorer.

The scorer's job is to put a checkpoint and a set of archive indices together.
The part worth testing is the index handling: a silent mismatch between the
validation split and the common holdout would score the wrong samples and still
print a plausible-looking table.
"""

import pytest

from holy_blocker_ml.score import positions_within, require_subset


class TestRequireSubset:
    def test_it_accepts_a_genuine_subset(self):
        require_subset([2, 4], [1, 2, 3, 4, 5], "common holdout", "validation split")

    def test_it_accepts_an_identical_set(self):
        require_subset([1, 2], [1, 2], "a", "b")

    def test_it_rejects_indices_outside_the_containing_set(self):
        """A common holdout leaking outside the split means the split changed.

        That silently scores samples the model may have trained on, which would
        inflate the result rather than fail.
        """
        with pytest.raises(ValueError) as excinfo:
            require_subset([2, 99], [1, 2, 3], "common holdout", "validation split")

        assert "common holdout" in str(excinfo.value)
        assert "validation split" in str(excinfo.value)

    def test_the_error_names_the_offending_indices(self):
        with pytest.raises(ValueError) as excinfo:
            require_subset([7, 8], [1, 2], "inner", "outer")

        message = str(excinfo.value)
        assert "7" in message and "8" in message

    def test_an_empty_subset_is_accepted(self):
        require_subset([], [1, 2], "inner", "outer")


class TestPositionsWithin:
    def test_it_maps_archive_indices_to_positions_in_the_scored_order(self):
        # Scored 10, 20, 30, 40; we want where 20 and 40 landed.
        assert positions_within([20, 40], [10, 20, 30, 40]) == [1, 3]

    def test_it_follows_the_containing_order_not_the_requested_order(self):
        """Predictions come back in dataset order, so positions must match it."""
        assert positions_within([40, 20], [10, 20, 30, 40]) == [1, 3]

    def test_it_rejects_an_index_that_was_not_scored(self):
        with pytest.raises(ValueError):
            positions_within([99], [10, 20])

    def test_it_returns_nothing_for_an_empty_request(self):
        assert positions_within([], [10, 20]) == []
