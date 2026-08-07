from pathlib import Path

from holy_blocker_ml.synth_ui import (
    SCENE_KINDS,
    generate_corpus,
    plan_corpus,
    render_image,
)


def test_plan_corpus_is_deterministic_given_same_seed() -> None:
    a = plan_corpus(20, seed=42)
    b = plan_corpus(20, seed=42)
    assert a == b


def test_plan_corpus_differs_across_seeds() -> None:
    a = plan_corpus(20, seed=1)
    b = plan_corpus(20, seed=2)
    assert a != b


def test_plan_corpus_returns_requested_count() -> None:
    specs = plan_corpus(37, seed=0)
    assert len(specs) == 37


def test_plan_corpus_only_uses_known_scene_kinds() -> None:
    specs = plan_corpus(50, seed=7)
    assert {s.scene for s in specs} <= set(SCENE_KINDS)


def test_plan_corpus_covers_multiple_scenes_over_enough_samples() -> None:
    specs = plan_corpus(200, seed=123)
    # With 200 draws over 6 scene kinds, seeing only one or two kinds would
    # indicate a selection bug, not bad luck.
    assert len({s.scene for s in specs}) >= 4


def test_render_image_produces_requested_dimensions() -> None:
    spec = plan_corpus(1, seed=5)[0]
    image = render_image(spec)
    assert image.size == (spec.width, spec.height)
    assert image.mode == "RGB"


def test_generate_corpus_writes_requested_file_count(tmp_path: Path) -> None:
    paths = generate_corpus(tmp_path, count=5, seed=1)
    assert len(paths) == 5
    for path in paths:
        assert path.is_file()
        assert path.suffix == ".png"


def test_generate_corpus_is_reproducible_in_composition(tmp_path: Path) -> None:
    out_a = tmp_path / "a"
    out_b = tmp_path / "b"
    paths_a = generate_corpus(out_a, count=4, seed=99)
    paths_b = generate_corpus(out_b, count=4, seed=99)
    # Same plan -> same filenames (scene names embedded), even though these
    # are two separate directories.
    assert [p.name for p in paths_a] == [p.name for p in paths_b]
