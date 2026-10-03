"""Fail when a cargo-mutants exclusion has no row in the claim's known-survivors table."""

import sys
import tomllib
from pathlib import Path


def missing_excludes(config: str, claim: str) -> list[str]:
    patterns = tomllib.loads(config).get("exclude_re", [])
    return [p for p in patterns if f"`{p}`" not in claim]


def main(argv: list[str]) -> int:
    config, claim = (Path(a).read_text() for a in argv[1:3])
    missing = missing_excludes(config, claim)
    for pattern in missing:
        print(f"exclude_re not listed in {argv[2]}: {pattern}")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
