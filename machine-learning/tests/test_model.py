import pytest

from holy_blocker_ml.model import resolve_nsfw_index


def test_resolve_nsfw_index_falconsai_label_set() -> None:
    # The exact id2label Falconsai/nsfw_image_detection ships.
    assert resolve_nsfw_index({0: "normal", 1: "nsfw"}) == 1


def test_resolve_nsfw_index_is_order_independent() -> None:
    assert resolve_nsfw_index({0: "nsfw", 1: "normal"}) == 0


def test_resolve_nsfw_index_tolerates_case_and_whitespace() -> None:
    assert resolve_nsfw_index({0: " Normal ", 1: " NSFW "}) == 1


def test_resolve_nsfw_index_falls_back_to_elimination_for_two_classes() -> None:
    # Neither label matches an NSFW alias exactly, but exactly one matches a
    # safe alias and there are only two classes -> the other one must be it.
    assert resolve_nsfw_index({0: "safe", 1: "questionable"}) == 1


def test_resolve_nsfw_index_raises_when_ambiguous() -> None:
    with pytest.raises(ValueError):
        resolve_nsfw_index({0: "cat", 1: "dog", 2: "bird"})


def test_resolve_nsfw_index_raises_when_multiple_labels_match() -> None:
    with pytest.raises(ValueError):
        resolve_nsfw_index({0: "nsfw", 1: "porn"})
