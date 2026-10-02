"""Thin step ledger for docs/components/<package>/plan.md.

A package keeps one file per step under steps/<step-id>.toml, plus package.toml for
the code roots it governs. A step file is the only record of a step's status; plan.md
stays prose, carries one ``<!-- step: id -->`` marker per step so the two cannot
drift, and fixes the order steps are offered in. Narrative, gotchas, and war stories
belong in plan.md, never in a step file.

Usage:
    python -m tools.plan.ledger validate docs/components/text-policy
    python -m tools.plan.ledger render   docs/components/text-policy
    python -m tools.plan.ledger next     docs/components/text-policy
    python -m tools.plan.ledger migrate  docs/components/text-policy
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

VALID_STATUS = ("pending", "at-pr", "done")
# `pending` — not settled. `at-pr` — settled through every loop gate, PR open, not
# yet on master; it is not actionable and does not satisfy dependencies. `done` —
# the step's implementation is on master (see the package.toml header); observationally
# verified routes live in coverage.md, never in a status.
# What kind of claim does `done` make? A `code` step is done when the diff exists and
# its checks pass; an `observation` step is done only once the world has been seen to
# change; a `product` step is done only once the user's verdict has closed it. The
# loop may merge the former on green and must stop at a PR for the latter two.
VALID_ACCEPTANCE = ("code", "observation", "product")
DEFAULT_ACCEPTANCE = "observation"
# `feature` — a planned capability. `bug` — a defect found; a `bug` may name the step
# it regressed (`regressed_step`) or stand alone when found by manual/exploratory testing.
# Unknown/misspelled `kind` keys silently default to `feature` like every other optional
# manifest field — a lost bug declaration is indistinguishable from none (recorded narrowing).
VALID_KIND = ("feature", "bug")
DEFAULT_KIND = "feature"
# `hard` exempts a step from the context-budget gate in step-loop: it may run past the
# token threshold instead of stopping for a handoff.
VALID_DIFFICULTY = ("normal", "hard")
DEFAULT_DIFFICULTY = "normal"
MARKER_RE = re.compile(r"<!--\s*step:\s*([A-Za-z0-9._-]+)\s*-->")
SAFE_ID_RE = re.compile(r"[A-Za-z0-9._-]+")
STEPS_DIR = "steps"
PACKAGE_FILE = "package.toml"
LEGACY_FILE = "steps.toml"
STEP_KEYS = (
    "id",
    "title",
    "status",
    "evidence",
    "acceptance",
    "depends_on",
    "verify",
    "kind",
    "regressed_step",
    "difficulty",
)


@dataclass(frozen=True)
class Step:
    id: str
    title: str
    status: str
    evidence: str
    acceptance: str = DEFAULT_ACCEPTANCE
    depends_on: tuple[str, ...] = field(default_factory=tuple)
    verify: str = ""
    kind: str = DEFAULT_KIND
    regressed_step: str = ""
    difficulty: str = DEFAULT_DIFFICULTY


def _step_from_raw(raw: dict) -> Step:
    return Step(
        id=raw["id"],
        title=raw.get("title", ""),
        status=raw["status"],
        evidence=raw.get("evidence", ""),
        acceptance=raw.get("acceptance", DEFAULT_ACCEPTANCE),
        depends_on=tuple(raw.get("depends_on", ())),
        verify=raw.get("verify", ""),
        kind=raw.get("kind", DEFAULT_KIND),
        regressed_step=raw.get("regressed_step", ""),
        difficulty=raw.get("difficulty", DEFAULT_DIFFICULTY),
    )


def _read_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def has_manifest(package_dir: Path) -> bool:
    package_dir = Path(package_dir)
    return (package_dir / STEPS_DIR).is_dir() or (package_dir / LEGACY_FILE).exists()


def discover(root: Path) -> list[Path]:
    """Package directories under a repository root that carry a step manifest."""
    root = Path(root)
    candidates = sorted((root / "docs" / "components").glob("*/"))
    candidates.append(root / "docs" / "engineering")
    return [path for path in candidates if path.is_dir() and has_manifest(path)]


def load_manifest(package_dir: Path) -> list[Step]:
    package_dir = Path(package_dir)
    steps_dir = package_dir / STEPS_DIR
    if not steps_dir.is_dir():
        data = _read_toml(package_dir / LEGACY_FILE)
        return [_step_from_raw(raw) for raw in data.get("step", [])]
    steps = [_step_from_raw(_read_toml(path)) for path in sorted(steps_dir.glob("*.toml"))]
    return _in_plan_order(steps, package_dir / "plan.md")


def _in_plan_order(steps: list[Step], plan: Path) -> list[Step]:
    markers = extract_markers(plan.read_text(encoding="utf-8")) if plan.exists() else []
    position = {marker: index for index, marker in reversed(list(enumerate(markers)))}
    return sorted(steps, key=lambda step: (position.get(step.id, len(markers)), step.id))


def load_paths(package_dir: Path) -> tuple[str, ...]:
    """Code roots a manifest governs; empty when the package has no code yet."""
    package_dir = Path(package_dir)
    path = package_dir / PACKAGE_FILE
    if not path.exists():
        path = package_dir / LEGACY_FILE
    if not path.exists():
        return ()
    return tuple(_read_toml(path).get("paths", ()))


def layout_problems(package_dir: Path) -> list[str]:
    """Faults in the file layout that `validate` cannot see from parsed steps."""
    package_dir = Path(package_dir)
    problems: list[str] = []
    if (package_dir / LEGACY_FILE).exists():
        problems.append(f"legacy {LEGACY_FILE}: run `python -m tools.plan.ledger migrate {package_dir}`")
    package_file = package_dir / PACKAGE_FILE
    if package_file.exists():
        try:
            package = _read_toml(package_file)
        except (tomllib.TOMLDecodeError, UnicodeDecodeError) as err:
            problems.append(f"{PACKAGE_FILE}: invalid TOML: {err}")
        else:
            for key in sorted(set(package) - {"paths"}):
                problems.append(f"{PACKAGE_FILE}: unknown key {key!r}")
            if not _is_string_list(package.get("paths", [])):
                problems.append(f"{PACKAGE_FILE}: paths must be a list of strings")
    steps_dir = package_dir / STEPS_DIR
    if not steps_dir.is_dir():
        return problems
    for path in sorted(steps_dir.iterdir()):
        if path.is_symlink():
            problems.append(f"{path.name}: symlink in {STEPS_DIR}/")
            continue
        if path.is_dir() or path.suffix != ".toml":
            problems.append(f"{path.name}: not a step file; {STEPS_DIR}/ holds only <id>.toml")
            continue
        try:
            raw = _read_toml(path)
        except (tomllib.TOMLDecodeError, UnicodeDecodeError) as err:
            problems.append(f"{path.name}: invalid TOML: {err}")
            continue
        for key in sorted(set(raw) - set(STEP_KEYS)):
            problems.append(f"{path.name}: unknown key {key!r}")
        if not _is_string_list(raw.get("depends_on", [])):
            problems.append(f"{path.name}: depends_on must be a list of strings")
        if "id" not in raw:
            problems.append(f"{path.name}: no id")
        elif raw["id"] != path.stem:
            problems.append(f"{path.name}: file name does not match id {raw['id']!r}")
    return problems


def _is_string_list(value: object) -> bool:
    return isinstance(value, list) and all(isinstance(item, str) for item in value)


def _toml_value(value: object) -> str:
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(_toml_value(item) for item in value) + "]"
    return json.dumps(value, ensure_ascii=False).replace("\x7f", "\\u007f")


def migrate(package_dir: Path) -> list[Path]:
    """Convert a legacy steps.toml into per-step files and package.toml.

    Everything is rendered and re-parsed before any file is written, so a step that
    would not round-trip aborts the migration with the legacy file untouched.
    """
    package_dir = Path(package_dir)
    legacy = package_dir / LEGACY_FILE
    text = legacy.read_text(encoding="utf-8")
    data = tomllib.loads(text)
    raws = data.get("step", [])
    ids = [raw["id"] for raw in raws]
    unsafe = [i for i in ids if not SAFE_ID_RE.fullmatch(i) or i in (".", "..")]
    if unsafe:
        raise ValueError(f"step ids that are not safe file names: {', '.join(unsafe)}")
    folded = [i.casefold() for i in ids]
    repeated = sorted({i for i, f in zip(ids, folded) if folded.count(f) > 1})
    if repeated:
        raise ValueError(f"duplicate step ids, ignoring case: {', '.join(repeated)}")

    rendered: list[tuple[Path, str]] = []
    steps_dir = package_dir / STEPS_DIR
    for raw in raws:
        expected = {key: raw[key] for key in STEP_KEYS if key in raw}
        try:
            body = "".join(f"{key} = {_toml_value(value)}\n" for key, value in expected.items())
            parsed = tomllib.loads(body)
        except (TypeError, tomllib.TOMLDecodeError) as err:
            raise ValueError(f"step {raw['id']!r} would not round-trip: {err}") from err
        if parsed != expected:
            raise ValueError(f"step {raw['id']!r} would not round-trip")
        rendered.append((steps_dir / f"{raw['id']}.toml", body))

    header: list[str] = []
    for line in text.splitlines():
        if not line.startswith("#") and line.strip():
            break
        header.append(line)
    while header and not header[-1].strip():
        header.pop()
    body = ""
    if data.get("paths"):
        items = "".join(f"  {json.dumps(path)},\n" for path in data["paths"])
        body = f"paths = [\n{items}]\n"
        if tomllib.loads(body).get("paths") != data["paths"]:
            raise ValueError("paths would not round-trip")
    package_text = "\n".join(header) + ("\n\n" if header and body else "\n" if header else "") + body

    clashes = [path for path, _ in rendered if path.exists()]
    if clashes:
        raise FileExistsError(", ".join(str(path) for path in clashes))
    steps_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for path, content in rendered:
        path.write_text(content, encoding="utf-8")
        written.append(path)
    if package_text:
        (package_dir / PACKAGE_FILE).write_text(package_text, encoding="utf-8")
        written.append(package_dir / PACKAGE_FILE)
    legacy.unlink()
    return written


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
        if step.kind not in VALID_KIND:
            problems.append(f"{step.id}: invalid kind {step.kind!r}")
        if step.difficulty not in VALID_DIFFICULTY:
            problems.append(f"{step.id}: invalid difficulty {step.difficulty!r}")
        if step.regressed_step and step.regressed_step not in ids:
            problems.append(f"{step.id}: unknown regressed_step {step.regressed_step!r}")
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

    "All done", "settled but unmerged", and "pending but blocked" are different
    answers: a dangling or mistyped dependency must not read as a finished plan,
    and a step whose branch is open at a PR must not be offered again or read as
    a satisfied dependency.
    """
    done = {step.id for step in steps if step.status == "done"}
    pending = [step for step in steps if step.status == "pending"]
    if not pending:
        awaiting = [step.id for step in steps if step.status == "at-pr"]
        if not awaiting:
            return None, "all steps are done"
        return None, "no pending steps; awaiting merge for " + ", ".join(awaiting)
    for step in pending:
        if all(dep in done for dep in step.depends_on):
            return step, ""
    blockers = [
        f"{step.id} waits on {', '.join(dep for dep in step.depends_on if dep not in done)}"
        for step in pending
    ]
    return None, "blocked: " + "; ".join(blockers)


