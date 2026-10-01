"""Machine-checkable PR contract for the step loop.

A PR that changes files governed by a manifest must name a step id and carry
non-empty `## Assumption audit` and `## Adversarial review` sections, so "the
loop ran" is a fact in the PR body.

Usage:
    python -m tools.plan.loop check --body-file BODY --changed-files LIST
    python -m tools.plan.loop check --body-file BODY --base SHA --head SHA
    python -m tools.plan.loop open <step-id>
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

from tools.plan import ledger

REQUIRED_SECTIONS = ("Assumption audit", "Adversarial review")
CODE_ROOTS = ("packages", "apps", "native-modules", "machine-learning")
_FENCE_RE = re.compile(r"^(```|~~~).*?^\1[^\n]*$", re.S | re.M)
_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
_HEADING_RE = re.compile(r"^(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$")


@dataclass(frozen=True)
class Governed:
    package: str
    paths: tuple[str, ...]
    step_ids: tuple[str, ...]


def load_governed(root: Path) -> list[Governed]:
    root = Path(root)
    packages = sorted((root / "docs" / "components").glob("*/steps.toml"))
    packages.append(root / "docs" / "engineering" / "steps.toml")
    governed = []
    for manifest in packages:
        if not manifest.exists():
            continue
        package = manifest.parent
        governed.append(
            Governed(
                package=package.relative_to(root).as_posix(),
                paths=ledger.load_paths(package),
                step_ids=tuple(step.id for step in ledger.load_manifest(package)),
            )
        )
    return governed


def load_governed_at(root: Path, rev: str) -> list[Governed]:
    archive = subprocess.run(
        ["git", "archive", "--end-of-options", rev, "docs"], cwd=root, check=True, capture_output=True
    ).stdout
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(["tar", "-x", "-C", tmp], input=archive, check=True)
        return load_governed(Path(tmp))


def merge_governed(head: list[Governed], base: list[Governed]) -> list[Governed]:
    merged = {m.package: m for m in head}
    for m in base:
        kept = merged.get(m.package)
        if kept is None:
            merged[m.package] = m
            continue
        merged[m.package] = Governed(
            package=m.package,
            paths=tuple(dict.fromkeys(kept.paths + m.paths)),
            step_ids=tuple(dict.fromkeys(kept.step_ids + m.step_ids)),
        )
    return list(merged.values())


def _owns(path: str, changed: str) -> bool:
    prefix = path.strip("/")
    return changed == prefix or changed.startswith(prefix + "/")


def governing(changed_files: list[str], manifests: list[Governed]) -> list[Governed]:
    return [
        m
        for m in manifests
        if any(_owns(p, f) for p in m.paths for f in changed_files)
    ]


def unclaimed(changed_files: list[str], manifests: list[Governed]) -> list[str]:
    owned = [p for m in manifests for p in m.paths]
    return [
        f
        for f in changed_files
        if f.split("/", 1)[0] in CODE_ROOTS and not any(_owns(p, f) for p in owned)
    ]


def _sanitize(body: str) -> str:
    body = body.replace("\r\n", "\n").replace("\r", "\n")
    return _COMMENT_RE.sub("", _FENCE_RE.sub("", body))


def _sections(body: str) -> dict[str, str]:
    sections: dict[str, list[str]] = {}
    current: list[str] | None = None
    for line in body.split("\n"):
        heading = _HEADING_RE.match(line)
        if heading and len(heading.group(1)) <= 2:
            if len(heading.group(1)) == 2:
                current = sections.setdefault(heading.group(2).strip().lower(), [])
            else:
                current = None
        elif current is not None:
            current.append(line)
    return {name: "\n".join(lines).strip() for name, lines in sections.items()}


def _names_step(body: str, manifests: list[Governed]) -> bool:
    for manifest in manifests:
        for step_id in manifest.step_ids:
            if re.search(rf"(?<![\w.-]){re.escape(step_id)}(?![\w.-])", body):
                return True
    return False


def check(body: str, changed_files: list[str], manifests: list[Governed]) -> list[str]:
    problems = [
        f"{f} is under a code root but no steps.toml claims it; add it to a manifest's paths"
        for f in unclaimed(changed_files, manifests)
    ]
    if not governing(changed_files, manifests):
        return problems
    body = _sanitize(body)
    if not _names_step(body, manifests):
        problems.append("PR body names no step id from any steps.toml")
    sections = _sections(body)
    for title in REQUIRED_SECTIONS:
        content = sections.get(title.lower())
        if content is None:
            problems.append(f"PR body has no top-level '## {title}' section")
        elif not content:
            problems.append(f"'## {title}' section is empty; write 'N/A — <reason>' if it does not apply")
    return problems


def render_body(step: ledger.Step) -> str:
    return (
        f"Step: `{step.id}` — {step.title}\n"
        f"Acceptance: {step.acceptance}\n"
        f"Verify: `{step.verify or 'none recorded'}`\n"
        "\n"
        "## Assumption audit\n"
        "<!-- Claim | Falsifier | Observed | Verdict. Replace with the table, or 'N/A — <reason>'. -->\n"
        "\n"
        "## Adversarial review\n"
        "<!-- Findings and how each was reproduced or labelled. Replace, or 'N/A — <reason>'. -->\n"
    )


def _changed_files(args: argparse.Namespace) -> list[str]:
    if args.changed_files:
        text = Path(args.changed_files).read_text(encoding="utf-8")
    else:
        out = subprocess.run(
            [
                "git",
                "diff",
                "-z",
                "--name-only",
                "--no-renames",
                "--end-of-options",
                f"{args.base}...{args.head}",
            ],
            cwd=args.root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        return [path for path in out.split("\0") if path]
    return [line.strip() for line in text.splitlines() if line.strip()]


def _cmd_check(args: argparse.Namespace) -> int:
    try:
        body = Path(args.body_file).read_text(encoding="utf-8")
    except OSError as err:
        print(f"cannot read PR body: {err}", file=sys.stderr)
        return 1
    try:
        changed = _changed_files(args)
    except (OSError, subprocess.CalledProcessError) as err:
        print(f"cannot determine changed files: {err}", file=sys.stderr)
        return 1
    governed = load_governed(args.root)
    if args.base:
        try:
            governed = merge_governed(governed, load_governed_at(args.root, args.base))
        except (OSError, subprocess.CalledProcessError) as err:
            print(f"cannot load base manifests: {err}", file=sys.stderr)
            return 1
    problems = check(body, changed, governed)
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


def _cmd_open(args: argparse.Namespace) -> int:
    for governed in load_governed(args.root):
        for step in ledger.load_manifest(Path(args.root) / governed.package):
            if step.id == args.step_id:
                print(render_body(step), end="")
                return 0
    print(f"no step with id {args.step_id}", file=sys.stderr)
    return 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.loop")
    sub = parser.add_subparsers(dest="command", required=True)

    check_cmd = sub.add_parser("check")
    check_cmd.add_argument("--root", default=".")
    check_cmd.add_argument("--body-file", required=True)
    source = check_cmd.add_mutually_exclusive_group(required=True)
    source.add_argument("--changed-files")
    source.add_argument("--base")
    check_cmd.add_argument("--head", default="HEAD")
    check_cmd.set_defaults(run=_cmd_check)

    open_cmd = sub.add_parser("open")
    open_cmd.add_argument("step_id")
    open_cmd.add_argument("--root", default=".")
    open_cmd.set_defaults(run=_cmd_open)

    args = parser.parse_args(argv)
    return args.run(args)


if __name__ == "__main__":
    raise SystemExit(main())
