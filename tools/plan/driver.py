"""Unattended step driver: one headless Claude session per ready ledger step.

Usage:
    python -m tools.plan.driver run [--max-steps N] [--only STEP] [--dry-run]
                                    [--budget USD] [--total-budget USD] [--timeout SECONDS] [--model M]
                                    [--permission-mode MODE]
    python -m tools.plan.driver status
    python -m tools.plan.driver release STEP

Runs append to HOME/driver/runs.jsonl. The driver itself calls no model; tokens are spent
only inside the sessions it launches.
"""

from __future__ import annotations

import argparse
import functools
import json
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from tools.plan import context, ledger, sessionlog

RUNS = sessionlog.RUNS
OUT_DIR = sessionlog.HOME / "driver" / "out"
OUTCOME_PREFIX = "DRIVER-OUTCOME:"
SETTLED = ("merged", "pr-open", "parked")
FAILED = ("failed", "timeout", "unreported", "violation")
MAX_ATTEMPTS = 2
OUT_RETENTION_DAYS = 30
HEARTBEAT_S = 30
PERMISSION_MODES = ("auto", "acceptEdits", "dontAsk")
PR_URL = re.compile(r"https://github\.com/[\w.-]+/[\w.-]+/pull/\d+")
PR_STATES = {"MERGED": "merged", "OPEN": "pr-open"}
OUTCOME_LINE = re.compile(
    rf"^\s*{re.escape(OUTCOME_PREFIX)}\s*({'|'.join(SETTLED)})\b:?\s*(.*?)\s*$", re.MULTILINE
)


class PreflightError(RuntimeError):
    pass


@dataclass(frozen=True)
class Options:
    budget: float = 15.0
    total_budget: float = 45.0
    timeout: int = 5400
    permission_mode: str = "auto"
    model: str = ""
    max_steps: int = 3
    max_failures: int = 2
    only: str = ""
    dry_run: bool = False


@dataclass(frozen=True)
class Candidate:
    step: ledger.Step
    package: str


@dataclass(frozen=True)
class Completed:
    returncode: int
    stdout: str
    stderr: str


@dataclass(frozen=True)
class TimedOut:
    stdout: str = ""
    stderr: str = ""


def candidates(root: Path, skip: frozenset[str] | set[str] = frozenset()) -> list[Candidate]:
    found = []
    for package in ledger.discover(Path(root)):
        try:
            steps = ledger.load_manifest(package)
        except (OSError, ValueError, KeyError, TypeError) as err:
            print(f"{package.name}: step manifest unreadable: {err!r}", file=sys.stderr)
            continue
        done = {step.id for step in steps if step.status == "done"}
        found.extend(
            Candidate(step, package.name)
            for step in steps
            if step.status == "pending"
            and step.id not in skip
            and all(dep in done for dep in step.depends_on)
        )
    return sorted(found, key=lambda c: (c.step.kind != "bug", not c.step.milestone))


def read_records(path: Path) -> list[dict]:
    return sessionlog.read_index(path)


def append_record(path: Path, record: dict) -> None:
    sessionlog.append_private(path, json.dumps(record) + "\n")


def skip_ids(records: list[dict]) -> set[str]:
    settled: set[str] = set()
    failures: dict[str, int] = {}
    for record in records:
        step = record.get("step")
        if record.get("event") == "release":
            settled.discard(step)
            failures.pop(step, None)
        elif record.get("event") == "run":
            if record.get("outcome") in SETTLED:
                settled.add(step)
            elif record.get("outcome") in FAILED:
                failures[step] = failures.get(step, 0) + 1
    return settled | {step for step, count in failures.items() if count >= MAX_ATTEMPTS}


def parse_outcome(text: str) -> tuple[str, str]:
    matches = OUTCOME_LINE.findall(text or "")
    return matches[-1] if matches else ("unreported", "")


