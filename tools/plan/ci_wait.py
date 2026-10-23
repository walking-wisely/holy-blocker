"""Wait for a pull request's gate check without sleeping in the caller.

Usage:
    python -m tools.plan.ci_wait PR [--gate "CI gate"] [--timeout 600] [--interval 10] [--grace 120]

Exit code 0 is a passed gate, 1 a failed one, 2 a timeout or a gate that never appeared. Every
other check is advisory: its unfinished or failing state is printed with the result and never
waited on.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time

GATE = "CI gate"
FAILED_BUCKETS = ("fail", "cancel", "skipping")
ADVISORY_BUCKETS = ("pending", "fail", "cancel")
NOT_STARTED = ""


def parse_checks(text: str) -> list[dict]:
    try:
        checks = json.loads(text)
    except ValueError:
        return []
    return [c for c in checks if isinstance(c, dict)] if isinstance(checks, list) else []


def _latest(checks: list[dict]) -> dict[str, dict]:
    latest: dict[str, dict] = {}
    for check in checks:
        name = check.get("name")
        started = check.get("startedAt") or NOT_STARTED
        if name not in latest or started > (latest[name].get("startedAt") or NOT_STARTED):
            latest[name] = check
    return latest


def evaluate(checks: list[dict], gate: str) -> tuple[str, dict[str, str]]:
    latest = _latest(checks)
    found = latest.pop(gate, None)
    bucket = found.get("bucket") if found else None
    state = "pending"
    if bucket == "pass":
        state = "pass"
    elif bucket in FAILED_BUCKETS:
        state = "fail"
    advisory = {
        name: check["bucket"] for name, check in latest.items() if check.get("bucket") in ADVISORY_BUCKETS
    }
    return state, advisory


def _result(gate: str, state: str, advisory: dict[str, str]) -> str:
    extra = ", ".join(f"{name} {bucket}" for name, bucket in advisory.items())
    return f"{gate}: {state}" + (f" (advisory: {extra})" if extra else "")


def wait(fetch, gate: str, timeout: float, interval: float, grace: float, sleep, clock, out) -> int:
    started = clock()
    previous = None
    while True:
        checks = fetch()
        state, advisory = evaluate(checks, gate)
        elapsed = clock() - started
        if state != "pending":
            out(_result(gate, state, advisory))
            return 0 if state == "pass" else 1
        if not any(c.get("name") == gate for c in checks) and elapsed >= grace:
            out(f"{gate} never appeared after {round(grace)}s")
            return 2
        if elapsed >= timeout:
            out(f"timed out after {round(timeout)}s: {_result(gate, state, advisory)}")
            return 2
        line = f"{gate}: pending"
        if line != previous:
            out(line)
            previous = line
        sleep(interval)


def fetch_checks(pr: str) -> list[dict]:
    done = subprocess.run(
        ["gh", "pr", "checks", pr, "--json", "name,bucket,startedAt"],
        capture_output=True, text=True,
    )
    return parse_checks(done.stdout)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.ci_wait")
    parser.add_argument("pr")
    parser.add_argument("--gate", default=GATE)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--interval", type=float, default=10)
    parser.add_argument("--grace", type=float, default=120)
    args = parser.parse_args(argv)
    return wait(
        lambda: fetch_checks(args.pr), args.gate, args.timeout, args.interval, args.grace,
        time.sleep, time.monotonic, lambda text: print(text, flush=True),
    )


if __name__ == "__main__":
    sys.exit(main())
