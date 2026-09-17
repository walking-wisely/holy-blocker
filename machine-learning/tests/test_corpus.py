from pathlib import Path

import pytest

from holy_blocker_ml.corpus import CorpusKind, CorpusSpec, discover_corpora, load_corpus


def test_load_corpus_raises_on_missing_root(tmp_path: Path) -> None:
    spec = CorpusSpec(name="missing", root=tmp_path / "nope", kind=CorpusKind.BENIGN)
    with pytest.raises(FileNotFoundError, match="missing"):
        load_corpus(spec)


def test_load_corpus_finds_only_image_files(tmp_path: Path) -> None:
    (tmp_path / "a.png").write_bytes(b"fake")
    (tmp_path / "b.PNG").write_bytes(b"fake")
    (tmp_path / "notes.txt").write_bytes(b"fake")
    sub = tmp_path / "sub"
    sub.mkdir()
    (sub / "c.jpg").write_bytes(b"fake")

    spec = CorpusSpec(name="mixed", root=tmp_path, kind=CorpusKind.BENIGN)
    found = load_corpus(spec)

    assert found == sorted(found)
    names = {p.name for p in found}
    assert names == {"a.png", "b.PNG", "c.jpg"}


def test_load_corpus_empty_dir_returns_empty_list(tmp_path: Path) -> None:
    spec = CorpusSpec(name="empty", root=tmp_path, kind=CorpusKind.BENIGN)
    assert load_corpus(spec) == []


def test_discover_corpora_splits_present_from_missing(tmp_path: Path) -> None:
    (tmp_path / "synthetic-ui").mkdir()
    # "real-world-macbook" deliberately not created — e.g. a fresh checkout
    # that never ran the manual screenshot batch.

    found, missing = discover_corpora(
        tmp_path,
        [
            ("synthetic-ui", CorpusKind.BENIGN),
            ("real-world-macbook", CorpusKind.BENIGN),
        ],
    )

    assert [spec.name for spec in found] == ["synthetic-ui"]
    assert found[0].root == tmp_path / "synthetic-ui"
    assert found[0].kind is CorpusKind.BENIGN
    assert missing == ["real-world-macbook"]


def test_discover_corpora_treats_a_file_as_not_present(tmp_path: Path) -> None:
    # A stray non-directory at the expected path (e.g. a leftover .DS_Store-like
    # artifact) must not be reported as a loadable corpus.
    (tmp_path / "synthetic-ui").write_bytes(b"not a directory")

    found, missing = discover_corpora(tmp_path, [("synthetic-ui", CorpusKind.BENIGN)])

    assert found == []
    assert missing == ["synthetic-ui"]


def test_discover_corpora_empty_input_returns_empty_lists(tmp_path: Path) -> None:
    assert discover_corpora(tmp_path, []) == ([], [])
