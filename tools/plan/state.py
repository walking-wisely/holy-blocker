"""Derived worktree state from git and gh facts.

A worktree's state is a function of its branch, that branch's pull request, how
many commits it carries ahead of the base, and whether its tree is dirty. None
of it is stored: a written status goes stale and forks per branch, which is the
failure this module exists to avoid. Pure classification is testable; the git
and gh calls are thin edges at the bottom.

The reaper acts on this state, so it errs toward keeping a worktree. A `gh`
lookup that fails is `unknown`, never "no PR"; a PR only counts for a branch if
it targets the base and its head still matches the local tip, so a reused branch
name cannot inherit an old merge.
"""

from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass


@dataclass(frozen=True)
class Worktree:
    path: str
    head: str
    branch: str | None


@dataclass(frozen=True)
class PullRequest:
    number: int
    state: str
    checks: str
    head_oid: str
    base: str


@dataclass(frozen=True)
class Classified:
    verdict: str
    reason: str


@dataclass(frozen=True)
class Row:
    path: str
    branch: str | None
    verdict: str
    reason: str
    ignored_state: bool = False


REAPABLE = ("merged", "abandoned")
# Ignored paths under these top-level names are build output or package caches:
# regenerable, so removing them with the worktree is fine. Anything else ignored
# (a `.env`, a local database) is state, and the reaper leaves it alone.
DISPOSABLE_IGNORED = frozenset(
    {
        "node_modules", "target", "dist", "build", "out", ".turbo",
        "__pycache__", ".venv", ".husky", ".ffi", ".pytest_cache", ".swiftpm",
    }
)


def parse_worktrees(text: str) -> list[Worktree]:
    out: list[Worktree] = []
    for block in text.strip().split("\n\n"):
        path = head = None
        branch: str | None = None
        for line in block.splitlines():
            if line.startswith("worktree "):
                path = line[len("worktree "):]
            elif line.startswith("HEAD "):
                head = line[len("HEAD "):]
            elif line.startswith("branch refs/heads/"):
                branch = line[len("branch refs/heads/"):]
            elif line.startswith("detached"):
                branch = None
        if path is not None:
            out.append(Worktree(path=path, head=head or "", branch=branch))
    return out


def parse_ignored(text: str) -> list[str]:
    paths: list[str] = []
    for line in text.splitlines():
        if line.startswith("!! "):
            paths.append(line[len("!! "):].strip())
    return paths


def is_disposable_ignored(paths: list[str]) -> bool:
    if not paths:
        return True
    for path in paths:
        parts = [part for part in path.strip().strip("/").split("/") if part]
        if not any(part in DISPOSABLE_IGNORED for part in parts):
            return False
    return True


def summarise_checks(rollup: list[dict]) -> str:
    if not rollup:
        return "none"
    pending = failing = False
    for check in rollup:
        conclusion = (check.get("conclusion") or check.get("state") or "").upper()
        status = (check.get("status") or "").upper()
        if conclusion in ("FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED"):
            failing = True
        elif conclusion in ("SUCCESS", "NEUTRAL", "SKIPPED"):
            continue
        elif status and status != "COMPLETED":
            pending = True
        else:
            pending = True
    if failing:
        return "failing"
    if pending:
        return "pending"
    return "passing"


def matching_pr(worktree: Worktree, pull_request: PullRequest | None, base: str) -> PullRequest | None:
    """The PR that actually covers this worktree's tip, or None.

    A branch name can be reused, so a `MERGED` PR for the name is not enough:
    its head must still equal the local tip, and it must target the base we are
    counting against. Otherwise the name's old merge would mark new, unlanded
    commits as landed.
    """
    if pull_request is None:
        return None
    if pull_request.head_oid and pull_request.head_oid != worktree.head:
        return None
    if pull_request.base and pull_request.base != base:
        return None
    return pull_request


def classify(
    worktree: Worktree,
    pull_request: PullRequest | None,
    ahead: int,
    local_only: int,
    dirty: bool,
) -> Classified:
    if worktree.branch is None:
        return Classified("detached", "detached HEAD; no branch to reap by")
    if dirty:
        return Classified("dirty", "uncommitted changes in the worktree")
    if local_only:
        return Classified("local-only", "commits not on any remote; push or delete deliberately")
    if pull_request is not None:
        if pull_request.state == "MERGED":
            return Classified("merged", "PR merged and nothing is local-only")
        if pull_request.state == "OPEN":
            return Classified("open", f"PR #{pull_request.number} open")
        if ahead:
            return Classified("closed", "PR closed without merging; branch carries unique commits")
        return Classified("abandoned", "PR closed without merging; branch carries no unique commits")
    if ahead:
        return Classified("unpushed", "no PR and commits not on the base")
    return Classified("abandoned", "no PR and no unique commits; branch is spent")


def reapable(classified: Classified) -> bool:
    return classified.verdict in REAPABLE


def select_reapable(rows: list[Row], protected: set[str], force: bool = False) -> list[Row]:
    """Rows safe to remove: a reapable verdict, never a protected path, and no
    non-disposable ignored state unless forced."""
    return [
        row
        for row in rows
        if row.verdict in REAPABLE
        and _real(row.path) not in protected
        and (force or not row.ignored_state)
    ]


def _real(path: str) -> str:
    return os.path.realpath(path)


def parse_pr(payload: str) -> PullRequest | None:
    data = json.loads(payload)
    if not data:
        return None
    first = data[0]
    return PullRequest(
        number=first["number"],
        state=first["state"],
        checks=summarise_checks(first.get("statusCheckRollup") or []),
        head_oid=first.get("headRefOid", ""),
        base=first.get("baseRefName", ""),
    )


def _git(args: list[str], cwd: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


def list_worktrees(cwd: str) -> list[Worktree]:
    return parse_worktrees(_git(["worktree", "list", "--porcelain"], cwd))


def is_dirty(path: str) -> bool:
    return bool(_git(["status", "--porcelain"], path).strip())


def ignored_entries(path: str) -> list[str]:
    return parse_ignored(_git(["status", "--porcelain", "--ignored"], path))


def count_between(base: str, branch: str, cwd: str) -> int:
    """Commits on `branch` not reachable from `base` — the work it carries."""
    out = _git(["rev-list", "--count", f"{base}..{branch}"], cwd).strip()
    return int(out or "0")


def count_local_only(branch: str, cwd: str) -> int:
    """Commits reachable from `branch` but from no remote — never-pushed work."""
    out = _git(["rev-list", "--count", branch, "--not", "--remotes"], cwd).strip()
    return int(out or "0")


def pr_for_branch(branch: str, cwd: str) -> PullRequest | None:
    """Raise rather than return None on a failed lookup: "no PR" and "could not
    ask" must not look alike, or a reaper would call an unreachable PR merged."""
    payload = subprocess.run(
        [
            "gh", "pr", "list", "--head", branch, "--state", "all", "--limit", "1",
            "--json", "number,state,baseRefName,headRefOid,statusCheckRollup",
        ],
        cwd=cwd, check=True, capture_output=True, text=True,
    ).stdout
    return parse_pr(payload)
