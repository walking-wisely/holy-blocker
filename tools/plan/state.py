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
from collections.abc import Callable
from dataclasses import dataclass, replace


@dataclass(frozen=True)
class Worktree:
    path: str
    head: str
    branch: str | None
    prunable: bool = False


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


REAPABLE = ("merged", "abandoned", "landed")
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
        prunable = False
        for line in block.splitlines():
            if line.startswith("worktree "):
                path = line[len("worktree "):]
            elif line.startswith("HEAD "):
                head = line[len("HEAD "):]
            elif line.startswith("branch refs/heads/"):
                branch = line[len("branch refs/heads/"):]
            elif line.startswith("detached"):
                branch = None
            elif line.startswith("prunable"):
                prunable = True
        if path is not None:
            out.append(Worktree(path=path, head=head or "", branch=branch, prunable=prunable))
    return out


def parse_ignored(text: str) -> list[str]:
    paths: list[str] = []
    for line in text.splitlines():
        if line.startswith("!! "):
            paths.append(line[len("!! "):].strip())
    return paths


def non_disposable_ignored(paths: list[str]) -> list[str]:
    kept: list[str] = []
    for path in paths:
        parts = [part for part in path.strip().strip("/").split("/") if part]
        if not any(part in DISPOSABLE_IGNORED for part in parts):
            kept.append(path)
    return kept


def is_disposable_ignored(paths: list[str]) -> bool:
    return not non_disposable_ignored(paths)


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
    commits as landed. An open PR is kept whatever it targets: it is live work.
    """
    if pull_request is None:
        return None
    if pull_request.head_oid and pull_request.head_oid != worktree.head:
        return None
    if pull_request.base and pull_request.base != base and pull_request.state != "OPEN":
        return None
    return pull_request


def reaches_base(pull_request: PullRequest, base: str, lookup: Callable[[str], PullRequest | None]) -> bool:
    """True when the PR, and every stacked PR it targets in turn, merged and the
    chain ends at `base`."""
    seen: set[str] = set()
    current: PullRequest | None = pull_request
    while current is not None and current.state == "MERGED":
        if current.base == base:
            return True
        if current.base in seen:
            return False
        seen.add(current.base)
        current = lookup(current.base)
    return False


def classify(
    worktree: Worktree,
    pull_request: PullRequest | None,
    ahead: int,
    local_only: int,
    dirty: bool,
    landed: bool = False,
) -> Classified:
    if worktree.branch is None:
        return Classified("detached", "detached HEAD; no branch to reap by")
    if dirty:
        return Classified("dirty", "uncommitted changes in the worktree")
    if landed and (pull_request is None or pull_request.state != "OPEN"):
        return Classified("landed", "merging the branch into the base changes nothing")
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


def residual_files(base: str, branch: str, cwd: str) -> list[str] | None:
    """Files a merge of `branch` into `base` would still change; None on conflict."""
    result = subprocess.run(
        ["git", "merge-tree", "--write-tree", "--no-messages", base, branch],
        cwd=cwd, capture_output=True, text=True,
    )
    if result.returncode == 1:
        return None
    result.check_returncode()
    merged = result.stdout.splitlines()[0]
    return _git(["diff", "--name-only", base, merged], cwd).splitlines()


def content_landed(base: str, branch: str, cwd: str) -> bool:
    """True when merging `branch` into `base` would change nothing.

    Unlike ancestry, this survives squash merges, rebases and cherry-picks. A
    conflict, or any change the merge would still make, is not landed.
    """
    return residual_files(base, branch, cwd) == []


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
