from pathlib import Path

import pytest

from holy_blocker_ml.env import find_dotenv, load_dotenv, require_env


def write_env(tmp_path: Path, body: str) -> Path:
    path = tmp_path / ".env"
    path.write_text(body)
    return path


def test_parses_simple_assignments(tmp_path: Path) -> None:
    path = write_env(tmp_path, "HF_TOKEN=hf_abc123\nOTHER=2\n")

    assert load_dotenv(path, apply=False) == {"HF_TOKEN": "hf_abc123", "OTHER": "2"}


def test_ignores_comments_and_blank_lines(tmp_path: Path) -> None:
    path = write_env(tmp_path, "# a comment\n\n  \nHF_TOKEN=x\n# trailing\n")

    assert load_dotenv(path, apply=False) == {"HF_TOKEN": "x"}


def test_strips_quotes_and_the_export_prefix(tmp_path: Path) -> None:
    path = write_env(tmp_path, 'export HF_TOKEN="hf_quoted"\nSINGLE=\'sq\'\n')

    assert load_dotenv(path, apply=False) == {"HF_TOKEN": "hf_quoted", "SINGLE": "sq"}


def test_keeps_equals_signs_inside_the_value(tmp_path: Path) -> None:
    path = write_env(tmp_path, "URL=https://x/y?a=1&b=2\n")

    assert load_dotenv(path, apply=False)["URL"] == "https://x/y?a=1&b=2"


def test_surrounding_whitespace_is_trimmed(tmp_path: Path) -> None:
    path = write_env(tmp_path, "  HF_TOKEN =  hf_spaced  \n")

    assert load_dotenv(path, apply=False) == {"HF_TOKEN": "hf_spaced"}


def test_lines_without_an_equals_sign_are_skipped(tmp_path: Path) -> None:
    path = write_env(tmp_path, "GARBAGE\nHF_TOKEN=ok\n")

    assert load_dotenv(path, apply=False) == {"HF_TOKEN": "ok"}


def test_missing_file_is_not_an_error(tmp_path: Path) -> None:
    assert load_dotenv(tmp_path / "nope.env", apply=False) == {}


def test_apply_does_not_clobber_an_existing_variable(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("HF_TOKEN", "from-shell")
    path = write_env(tmp_path, "HF_TOKEN=from-file\n")

    load_dotenv(path, apply=True)

    # The shell is the more explicit source; the file must not win.
    import os

    assert os.environ["HF_TOKEN"] == "from-shell"


def test_override_forces_the_file_value(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("HF_TOKEN", "from-shell")
    path = write_env(tmp_path, "HF_TOKEN=from-file\n")

    load_dotenv(path, apply=True, override=True)

    import os

    assert os.environ["HF_TOKEN"] == "from-file"


def test_find_dotenv_walks_up_from_a_nested_directory(tmp_path: Path) -> None:
    (tmp_path / ".env").write_text("HF_TOKEN=x\n")
    nested = tmp_path / "a" / "b"
    nested.mkdir(parents=True)

    assert find_dotenv(nested) == tmp_path / ".env"


def test_find_dotenv_returns_none_when_absent(tmp_path: Path) -> None:
    assert find_dotenv(tmp_path) is None


def test_require_env_returns_the_value(monkeypatch) -> None:
    monkeypatch.setenv("HF_TOKEN", "hf_ok")

    assert require_env("HF_TOKEN") == "hf_ok"


def test_require_env_error_names_the_variable_and_how_to_set_it(monkeypatch) -> None:
    monkeypatch.delenv("HF_TOKEN", raising=False)

    with pytest.raises(SystemExit) as excinfo:
        require_env("HF_TOKEN", hint="Accept the terms first.")

    message = str(excinfo.value)
    assert "HF_TOKEN" in message
    assert ".env" in message
    assert "Accept the terms first." in message


def test_require_env_treats_an_empty_value_as_missing(monkeypatch) -> None:
    monkeypatch.setenv("HF_TOKEN", "   ")

    with pytest.raises(SystemExit):
        require_env("HF_TOKEN")
