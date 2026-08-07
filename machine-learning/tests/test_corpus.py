from pathlib import Path

import pytest

from holy_blocker_ml.corpus import CorpusKind, CorpusSpec, load_corpus


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
