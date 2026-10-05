"""Inventory and delete remote branches that a merged pull request left behind.

A squash merge leaves the branch tip outside the base's history, so ancestry
cannot say a branch landed; the pull request record can. A merged PR still is
not enough: commits pushed after the merge would be lost with the branch, so a
branch counts as `merged` only when its tip equals the PR's head. Deletion
leases on the inventoried tip, so a branch that moved since is left alone.

    python -m tools.plan.branches inventory
    python -m tools.plan.branches reap --only NAME... [--discard] [--yes]

Only `merged` and `ancestor` branches are deletable by default; `--discard`
widens that to branches with unmerged work. A branch with an open PR is never
deleted: GitHub would close the PR.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass

REMOTE = "origin"
DELETABLE = ("merged", "ancestor")
NEVER = ("open",)
PR_LIMIT = 1000
DEFAULT_NAMES = frozenset({"master", "main"})
ORDER = ("merged", "ancestor", "merged-diverged", "merged-other-base", "closed", "none", "open")


@dataclass(frozen=True)
class PrRecord:
    number: int
    state: str
    head: str
    head_oid: str
    base: str


@dataclass(frozen=True)
class RemoteBranch:
    name: str
    tip: str
    verdict: str
    reason: str
    pr: str


def parse_prs(payload: str) -> list[PrRecord]:
    return [
        PrRecord(
            number=item["number"],
            state=item["state"],
            head=item["headRefName"],
            head_oid=item.get("headRefOid", ""),
            base=item.get("baseRefName", ""),
        )
        for item in json.loads(payload)
    ]


def classify(name: str, tip: str, prs: list[PrRecord], base: str, in_base: bool) -> tuple[str, str]:
    own = [p for p in prs if p.head == name]
    open_prs = [p for p in own if p.state == "OPEN"]
    if open_prs:
        return "open", f"PR #{open_prs[0].number} open"
    merged = [p for p in own if p.state == "MERGED"]
    for p in merged:
        if p.base == base and p.head_oid == tip:
            return "merged", f"PR #{p.number} merged and the tip is its head"
    if in_base:
        return "ancestor", "tip is already in the base"
    for p in merged:
        if p.base == base:
            return "merged-diverged", f"PR #{p.number} merged but commits were pushed after it"
    if merged:
        return "merged-other-base", f"PR #{merged[0].number} merged into {merged[0].base}, not {base}"
    closed = [p for p in own if p.state == "CLOSED"]
    if closed:
        return "closed", f"PR #{closed[0].number} closed unmerged; the branch carries unique commits"
    return "none", "no PR and the tip is not in the base"


def select(
    rows: list[RemoteBranch], names: list[str], discard: bool, protected: set[str] | None = None
) -> tuple[list[RemoteBranch], list[tuple[str, str]]]:
    protected = DEFAULT_NAMES | (protected or set())
    chosen: list[RemoteBranch] = []
    refused: list[tuple[str, str]] = []
    for name in names:
        match = next((r for r in rows if r.name == name), None)
        if match is None:
            refused.append((name, "no such remote branch"))
        elif name in protected:
            refused.append((name, "protected: base or default branch"))
        elif match.verdict in NEVER:
            refused.append((name, f"{match.verdict}: deleting it would close its PR"))
        elif match.verdict not in DELETABLE and not discard:
            refused.append((name, f"{match.verdict}; pass --discard to delete it anyway"))
        else:
            chosen.append(match)
    return chosen, refused


def _git(args: list[str], cwd: str) -> str:
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout


def list_remote(cwd: str) -> list[tuple[str, str]]:
    prefix = f"refs/remotes/{REMOTE}/"
    out = _git(["for-each-ref", prefix, "--format=%(refname)\t%(objectname)"], cwd)
    pairs = []
    for line in out.splitlines():
        ref, tip = line.split("\t")
        name = ref[len(prefix):]
        if name != "HEAD":
            pairs.append((name, tip))
    return pairs


def is_in_base(cwd: str, base: str, tip: str) -> bool:
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", tip, f"{REMOTE}/{base}"], cwd=cwd, capture_output=True
    )
    return result.returncode == 0


def fetch_prs(cwd: str) -> list[PrRecord]:
    payload = subprocess.run(
        ["gh", "pr", "list", "--state", "all", "--limit", str(PR_LIMIT),
         "--json", "number,state,headRefName,headRefOid,baseRefName"],
        cwd=cwd, check=True, capture_output=True, text=True,
    ).stdout
    prs = parse_prs(payload)
    if len(prs) >= PR_LIMIT:
        print(f"warning: {PR_LIMIT} PRs fetched; older ones may be missing and read as `none`", file=sys.stderr)
    return prs


def build(cwd: str, base: str, prs: list[PrRecord]) -> list[RemoteBranch]:
    rows = []
    for name, tip in list_remote(cwd):
        if name == base:
            continue
        verdict, reason = classify(name, tip, prs, base, is_in_base(cwd, base, tip))
        own = [p for p in prs if p.head == name]
        rows.append(RemoteBranch(name, tip, verdict, reason, f"#{own[0].number}" if own else "none"))
    return sorted(rows, key=lambda r: (ORDER.index(r.verdict), r.name))


def delete(row: RemoteBranch, cwd: str) -> None:
    ref = f"refs/heads/{row.name}"
    _git(["push", REMOTE, f"--force-with-lease={ref}:{row.tip}", f":{ref}"], cwd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.branches")
    parser.add_argument("command", choices=("inventory", "reap"))
    parser.add_argument("--base", default="master")
    parser.add_argument("--only", nargs="+", default=[], metavar="NAME")
    parser.add_argument("--discard", action="store_true", help="also delete branches with unmerged work")
    parser.add_argument("--yes", action="store_true", help="actually delete; without it, dry run")
    parser.add_argument("--no-fetch", action="store_true")
    args = parser.parse_args(argv)

    cwd = "."
    if not args.no_fetch:
        _git(["fetch", "--prune", REMOTE], cwd)
    try:
        rows = build(cwd, args.base, fetch_prs(cwd))
    except (subprocess.CalledProcessError, FileNotFoundError) as error:
        print(f"could not read pull requests: {error}", file=sys.stderr)
        return 2

    if args.command == "inventory":
        for verdict in ORDER:
            group = [r for r in rows if r.verdict == verdict]
            if group:
                print(f"\n{verdict} ({len(group)})")
                for r in group:
                    print(f"  {r.name}  [{r.pr}]  {r.reason}")
        return 0

    if not args.only:
        print("reap needs --only NAME...; nothing is deleted in bulk", file=sys.stderr)
        return 2
    chosen, refused = select(rows, args.only, args.discard, {args.base})
    for name, why in refused:
        print(f"refused {name}: {why}", file=sys.stderr)
    for r in chosen:
        print(f"{r.verdict:<17} {r.name} @ {r.tip[:10]}  {r.reason}")
    if refused:
        return 2
    if not args.yes:
        print(f"\ndry run: {len(chosen)} branch(es) would be deleted; pass --yes", file=sys.stderr)
        return 0
    for r in chosen:
        delete(r, cwd)
        print(f"deleted {r.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
