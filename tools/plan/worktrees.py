"""Report and reap stranded git worktrees.

The working rhythm is worktree-in, PR-out, but nothing reaped the worktree on
the way out, so finished branches piled up. This classifies every worktree from
git and gh facts (``tools.plan.state``) and removes the ones whose work is
already merged or whose branch carries nothing unique.

    python -m tools.plan.worktrees report
    python -m tools.plan.worktrees reap            # dry run
    python -m tools.plan.worktrees reap --yes
    python -m tools.plan.worktrees reap --yes --force   # also removed: ignored state

Branches are never deleted: a squash merge leaves git unable to tell a merged
branch from an unmerged one, so that call stays with a human.
"""

from __future__ import annotations

import argparse
import subprocess
import sys

from tools.plan import state


def build_rows(cwd: str, base: str) -> list[state.Row]:
    rows: list[state.Row] = []
    for worktree in state.list_worktrees(cwd):
        dirty = state.is_dirty(worktree.path)
        ignored_state = not dirty and not state.is_disposable_ignored(state.ignored_entries(worktree.path))
        if worktree.branch is None:
            classified = state.classify(worktree, None, 0, 0, dirty)
        else:
            ahead = state.count_between(base, worktree.branch, cwd)
            local_only = state.count_local_only(worktree.branch, cwd)
            try:
                pull_request = state.matching_pr(
                    worktree, state.pr_for_branch(worktree.branch, cwd), base
                )
            except (subprocess.CalledProcessError, FileNotFoundError):
                rows.append(state.Row(worktree.path, worktree.branch, "unknown", "gh could not report PR state"))
                continue
            classified = state.classify(worktree, pull_request, ahead, local_only, dirty)
        rows.append(
            state.Row(worktree.path, worktree.branch, classified.verdict, classified.reason, ignored_state)
        )
    return rows


def protected_paths(cwd: str, base: str) -> set[str]:
    """Real paths the reaper must never remove: the main worktree, the one we
    are running in, and whatever carries the base branch."""
    protected = set()
    trees = state.list_worktrees(cwd)
    if trees:
        protected.add(trees[0].path)
    for tree in trees:
        if tree.branch == base:
            protected.add(tree.path)
    current = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], cwd=cwd, capture_output=True, text=True
    ).stdout.strip()
    if current:
        protected.add(current)
    return {state._real(path) for path in protected}


def _report(rows: list[state.Row]) -> None:
    for row in rows:
        branch = row.branch or "(detached)"
        marker = " [ignored state]" if row.ignored_state else ""
        print(f"{row.verdict:<10} {branch:<45} {row.reason}{marker}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.worktrees")
    parser.add_argument("command", choices=("report", "reap"))
    parser.add_argument("--base", default="master")
    parser.add_argument("--yes", action="store_true", help="actually remove; without it, dry run")
    parser.add_argument("--force", action="store_true", help="also remove worktrees with ignored state")
    args = parser.parse_args(argv)

    cwd = "."
    rows = build_rows(cwd, args.base)
    if args.command == "report":
        _report(rows)
        return 0

    doomed = state.select_reapable(rows, protected_paths(cwd, args.base), force=args.force)
    if not doomed:
        print("nothing to reap")
        return 0
    for row in doomed:
        marker = " [ignored state]" if row.ignored_state else ""
        print(f"{row.verdict:<10} {row.branch or '(detached)'}{marker}")
        print(f"           {row.path}")
    if not args.yes:
        print(f"\ndry run: {len(doomed)} worktree(s) would be removed; pass --yes to remove", file=sys.stderr)
        return 0
    for row in doomed:
        subprocess.run(["git", "worktree", "remove", row.path], cwd=cwd, check=True)
        print(f"removed {row.path}")
    subprocess.run(["git", "worktree", "prune"], cwd=cwd, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
