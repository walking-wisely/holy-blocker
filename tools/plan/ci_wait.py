"""Wait for a pull request's gate workflow without sleeping in the caller.

Usage:
    python -m tools.plan.ci_wait PR [--gate "CI gate"] [--sha SHA] [--timeout 600]
                                    [--interval 10] [--grace 120]

The gate is every check in the workflow that owns the `CI gate` check, so a re-run triggered by a
PR body edit, which skips the heavy jobs, cannot hide a failure in the original run. Checks from
other workflows, such as CodeQL, are advisory: printed with the result, never waited on.

Exit code 0 passed, 1 failed, 2 timed out, 3 the gate never appeared, the PR head is not the
expected commit, or gh failed, 4 cancelled.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from dataclasses import dataclass

GATE = "CI gate"
PASS, FAIL, PENDING, CANCELLED = "pass", "fail", "pending", "cancelled"
EXIT = {PASS: 0, FAIL: 1, CANCELLED: 4}
TIMEOUT_EXIT, MISSING_EXIT = 2, 3
SEVERITY = {"pass": 0, "skipping": 0, "pending": 1, "cancel": 2, "fail": 3}
NOT_STARTED_YEAR = "0001"
NEWEST = "9999"
PR_ARGUMENT = re.compile(r"\d+|https://github\.com/[\w.-]+/[\w.-]+/pull/\d+")
NO_CHECKS = "no checks reported"


@dataclass(frozen=True)
class Snapshot:
    checks: list
    head: str = ""
    error: str = ""


def parse_checks(text: str) -> list[dict]:
    try:
        checks = json.loads(text)
    except ValueError:
        return []
    return [c for c in checks if isinstance(c, dict)] if isinstance(checks, list) else []


def _order(check: dict) -> tuple[str, int]:
    """A rerun that has not started yet is newer than every finished run."""
    started = check.get("startedAt") or ""
    if not started or started.startswith(NOT_STARTED_YEAR):
        started = NEWEST
    return started, SEVERITY.get(check.get("bucket"), 1)


def _latest(checks: list[dict]) -> dict[str, dict]:
    latest: dict[str, dict] = {}
    for check in checks:
        name = check.get("name")
        if name not in latest or _order(check) > _order(latest[name]):
            latest[name] = check
    return latest


def evaluate(checks: list[dict], gate: str) -> tuple[str, dict[str, str]]:
    latest = _latest(checks)
    found = latest.get(gate)
    if found is None:
        return PENDING, {}
    workflow = found.get("workflow")
    blocking = {n: c for n, c in latest.items() if workflow and c.get("workflow") == workflow}
    blocking[gate] = found
    advisory = {
        n: c["bucket"]
        for n, c in latest.items()
        if n not in blocking and c.get("bucket") in ("pending", "fail", "cancel")
    }
    buckets = {c.get("bucket") for c in blocking.values()}
    if "fail" in buckets or found.get("bucket") == "skipping":
        return FAIL, advisory
    if "cancel" in buckets:
        return CANCELLED, advisory
    if "pending" in buckets or buckets - {"pass", "skipping"} or found.get("bucket") != "pass":
        return PENDING, advisory
    return PASS, advisory


def _result(gate: str, state: str, advisory: dict[str, str]) -> str:
    extra = ", ".join(f"{name} {bucket}" for name, bucket in advisory.items())
    return f"{gate}: {state}" + (f" (advisory: {extra})" if extra else "")


def wait(
    fetch, gate: str, timeout: float, interval: float, grace: float, sleep, clock, out,
    expected_sha: str,
) -> int:
    if not expected_sha:
        raise ValueError("expected_sha is required: checks are only trusted for a known commit")
    started = clock()
    previous = None
    while True:
        snapshot = fetch()
        elapsed = clock() - started
        stale = snapshot.head != expected_sha
        checks = [] if stale else snapshot.checks
        state, advisory = evaluate(checks, gate)
        if state != PENDING:
            out(_result(gate, state, advisory))
            return EXIT[state]
        if not any(c.get("name") == gate for c in checks) and elapsed >= grace:
            if stale and not snapshot.head:
                reason = f"could not read the PR head ({snapshot.error or 'no output'})"
            elif stale:
                reason = f"the PR head is {snapshot.head[:10]}, not the pushed {expected_sha[:10]}"
            elif snapshot.error:
                reason = f"gh failed: {snapshot.error}"
            else:
                reason = f"{gate} never appeared"
            out(f"{reason} after {round(grace)}s")
            return MISSING_EXIT
        if elapsed >= timeout:
            out(f"timed out after {round(timeout)}s: {_result(gate, state, advisory)}")
            return TIMEOUT_EXIT
        line = f"{gate}: pending"
        if line != previous:
            out(line)
            previous = line
        sleep(interval)


def _gh(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["gh", *args], capture_output=True, text=True)


def _head(pr: str) -> subprocess.CompletedProcess:
    return _gh("pr", "view", pr, "--json", "headRefOid", "--jq", ".headRefOid")


def fetch_snapshot(pr: str) -> Snapshot:
    """Checks plus the head they belong to; a head that moved while reading counts as unknown."""
    before = _head(pr)
    checks = _gh("pr", "checks", pr, "--json", "name,bucket,startedAt,workflow")
    after = _head(pr)
    parsed = parse_checks(checks.stdout)
    problem = ""
    if not parsed and NO_CHECKS not in checks.stderr and checks.returncode not in (0, 8):
        problem = checks.stderr.strip()[:200]
    head = after.stdout.strip() if before.stdout.strip() == after.stdout.strip() else ""
    if not head and not problem:
        problem = (after.stderr or before.stderr).strip()[:200] or "the PR head changed while reading"
    return Snapshot(parsed, head, problem)


def local_head() -> str:
    done = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True)
    return done.stdout.strip() if done.returncode == 0 else ""


def pr_argument(text: str) -> str:
    if not PR_ARGUMENT.fullmatch(text):
        raise argparse.ArgumentTypeError(f"{text!r} is not a PR number or a pull request URL")
    return text


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.ci_wait")
    parser.add_argument("pr", type=pr_argument)
    parser.add_argument("--gate", default=GATE)
    parser.add_argument("--sha", default=None)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--interval", type=float, default=10)
    parser.add_argument("--grace", type=float, default=120)
    args = parser.parse_args(argv)
    expected = args.sha if args.sha is not None else local_head()
    if not expected:
        print("cannot tell which commit was pushed; run this from the checkout you pushed from "
              "or pass --sha", flush=True)
        return MISSING_EXIT
    return wait(
        lambda: fetch_snapshot(args.pr), args.gate, args.timeout, args.interval, args.grace,
        time.sleep, time.monotonic, lambda text: print(text, flush=True), expected,
    )


if __name__ == "__main__":
    sys.exit(main())