def render(steps: list[Step]) -> str:
    lines = ["| Step | Kind | Status | Evidence |", "|---|---|---|---|"]
    for step in steps:
        lines.append(f"| `{step.id}` | {step.kind} | {step.status} | {step.evidence} |")
    return "\n".join(lines)


def write_todo(package: Path, steps: list[Step]) -> Path:
    """Render steps to <package>/TODO.md — a gitignored, human-readable view.

    The step files stay the source of truth scripts read; this file exists only so
    a person can glance at status without parsing TOML. Regenerated on every
    render --write, never hand-edited.
    """
    path = Path(package) / "TODO.md"
    body = (
        f"<!-- Generated by `python -m tools.plan.ledger render --write {package}`. "
        "Do not edit by hand — edit the step files instead. -->\n\n"
        f"{render(steps)}\n"
    )
    path.write_text(body, encoding="utf-8")
    return path


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
        if has_manifest(path):
            selected.append(path)
        elif (path / "plan.md").exists():
            skipped.append(path)
    return selected, skipped, missing


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.ledger")
    parser.add_argument("command", choices=("validate", "render", "next", "migrate"))
    parser.add_argument(
        "--write",
        action="store_true",
        help="render: write <package>/TODO.md instead of printing to stdout",
    )
    parser.add_argument("packages", nargs="+")
    args = parser.parse_args(argv)

    packages, skipped, missing = _packages(args.packages)
    for path in missing:
        print(f"no such directory: {path}", file=sys.stderr)
    if not packages:
        print("no step manifest found in the given paths", file=sys.stderr)
        return 1
    for package in skipped:
        print(f"skipped, no step manifest: {package}", file=sys.stderr)

    failures = 0
    for package in packages:
        if args.command == "migrate":
            if not (package / LEGACY_FILE).exists():
                print(f"nothing to migrate: {package}", file=sys.stderr)
                continue
            try:
                written = migrate(package)
            except (ValueError, FileExistsError) as err:
                print(f"{package}: cannot migrate: {err}", file=sys.stderr)
                failures += 1
                continue
            for path in written:
                print(path)
            continue
        steps = load_manifest(package)
        if args.command == "next":
            step, reason = next_plan(steps)
            print(f"### {package.name}\n")
            if step is None:
                print(f"{reason}\n")
            else:
                print(f"`{step.id}` — {step.title}")
                print(f"acceptance: {step.acceptance}")
                if step.difficulty != DEFAULT_DIFFICULTY:
                    print(f"difficulty: {step.difficulty}")
                if step.verify:
                    print(f"verify: {step.verify}")
                print()
            continue
        plan = (package / "plan.md").read_text(encoding="utf-8")
        problems = layout_problems(package) + validate(steps, extract_markers(plan))
        if problems:
            failures += 1
            for problem in problems:
                print(f"{package}: {problem}", file=sys.stderr)
        if args.command == "render":
            if args.write:
                write_todo(package, steps)
            else:
                print(f"### {package.name}\n")
                print(render(steps))
                print()
    return 1 if failures or missing else 0


if __name__ == "__main__":
    raise SystemExit(main())