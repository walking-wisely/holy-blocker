"""Minimal `.env` loading for dev-time credentials.

Hand-rolled rather than pulling in `python-dotenv`: the parser is ~20 lines and
this package deliberately keeps its dependency surface small. Only the subset
that matters here is supported — `KEY=VALUE`, `#` comments, optional `export`
prefix, optional surrounding quotes. No variable interpolation, no multi-line
values.

The only secret in play is a Hugging Face read token used to fetch a gated
evaluation corpus. Nothing here is needed at runtime by the shipped product.
"""

import os
from pathlib import Path

DOTENV_NAME = ".env"

#: How far up the tree to look, so the file works from the repo root or from
#: `machine-learning/` without needing an absolute path.
SEARCH_DEPTH = 4


def find_dotenv(start: Path | None = None, depth: int = SEARCH_DEPTH) -> Path | None:
    """Walk upward from `start` looking for a `.env`. Returns None if absent."""
    current = (start or Path.cwd()).resolve()
    for _ in range(depth + 1):
        candidate = current / DOTENV_NAME
        if candidate.is_file():
            return candidate
        if current.parent == current:
            break
        current = current.parent
    return None


def parse_dotenv(text: str) -> dict[str, str]:
    """Parse `.env` text into a mapping. Malformed lines are skipped."""
    values: dict[str, str] = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        if line.startswith("export "):
            line = line[len("export ") :]

        key, _, value = line.partition("=")  # partition: keep '=' inside values
        key = key.strip()
        if not key:
            continue

        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        values[key] = value
    return values


def load_dotenv(
    path: Path | None = None,
    apply: bool = True,
    override: bool = False,
) -> dict[str, str]:
    """Read a `.env` and optionally copy it into `os.environ`.

    Existing environment variables win by default — an explicitly exported value
    is a stronger signal of intent than a file sitting in the tree.
    """
    path = path or find_dotenv()
    if path is None or not Path(path).is_file():
        return {}

    values = parse_dotenv(Path(path).read_text())
    if apply:
        for key, value in values.items():
            if override or key not in os.environ:
                os.environ[key] = value
    return values


def require_env(name: str, hint: str = "") -> str:
    """Return an environment variable or exit with instructions.

    Raises SystemExit rather than KeyError so a missing token reads as a setup
    problem in the terminal instead of a stack trace.
    """
    value = os.environ.get(name, "").strip()
    if value:
        return value

    lines = [
        f"{name} is not set.",
        f"Add it to a {DOTENV_NAME} file (searched upward from the working "
        f"directory) or export it in your shell:",
        "",
        f"    echo '{name}=...' >> {DOTENV_NAME}",
        "",
        f"See {DOTENV_NAME}.example for the expected keys.",
    ]
    if hint:
        lines += ["", hint]
    raise SystemExit("\n".join(lines))
