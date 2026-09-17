"""Discover pending steps across all worktrees.

Scans every git worktree for ``steps.toml`` files under ``docs/components/*/``
and ``docs/engineering/``, then reports pending steps with execution context:
which worktree, which branch, the worktree's health state, and the step's
manifest metadata.

State is derived, never stored. This tool reads the filesystem and git/gh
facts at call time and produces a snapshot; there is no cache, no queue file,
and no state file to go stale.

Usage:
    python -m tools.plan.todos               # pending steps only
    python -m tools.plan.todos --all         # all steps (pending + done)
    python -m tools.plan.todos --blockers    # only worktrees with issues
"""

from __future__ import annotations

import argparse
import subprocess
import tomllib
import sys
from dataclasses import dataclass
from pathlib import Path

from tools.plan import ledger, state


ENGINEERING_DIR = "docs/engineering"
COMPONENTS_GLOB = "docs/components/*"


@dataclass(frozen=True)
class Todo:
    step_id: str
    title: str
    status: str
    acceptance: str
    verify: str
    evidence: str
    package: str
    worktree_path: str
    branch: str | None
    worktree_verdict: str
    worktree_reason: str


def discover_manifests(root: Path) -> list[tuple[Path, str]]:
    """All ``steps.toml`` paths under a worktree root, with their package label."""
    found: list[tuple[Path, str]] = []

    eng = root / ENGINEERING_DIR / "steps.toml"
    if eng.exists():
        found.append((eng, "engineering"))

    for p in sorted(root.glob(COMPONENTS_GLOB + "/steps.toml")):
        found.append((p, p.parent.name))

    return found


def load_steps(manifest_path: Path, package: str) -> list[ledger.Step]:
    """Parse a ``steps.toml`` into ``ledger.Step`` instances."""
    try:
        data = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (FileNotFoundError, PermissionError, tomllib.TOMLDecodeError):
        return []
    return [
        ledger.Step(
            id=raw.get("id", f"{package}.unknown"),
            title=raw.get("title", ""),
            status=raw.get("status", "pending"),
            evidence=raw.get("evidence", ""),
            acceptance=raw.get("acceptance", ledger.DEFAULT_ACCEPTANCE),
            depends_on=tuple(raw.get("depends_on", ())),
            verify=raw.get("verify", ""),
        )
        for raw in data.get("step", [])
    ]


def classify_worktree(worktree: state.Worktree, cwd: str, base: str) -> state.Classified:
    """Classify a worktree's state using the same logic the reaper uses."""
    dirty = state.is_dirty(worktree.path)
    if worktree.branch is None:
        return state.classify(worktree, None, 0, 0, dirty)
    ahead = state.count_between(base, worktree.branch, cwd)
    local_only = state.count_local_only(worktree.branch, cwd)
    try:
        pull_request = state.matching_pr(
            worktree, state.pr_for_branch(worktree.branch, cwd), base
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        pull_request = None
    return state.classify(worktree, pull_request, ahead, local_only, dirty)


def collect(cwd: str, base: str = "master") -> list[Todo]:
    """Scan every worktree and return all steps with execution context.

    Classifies each worktree once (not once per step/package) to avoid
    N*M gh API calls.
    """
    trees = state.list_worktrees(cwd)

    verdicts: dict[str, state.Classified] = {}
    for wt in trees:
        manifests = discover_manifests(Path(wt.path))
        if not manifests:
            continue
        try:
            verdicts[wt.path] = classify_worktree(wt, cwd, base)
        except Exception as exc:
            verdicts[wt.path] = state.Classified("unknown", str(exc))

    todos: list[Todo] = []
    for wt in trees:
        root = Path(wt.path)
        verdict = verdicts.get(wt.path)
        if verdict is None:
            continue
        for manifest_path, package in discover_manifests(root):
            for step in load_steps(manifest_path, package):
                todos.append(Todo(
                    step_id=step.id,
                    title=step.title,
                    status=step.status,
                    acceptance=step.acceptance,
                    verify=step.verify,
                    evidence=step.evidence,
                    package=package,
                    worktree_path=wt.path,
                    branch=wt.branch,
                    worktree_verdict=verdict.verdict,
                    worktree_reason=verdict.reason,
                ))
    return todos


def format_blocker_flag(verdict: str) -> str:
    if verdict in ("dirty", "local-only", "open"):
        return " ⚠"
    if verdict in ("unknown", "detached"):
        return " ?"
    return ""


def show(todos: list[Todo], show_all: bool, blockers_only: bool) -> None:
    by_tree: dict[str, list[Todo]] = {}
    for todo in todos:
        by_tree.setdefault(todo.worktree_path, []).append(todo)

    if blockers_only:
        by_tree = {
            path: ts for path, ts in by_tree.items()
            if any(t.worktree_verdict in ("dirty", "local-only", "open", "unknown", "detached") for t in ts)
        }

    has_output = False
    for path in sorted(by_tree):
        ts = by_tree[path]
        branch = ts[0].branch or "(detached)"
        verdict = ts[0].worktree_verdict
        reason = ts[0].worktree_reason
        flag = format_blocker_flag(verdict)

        basename = Path(path).name
        label = f"{basename} ({branch})" if basename != path else branch
        print(f"── {label}{flag} ──")
        print(f"   state: {verdict} — {reason}")

        pending = [t for t in ts if t.status == "pending"]
        if not pending and not show_all:
            print("   (no pending steps)")
        else:
            display = ts if show_all else pending
            for t in display:
                pad = "     "
                print(f"{pad}{'⬜' if t.status == 'pending' else '✓'} {t.step_id}")
                if t.title:
                    print(f"{pad}  {t.title}")
            has_output = True
        print()

    if not has_output and blockers_only:
        print("no worktrees with blockers")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.todos")
    parser.add_argument("--all", action="store_true", help="show all steps, not just pending")
    parser.add_argument("--blockers", action="store_true", help="only worktrees in a blocking state")
    parser.add_argument("--base", default="master", help="base branch for ahead/behind count")
    args = parser.parse_args(argv)

    try:
        todos = collect(cwd=".", base=args.base)
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    show(todos, show_all=args.all, blockers_only=args.blockers)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())