def build_prompt(step: ledger.Step, package: str) -> str:
    return (
        f"Run the step-loop skill on ledger step {step.id} ({package}): {step.title}.\n"
        f"Its acceptance kind is {step.acceptance}.\n\n"
        "You are running unattended and no human will answer questions.\n"
        "- A previous attempt may have left a worktree, branch or open PR for this step. Look for "
        "one first and continue from it instead of creating another.\n"
        "- Follow the CLAUDE.md routing and every step-loop gate, including the fresh-context "
        "reviews.\n"
        "- Never ask a question. Park the step instead when it needs an owner decision, a "
        "load-bearing claim is false, a review escalation is unresolved, it needs privilege "
        "beyond the dispatcher, or it cannot be verified on this host.\n"
        "- Use the main checkout only to create the step's worktree.\n"
        "- Merge only as the step's acceptance kind allows.\n\n"
        f"Your final message must end with exactly one line:\n"
        f"{OUTCOME_PREFIX} merged <PR url>\n"
        f"{OUTCOME_PREFIX} pr-open <PR url>\n"
        f"{OUTCOME_PREFIX} parked: <one-line reason, phrased as the question for the owner>"
    )


def build_command(prompt: str, session_id: str, options: Options) -> list[str]:
    command = [
        "claude", "-p", prompt,
        "--output-format", "json",
        "--session-id", session_id,
        "--permission-mode", options.permission_mode,
        "--max-budget-usd", str(options.budget),
    ]
    if options.model:
        command += ["--model", options.model]
    return command


def run_process(
    command: list[str], cwd: Path, timeout: int, on_tick=None, interval: float = HEARTBEAT_S
) -> Completed | TimedOut:
    child = subprocess.Popen(
        command, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, text=True, start_new_session=True,
    )
    started = time.monotonic()
    while True:
        remaining = timeout - (time.monotonic() - started)
        try:
            stdout, stderr = child.communicate(timeout=max(0.0, min(interval, remaining)))
            return Completed(child.returncode, stdout, stderr)
        except subprocess.TimeoutExpired:
            if time.monotonic() - started >= timeout:
                os.killpg(child.pid, signal.SIGKILL)
                stdout, stderr = child.communicate()
                return TimedOut(stdout or "", stderr or "")
            if on_tick:
                on_tick(time.monotonic() - started)


def progress_line(session_id: str, elapsed: float) -> str:
    transcript = context.session_transcript(session_id, context.PROJECTS)
    head = f"  {sessionlog.fmt_duration(elapsed)}"
    if transcript is None:
        return f"{head} · starting"
    seen = sessionlog.last_activity(transcript)
    idle = "" if seen["idle_s"] is None else f" · idle {sessionlog.fmt_duration(seen['idle_s'])}"
    last = f" · last {seen['last_tool']}" if seen["last_tool"] else ""
    return f"{head} · {seen['tool_calls']} tool calls{last}{idle}"


def github_pr_state(url: str) -> str | None:
    done = subprocess.run(
        ["gh", "pr", "view", url, "--json", "state", "--jq", ".state"],
        capture_output=True, text=True,
    )
    return done.stdout.strip() or None if done.returncode == 0 else None


def open_pr_steps(step_ids: list[str]) -> set[str]:
    """Steps named by an open PR; a PR names its step id in the title or body."""
    done = subprocess.run(
        ["gh", "pr", "list", "--state", "open", "--limit", "200", "--json", "title,body,headRefName"],
        capture_output=True, text=True,
    )
    if done.returncode != 0:
        raise PreflightError(f"gh pr list: {done.stderr.strip()}")
    text = done.stdout
    return {step_id for step_id in step_ids if step_id in text}


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def execute(
    candidate: Candidate,
    options: Options,
    root: Path,
    process=run_process,
    out_dir: Path = OUT_DIR,
    pr_state=github_pr_state,
    announce=print,
    progress=progress_line,
) -> dict:
    session_id = str(uuid.uuid4())
    prompt = build_prompt(candidate.step, candidate.package)
    record = {
        "event": "run", "step": candidate.step.id, "package": candidate.package,
        "session_id": session_id, "started": _now(),
    }
    announce(f"  session {session_id}", flush=True)
    result = process(
        build_command(prompt, session_id, options), Path(root), options.timeout,
        lambda elapsed: announce(progress(session_id, elapsed), flush=True),
    )
    record["ended"] = _now()
    returncode = None if isinstance(result, TimedOut) else result.returncode
    sessionlog.write_private(
        Path(out_dir) / f"{session_id}.json",
        json.dumps({"returncode": returncode, "stdout": result.stdout, "stderr": result.stderr}),
    )
    if isinstance(result, TimedOut):
        return {**record, "outcome": "timeout", "detail": f"over {options.timeout}s"}
    record["exit_code"] = result.returncode
    try:
        payload = json.loads(result.stdout)
    except ValueError:
        payload = None
    if not isinstance(payload, dict) or result.returncode != 0:
        return {**record, "outcome": "failed", "detail": (result.stderr or result.stdout)[-200:]}
    record.update(
        cost_usd=payload.get("total_cost_usd"), turns=payload.get("num_turns"),
        duration_ms=payload.get("duration_ms"), denials=len(payload.get("permission_denials") or []),
    )
    text = payload.get("result") or ""
    if payload.get("is_error"):
        return {**record, "outcome": "failed", "detail": text[-200:]}
    outcome, detail = parse_outcome(text)
    if outcome in ("merged", "pr-open"):
        outcome, detail = verify_pr(candidate.step, outcome, detail, pr_state)
    return {**record, "outcome": outcome, "detail": detail}


