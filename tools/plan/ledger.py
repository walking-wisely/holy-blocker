"""Thin step ledger for docs/components/<package>/plan.md.

A package's steps.toml is the source of truth for a step's status; plan.md stays
prose and carries one ``<!-- step: id -->`` marker per step so the two cannot
drift. Narrative, gotchas, and war stories belong in plan.md, never here.

Usage:
    python -m tools.plan.ledger validate docs/components/text-policy
    python -m tools.plan.ledger render   docs/components/text-policy
    python -m tools.plan.ledger next     docs/components/text-policy
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

VALID_STATUS = ("pending", "done")
# What kind of claim does `done` make? A `code` step is done when the diff exists and
# its checks pass; an `observation` step is done only once the world has been seen to
# change. The loop may merge the former on green and must stop at a PR for the latter.
# Default is the safe one: never auto-merge a step unless it says it is code.
VALID_ACCEPTANCE = ("code", "observation")
DEFAULT_ACCEPTANCE = "observation"
MARKER_RE = re.compile(r"<!--\s*step:\s*([A-Za-z0-9._-]+)\s*-->")


@dataclass(frozen=True)
class Step:
    id: str
    title: str
    status: str
    evidence: str
    acceptance: str = DEFAULT_ACCEPTANCE
    depends_on: tuple[str, ...] = field(default_factory=tuple)
    verify: str = ""


def load_manifest(package_dir: Path) -> list[Step]:
    path = Path(package_dir) / "steps.toml"
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return [
        Step(
            id=raw["id"],
            title=raw.get("title", ""),
            status=raw["status"],
            evidence=raw.get("evidence", ""),
            acceptance=raw.get("acceptance", DEFAULT_ACCEPTANCE),
            depends_on=tuple(raw.get("depends_on", ())),
            verify=raw.get("verify", ""),
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
        if step.acceptance not in VALID_ACCEPTANCE:
            problems.append(f"{step.id}: invalid acceptance {step.acceptance!r}")
    problems.extend(_dependency_problems(steps))

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


def _dependency_problems(steps: list[Step]) -> list[str]:
    problems: list[str] = []
    by_id = {step.id: step for step in steps}
    done = {step.id for step in steps if step.status == "done"}
    for step in steps:
        for dep in step.depends_on:
            if dep == step.id:
                problems.append(f"{step.id}: depends on itself")
            elif dep not in by_id:
                problems.append(f"{step.id}: unknown dependency {dep!r}")
            elif step.status == "done" and dep not in done:
                problems.append(f"{step.id}: done but dependency {dep!r} is not")
    for cycle in _cycles(by_id):
        problems.append("dependency cycle: " + " -> ".join(cycle))
    return problems


def _cycles(by_id: dict[str, Step]) -> list[list[str]]:
    found: list[list[str]] = []
    WHITE, GREY, BLACK = 0, 1, 2
    colour = {step_id: WHITE for step_id in by_id}

    def visit(node: str, stack: list[str]) -> None:
        colour[node] = GREY
        stack.append(node)
        for dep in by_id[node].depends_on:
            if dep not in by_id:
                continue
            if colour[dep] == GREY:
                found.append(stack[stack.index(dep):] + [dep])
            elif colour[dep] == WHITE:
                visit(dep, stack)
        stack.pop()
        colour[node] = BLACK

    for step_id in by_id:
        if colour[step_id] == WHITE:
            visit(step_id, [])
    return found


def next_step(steps: list[Step]) -> Step | None:
    """The first pending step whose dependencies are all done, in manifest order."""
    return next_plan(steps)[0]


def next_plan(steps: list[Step]) -> tuple[Step | None, str]:
    """The next actionable step, or a reason there is none.

    "All done" and "pending but blocked" are different answers: a dangling or
    mistyped dependency must not read as a finished plan.
    """
    done = {step.id for step in steps if step.status == "done"}
    pending = [step for step in steps if step.status == "pending"]
    if not pending:
        return None, "all steps are done"
    for step in pending:
        if all(dep in done for dep in step.depends_on):
            return step, ""
    blockers = [
        f"{step.id} waits on {', '.join(dep for dep in step.depends_on if dep not in done)}"
        for step in pending
    ]
    return None, "blocked: " + "; ".join(blockers)


def render(steps: list[Step]) -> str:
    lines = ["| Step | Status | Evidence |", "|---|---|---|"]
    for step in steps:
        lines.append(f"| `{step.id}` | {step.status} | {step.evidence} |")
    return "\n".join(lines)


def _packages(paths: list[str]) -> tuple[list[Path], list[Path], list[Path]]:
    """Classify path arguments into validated, plan-only, and missing.

    A path that is not a directory is reported as missing rather than dropped:
    a shell that failed to expand a glob, or a caller that passed argv without a
    shell, must fail loudly instead of silently validating nothing.
    """
    selected: list[Path] = []
    skipped: list[Path] = []
    missing: list[Path] = []
    for raw in paths:
        path = Path(raw)
        if not path.is_dir():
            missing.append(path)
            continue
        if (path / "steps.toml").exists():
            selected.append(path)
        elif (path / "plan.md").exists():
            skipped.append(path)
    return selected, skipped, missing


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.ledger")
    parser.add_argument("command", choices=("validate", "render", "next"))
    parser.add_argument("packages", nargs="+")
    args = parser.parse_args(argv)

    packages, skipped, missing = _packages(args.packages)
    for path in missing:
        print(f"no such directory: {path}", file=sys.stderr)
    if not packages:
        print("no steps.toml found in the given paths", file=sys.stderr)
        return 1
    for package in skipped:
        print(f"skipped, no steps.toml: {package}", file=sys.stderr)

    failures = 0
    for package in packages:
        steps = load_manifest(package)
        if args.command == "next":
            step, reason = next_plan(steps)
            print(f"### {package.name}\n")
            if step is None:
                print(f"{reason}\n")
            else:
                print(f"`{step.id}` — {step.title}")
                print(f"acceptance: {step.acceptance}")
                if step.verify:
                    print(f"verify: {step.verify}")
                print()
            continue
        plan = (package / "plan.md").read_text(encoding="utf-8")
        problems = validate(steps, extract_markers(plan))
        if problems:
            failures += 1
            for problem in problems:
                print(f"{package}: {problem}", file=sys.stderr)
        if args.command == "render":
            print(f"### {package.name}\n")
            print(render(steps))
            print()
    return 1 if failures or missing else 0


if __name__ == "__main__":
    raise SystemExit(main())