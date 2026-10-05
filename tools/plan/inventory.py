"""What a worktree removal would lose, listed before anything is removed.

`git worktree remove` never touches the branch, so the branch's merge state is
not what gates a removal. What a removal destroys is uncommitted and untracked
files, ignored files that are state rather than build output, and commits on a
detached HEAD that no ref holds. Git itself refuses the first and silently
deletes the second, so the ignored-state check lives here.

Unpushed commits on a clean worktree are a warning, not a block: the branch
survives the removal.
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass

from tools.plan import state

REMOVABLE = ("clean", "prunable")
LIST_CAP = 10


@dataclass(frozen=True)
class Inventory:
    path: str
    branch: str | None
    verdict: str
    uncommitted: tuple[str, ...]
    ignored: tuple[str, ...]
    commits: tuple[str, ...]
    local_only: int
    branch_at_risk: bool
    pr: str

    @property
    def name(self) -> str:
        return self.branch or os.path.basename(self.path)


def verdict_for(*, prunable: bool, uncommitted: tuple[str, ...], ignored: tuple[str, ...], orphaned: bool) -> str:
    if prunable:
        return "prunable"
    if orphaned:
        return "orphaned-head"
    if uncommitted:
        return "has-uncommitted"
    if ignored:
        return "has-ignored-state"
    return "clean"


def _git(args: list[str], cwd: str) -> str:
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout


def _held_by_a_ref(head: str, cwd: str) -> bool:
    out = _git(["for-each-ref", "--contains", head, "refs/heads", "refs/remotes"], cwd)
    return bool(out.strip())


def _pr_label(branch: str | None, cwd: str, pr_lookup) -> str:
    if branch is None:
        return "none"
    try:
        found = pr_lookup(branch, cwd)
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"
    return f"#{found.number} {found.state}" if found else "none"


def build(worktree: state.Worktree, base: str, cwd: str, pr_lookup=state.pr_for_branch, light: bool = False) -> Inventory:
    if worktree.prunable:
        return Inventory(worktree.path, worktree.branch, "prunable", (), (), (), 0, False, "none")
    uncommitted = tuple(_git(["status", "--porcelain"], worktree.path).splitlines())
    ignored = tuple(state.non_disposable_ignored(state.ignored_entries(worktree.path)))
    orphaned = worktree.branch is None and not _held_by_a_ref(worktree.head, cwd)
    commits: tuple[str, ...] = ()
    local_only = 0
    at_risk = False
    if worktree.branch is not None:
        commits = tuple(_git(["log", "--format=%h %s", f"{base}..{worktree.branch}"], cwd).splitlines())
        local_only = state.count_local_only(worktree.branch, cwd)
        at_risk = not light and local_only > 0 and not state.content_landed(base, worktree.branch, cwd)
    pr = "n/a" if light else _pr_label(worktree.branch, cwd, pr_lookup)
    verdict = verdict_for(prunable=False, uncommitted=uncommitted, ignored=ignored, orphaned=orphaned)
    return Inventory(worktree.path, worktree.branch, verdict, uncommitted, ignored, commits, local_only, at_risk, pr)


def removable(row: Inventory, discard: bool) -> bool:
    return row.verdict in REMOVABLE or discard


def select(
    rows: list[Inventory], names: list[str], protected: set[str], discard: bool
) -> tuple[list[Inventory], list[tuple[str, str]]]:
    chosen: list[Inventory] = []
    refused: list[tuple[str, str]] = []
    for name in names:
        match = next((r for r in rows if name in (r.branch, r.path, r.name)), None)
        if match is None:
            refused.append((name, "no such worktree"))
        elif state._real(match.path) in protected:
            refused.append((name, "protected: main checkout, current worktree or base branch"))
        elif not removable(match, discard):
            refused.append((name, f"{match.verdict}; pass --discard to remove it anyway"))
        else:
            chosen.append(match)
    return chosen, refused


def summary_line(rows: list[Inventory], protected: set[str], remote_branches: int) -> str:
    own = [r for r in rows if state._real(r.path) not in protected]
    if not own and not remote_branches:
        return ""
    clean = sum(1 for r in own if r.verdict in REMOVABLE)
    blocked = len(own) - clean
    return (
        f"worktrees: {len(own)} worktrees, {clean} removable, {blocked} blocked; "
        f"{remote_branches} remote branches tracked. "
        "Run `python3 -m tools.plan.worktrees inventory` and `python3 -m tools.plan.branches inventory`."
    )


def _capped(items: tuple[str, ...], indent: str) -> list[str]:
    lines = [f"{indent}{item}" for item in items[:LIST_CAP]]
    if len(items) > LIST_CAP:
        lines.append(f"{indent}... {len(items) - LIST_CAP} more")
    return lines


def format_inventory(row: Inventory) -> list[str]:
    lines = [f"{row.verdict:<18} {row.name}  [PR: {row.pr}]", f"                   {row.path}"]
    if row.uncommitted:
        lines.append("  uncommitted:")
        lines += _capped(row.uncommitted, "    ")
    if row.ignored:
        lines.append("  ignored state (not build output):")
        lines += _capped(row.ignored, "    ")
    if row.commits:
        lines.append(f"  {len(row.commits)} commit(s) ahead of base:")
        lines += _capped(row.commits, "    ")
    if row.branch_at_risk:
        lines.append(f"  branch-at-risk: {row.local_only} commit(s) on no remote; the branch survives removal")
    return lines


def remove(row: Inventory, cwd: str, discard: bool) -> None:
    if row.verdict == "prunable":
        _git(["worktree", "prune"], cwd)
        return
    args = ["worktree", "remove"]
    if discard:
        args.append("--force")
    _git([*args, row.path], cwd)