def verify_pr(step: ledger.Step, claimed: str, detail: str, pr_state) -> tuple[str, str]:
    """The PR's real state replaces the session's claim; a claim with no readable PR is unreported."""
    match = PR_URL.search(detail)
    if not match:
        return "unreported", f"{claimed} claimed without a PR url"
    state = pr_state(match.group())
    if state is None:
        return "unreported", f"could not read {match.group()}"
    actual = PR_STATES.get(state)
    if actual is None:
        return "failed", f"{match.group()} is {state.lower()}"
    if actual == "merged" and step.acceptance == "product":
        return "violation", f"{match.group()} merged a product step"
    return actual, match.group()


def preflight(root: Path) -> None:
    if shutil.which("claude") is None:
        raise PreflightError("the claude CLI is not on PATH")

    def git(*args: str) -> str:
        done = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)
        if done.returncode != 0:
            raise PreflightError(f"git {' '.join(args)}: {done.stderr.strip()}")
        return done.stdout.strip()

    if git("rev-parse", "--abbrev-ref", "HEAD") != "master":
        raise PreflightError("the main checkout must be on master")
    if git("status", "--porcelain"):
        raise PreflightError("the main checkout has uncommitted changes")
    git("pull", "--ff-only", "origin", "master")


def prune_out(out_dir: Path = OUT_DIR, days: int = OUT_RETENTION_DAYS) -> None:
    cutoff = time.time() - days * 86400
    for path in Path(out_dir).glob("*.json"):
        if path.stat().st_mtime < cutoff:
            path.unlink()


def release_closed(runs: Path, pr_state) -> None:
    """A pr-open step whose PR was closed unmerged is offered again."""
    latest: dict[str, dict] = {}
    for record in read_records(runs):
        if record.get("event") == "release":
            latest.pop(record["step"], None)
        elif record.get("event") == "run":
            latest[record["step"]] = record
    for step, record in latest.items():
        if record["outcome"] == "pr-open" and PR_URL.search(record.get("detail", "")):
            if pr_state(record["detail"]) == "CLOSED":
                append_record(runs, {"event": "release", "step": step, "at": _now()})


def run_loop(
    root: Path,
    options: Options,
    runs: Path = RUNS,
    run_step=None,
    prepare=preflight,
    pr_state=github_pr_state,
    in_flight=open_pr_steps,
):
    run_step = run_step or functools.partial(execute, pr_state=pr_state)
    records: list[dict] = []
    streak = 0
    spent = 0.0
    prepare(root)
    if not options.dry_run:
        release_closed(runs, pr_state)
    while len(records) < options.max_steps:
        if records:
            try:
                prepare(root)
            except PreflightError as err:
                print(f"preflight failed after {records[-1]['step']}: {err}", file=sys.stderr)
                break
        ready = candidates(root, skip_ids(read_records(runs)))
        busy = in_flight([c.step.id for c in ready]) if ready else set()
        ready = [c for c in ready if c.step.id not in busy]
        if options.only:
            ready = [c for c in ready if c.step.id == options.only]
        if not ready:
            print("no ready step", file=sys.stderr)
            break
        if options.dry_run:
            for candidate in ready[: options.max_steps]:
                print(f"would run {candidate.step.id} ({candidate.package}): {candidate.step.title}")
            break
        candidate = ready[0]
        print(f"running {candidate.step.id} ...", flush=True)
        append_record(runs, {
            "event": "start", "step": candidate.step.id, "pid": os.getpid(), "started": _now(),
        })
        record = run_step(candidate, options, root)
        append_record(runs, record)
        records.append(record)
        print(f"  {record['outcome']} {record.get('detail', '')}".rstrip(), flush=True)
        spent += record.get("cost_usd") or 0
        streak = streak + 1 if record["outcome"] in FAILED else 0
        if streak >= options.max_failures:
            print(f"stopping after {streak} consecutive failures", file=sys.stderr)
            break
        if spent >= options.total_budget:
            print(f"stopping at ${spent:.2f} of the ${options.total_budget:.2f} total budget", file=sys.stderr)
            break
    return records


