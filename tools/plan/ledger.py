"""Thin step ledger for docs/components/<package>/plan.md.

A package's steps.toml is the source of truth for a step's status; plan.md stays
prose and carries one ``<!-- step: id -->`` marker per step so the two cannot
drift. Narrative, gotchas, and war stories belong in plan.md, never here.

Usage:
    python -m tools.plan.ledger validate docs/components/text-policy
    python -m tools.plan.ledger render   docs/components/text-policy
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

VALID_STATUS = ("pending", "done")
MARKER_RE = re.compile(r"<!--\s*step:\s*([A-Za-z0-9._-]+)\s*-->")


@dataclass(frozen=True)
class Step:
    id: str
    title: str
    status: str
    evidence: str


def load_manifest(package_dir: Path) -> list[Step]:
    path = Path(package_dir) / "steps.toml"
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return [
        Step(
            id=raw["id"],
            title=raw.get("title", ""),
            status=raw["status"],
            evidence=raw.get("evidence", ""),
        )
        for raw in data.get("step", [])
    ]


def extract_markers(text: str) -> list[str]:
    return MARKER_RE.findall(text)


def validate(steps: list[Step], markers: list[str]) -> list[str]:
    problems: list[str] = []
    ids = [step.id for step in steps]
    seen: set[str] = set()
    for step in steps:
        if step.id in seen:
            problems.append(f"duplicate manifest id: {step.id}")
        seen.add(step.id)
        if step.status not in VALID_STATUS:
            problems.append(f"{step.id}: invalid status {step.status!r}")
        if step.status == "done" and not step.evidence.strip():
            problems.append(f"{step.id}: done without evidence")

    counts: dict[str, int] = {}
    for marker in markers:
        counts[marker] = counts.get(marker, 0) + 1
    for marker, count in counts.items():
        if count > 1:
            problems.append(f"duplicate marker: {marker}")
        if marker not in seen:
            problems.append(f"marker {marker}: no manifest entry")
    for step in steps:
        if step.id not in counts:
            problems.append(f"{step.id}: no <!-- step: {step.id} --> marker in plan.md")
    return problems


def render(steps: list[Step]) -> str:
    lines = ["| Step | Status | Evidence |", "|---|---|---|"]
    for step in steps:
        lines.append(f"| `{step.id}` | {step.status} | {step.evidence} |")
    return "\n".join(lines)


def _packages(paths: list[str]) -> list[Path]:
    out: list[Path] = []
    for raw in paths:
        path = Path(raw)
        if path.is_dir() and (path / "steps.toml").exists():
            out.append(path)
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.ledger")
    parser.add_argument("command", choices=("validate", "render"))
    parser.add_argument("packages", nargs="+")
    args = parser.parse_args(argv)

    failures = 0
    for package in _packages(args.packages):
        plan = (package / "plan.md").read_text(encoding="utf-8")
        steps = load_manifest(package)
        problems = validate(steps, extract_markers(plan))
        if problems:
            failures += 1
            for problem in problems:
                print(f"{package}: {problem}", file=sys.stderr)
        if args.command == "render":
            print(f"### {package.name}\n")
            print(render(steps))
            print()
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())