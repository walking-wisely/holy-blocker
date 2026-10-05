"""Report and reap stranded git worktrees.

The working rhythm is worktree-in, PR-out, but nothing reaped the worktree on
the way out, so finished branches piled up. `report` classifies every worktree
by its branch's fate from git and gh facts (``tools.plan.state``). Removal is
gated by the worktree's contents instead (``tools.plan.inventory``): the owner
reads the inventory and names what to remove.

    python -m tools.plan.worktrees report
    python -m tools.plan.worktrees inventory            # what a removal would lose
    python -m tools.plan.worktrees inventory --summary  # one offline line
    python -m tools.plan.worktrees reap --only BRANCH...         # dry run
    python -m tools.plan.worktrees reap --only BRANCH... --yes
    python -m tools.plan.worktrees reap --only BRANCH... --yes --discard

Local branches are never deleted: `git worktree remove` leaves them, and that
call stays with a human. Remote branches are swept by ``tools.plan.branches``.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from dataclasses import replace

from tools.plan import branches, inventory, state


def build_rows(cwd: str, base: str) -> list[state.Row]:
    rows: list[state.Row] = []
    for worktree in state.list_worktrees(cwd):
        if worktree.prunable:
            rows.append(
                state.Row(worktree.path, worktree.branch, "prunable", "directory is gone; `git worktree prune` clears it")
            )
            continue
        dirty = state.is_dirty(worktree.path)
        ignored_state = not dirty and not state.is_disposable_ignored(state.ignored_entries(worktree.path))
        ahead = 0
        residual: list[str] | None = []
        if worktree.branch is None:
            classified = state.classify(worktree, None, 0, 0, dirty)
        else:
            ahead = state.count_between(base, worktree.branch, cwd)
            local_only = state.count_local_only(worktree.branch, cwd)
            try:
                found = state.pr_for_branch(worktree.branch, cwd)
                if found and found.base != base and state.reaches_base(
                    found, base, lambda branch: state.pr_for_branch(branch, cwd)
                ):
                    found = replace(found, base=base)
                pull_request = state.matching_pr(worktree, found, base)
            except (subprocess.CalledProcessError, FileNotFoundError):
                rows.append(state.Row(worktree.path, worktree.branch, "unknown", "gh could not report PR state"))
                continue
            residual = state.residual_files(base, worktree.branch, cwd) if ahead else []
            classified = state.classify(worktree, pull_request, ahead, local_only, dirty, landed=residual == [])
        reason = classified.reason
        if not state.reapable(classified) and ahead:
            reason += "; merge conflicts" if residual is None else f"; merge would change {len(residual)} file(s)"
        rows.append(state.Row(worktree.path, worktree.branch, classified.verdict, reason, ignored_state))
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


def _inventories(cwd: str, base: str, light: bool) -> list[inventory.Inventory]:
    rows = []
    for tree in state.list_worktrees(cwd):
        try:
            rows.append(inventory.build(tree, base, cwd, light=light))
        except (subprocess.CalledProcessError, OSError) as error:
            print(f"cannot read {tree.path}: {error}", file=sys.stderr)
    return rows


def _summary(cwd: str, base: str) -> int:
    try:
        rows = _inventories(cwd, base, light=True)
        remote = [name for name, _ in branches.list_remote(cwd) if name != base]
        line = inventory.summary_line(rows, protected_paths(cwd, base), len(remote))
    except (subprocess.CalledProcessError, FileNotFoundError):
        return 0
    if line:
        print(line)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.worktrees")
    parser.add_argument("command", choices=("report", "inventory", "reap"))
    parser.add_argument("--base", default="master")
    parser.add_argument("--only", nargs="+", default=[], metavar="BRANCH", help="reap: worktrees to remove, by branch or path")
    parser.add_argument("--yes", action="store_true", help="actually remove; without it, dry run")
    parser.add_argument("--discard", action="store_true", help="also remove worktrees holding uncommitted or ignored state")
    parser.add_argument("--summary", action="store_true", help="inventory: one offline line")
    args = parser.parse_args(argv)

    cwd = "."
    if args.command == "report":
        _report(build_rows(cwd, args.base))
        return 0
    if args.command == "inventory" and args.summary:
        return _summary(cwd, args.base)

    rows = _inventories(cwd, args.base, light=False)
    protected = protected_paths(cwd, args.base)
    if args.command == "inventory":
        for row in rows:
            if state._real(row.path) not in protected:
                print("\n".join(inventory.format_inventory(row)))
        return 0

    if not args.only:
        print("reap needs --only BRANCH...; nothing is removed in bulk", file=sys.stderr)
        return 2
    chosen, refused = inventory.select(rows, args.only, protected, args.discard)
    for name, why in refused:
        print(f"refused {name}: {why}", file=sys.stderr)
    for row in chosen:
        print("\n".join(inventory.format_inventory(row)))
    if refused:
        return 2
    if not args.yes:
        print(f"\ndry run: {len(chosen)} worktree(s) would be removed; pass --yes to remove", file=sys.stderr)
        return 0
    failed = False
    for row in chosen:
        try:
            inventory.remove(row, cwd, args.discard)
        except (subprocess.CalledProcessError, OSError) as error:
            detail = (getattr(error, "stderr", "") or str(error)).strip()
            print(f"failed {row.path}: {detail}", file=sys.stderr)
            failed = True
            continue
        print(f"removed {row.path}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