def pid_alive(pid) -> bool:
    try:
        os.kill(int(pid), 0)
    except (OSError, TypeError, ValueError):
        return False
    return True


def status(records: list[dict], alive=pid_alive) -> str:
    runs = [r for r in records if r.get("event") == "run"]
    latest: dict[str, dict] = {}
    started: dict[str, dict] = {}
    for record in records:
        if record.get("event") == "release":
            latest.pop(record["step"], None)
        elif record.get("event") == "start":
            started[record["step"]] = record
        elif record.get("event") == "run":
            latest[record["step"]] = record
            started.pop(record["step"], None)
    if not runs and not started:
        return "no runs\n"
    skipped = skip_ids(records)
    parked = [r for r in latest.values() if r["outcome"] == "parked"]
    stuck = [r for r in latest.values() if r["outcome"] in FAILED and r["step"] in skipped]
    spend = sum(r.get("cost_usd") or 0 for r in runs)
    lines = [f"{len(runs)} runs, ${spend:.2f}", ""]
    for r in started.values():
        state = "running" if alive(r.get("pid")) else "interrupted, no result recorded"
        lines += [f"{r['step']}: {state} since {r.get('started', '')[:19]}", ""]
    lines += ["| started | step | outcome | detail |", "|---|---|---|---|"]
    for r in sorted(runs, key=lambda r: r.get("started", ""), reverse=True)[:10]:
        lines.append(f"| {r.get('started', '')[:16]} | {r['step']} | {r['outcome']} | {r.get('detail', '')} |")
    if parked:
        lines += ["", "## Waiting on the owner", ""]
        lines += [f"- {r['step']}: {r.get('detail') or 'no reason given'}" for r in parked]
    if stuck:
        lines += ["", f"## Failed {MAX_ATTEMPTS} times, skipped", ""]
        lines += [f"- {r['step']} ({r['outcome']}): {r.get('detail') or 'no detail'}" for r in stuck]
    if parked or stuck:
        lines += ["", "Resume one with `python -m tools.plan.driver release STEP`."]
    return "\n".join(lines) + "\n"


def _positive(text: str) -> float:
    value = float(text)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError(f"{text!r} is not a positive number")
    return value


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.driver")
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run")
    defaults = Options()
    run.add_argument("--max-steps", type=int, default=defaults.max_steps)
    run.add_argument("--only", default="")
    run.add_argument("--dry-run", action="store_true")
    run.add_argument("--budget", type=_positive, default=defaults.budget)
    run.add_argument("--total-budget", type=_positive, default=defaults.total_budget)
    run.add_argument("--timeout", type=int, default=defaults.timeout)
    run.add_argument("--model", default="")
    run.add_argument(
        "--permission-mode", choices=PERMISSION_MODES, default=defaults.permission_mode
    )
    sub.add_parser("status")
    release = sub.add_parser("release")
    release.add_argument("step")
    args = parser.parse_args(argv)

    if args.command == "status":
        print(status(read_records(RUNS)))
        return 0
    if args.command == "release":
        append_record(RUNS, {"event": "release", "step": args.step, "at": _now()})
        print(f"released {args.step}")
        return 0
    options = Options(
        budget=args.budget, total_budget=args.total_budget, timeout=args.timeout, permission_mode=args.permission_mode,
        model=args.model, max_steps=args.max_steps, only=args.only, dry_run=args.dry_run,
    )
    try:
        if not args.dry_run and OUT_DIR.is_dir():
            prune_out()
        records = run_loop(Path(sessionlog.main_checkout()), options)
    except PreflightError as err:
        print(f"preflight failed: {err}", file=sys.stderr)
        return 2
    print(status(records))
    return 1 if records and all(r["outcome"] in FAILED for r in records) else 0


if __name__ == "__main__":
    sys.exit(main())
