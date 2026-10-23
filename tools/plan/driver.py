"""Unattended step driver: one headless Claude session per ready ledger step.

Usage:
    python -m tools.plan.driver run [--max-steps N] [--only STEP] [--dry-run]
                                    [--budget USD] [--timeout SECONDS] [--model M]
                                    [--permission-mode MODE]
    python -m tools.plan.driver status
    python -m tools.plan.driver release STEP

Runs append to HOME/driver/runs.jsonl. The driver itself calls no model; tokens are spent
only inside the sessions it launches.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from tools.plan import ledger, sessionlog

RUNS = sessionlog.RUNS
OUT_DIR = sessionlog.HOME / "driver" / "out"
OUTCOME_PREFIX = "DRIVER-OUTCOME:"
SETTLED = ("merged", "pr-open", "parked")
FAILED = ("failed", "timeout", "unreported")
MAX_ATTEMPTS = 2
OUTCOME_LINE = re.compile(
    rf"^\s*{re.escape(OUTCOME_PREFIX)}\s*({'|'.join(SETTLED)})\b:?\s*(.*?)\s*$", re.MULTILINE
)


class PreflightError(RuntimeError):
    pass


@dataclass(frozen=True)
class Options:
    budget: float = 15.0
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
    pass


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
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with Path(path).open("a", encoding="utf-8") as out:
        out.write(json.dumps(record) + "\n")


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
        f"Run the step-loop skill on ledger step {step.id} ({package}): {step.title}.\n\n"
        "You are running unattended and no human will answer questions.\n"
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


def run_process(command: list[str], cwd: Path, timeout: int) -> Completed | TimedOut:
    try:
        done = subprocess.run(
            command, cwd=cwd, capture_output=True, text=True, timeout=timeout,
            stdin=subprocess.DEVNULL,
        )
    except subprocess.TimeoutExpired:
        return TimedOut()
    return Completed(done.returncode, done.stdout, done.stderr)


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def execute(
    candidate: Candidate,
    options: Options,
    root: Path,
    process=run_process,
    out_dir: Path = OUT_DIR,
) -> dict:
    session_id = str(uuid.uuid4())
    prompt = build_prompt(candidate.step, candidate.package)
    record = {
        "event": "run", "step": candidate.step.id, "package": candidate.package,
        "session_id": session_id, "started": _now(),
    }
    result = process(build_command(prompt, session_id, options), Path(root), options.timeout)
    record["ended"] = _now()
    if isinstance(result, TimedOut):
        return {**record, "outcome": "timeout", "detail": f"over {options.timeout}s"}

    Path(out_dir).mkdir(parents=True, exist_ok=True)
    (Path(out_dir) / f"{session_id}.json").write_text(
        json.dumps({"returncode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}),
        encoding="utf-8",
    )
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
    return {**record, "outcome": outcome, "detail": detail}


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


def run_loop(root: Path, options: Options, runs: Path = RUNS, run_step=execute, prepare=preflight):
    records: list[dict] = []
    streak = 0
    while len(records) < options.max_steps:
        prepare(root)
        ready = candidates(root, skip_ids(read_records(runs)))
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
        record = run_step(candidate, options, root)
        append_record(runs, record)
        records.append(record)
        print(f"  {record['outcome']} {record.get('detail', '')}".rstrip(), flush=True)
        streak = streak + 1 if record["outcome"] in FAILED else 0
        if streak >= options.max_failures:
            print(f"stopping after {streak} consecutive failures", file=sys.stderr)
            break
    return records


def status(records: list[dict]) -> str:
    runs = [r for r in records if r.get("event") == "run"]
    if not runs:
        return "no runs\n"
    latest: dict[str, dict] = {}
    for record in records:
        if record.get("event") == "release":
            latest.pop(record["step"], None)
        elif record.get("event") == "run":
            latest[record["step"]] = record
    parked = [r for r in latest.values() if r["outcome"] == "parked"]
    spend = sum(r.get("cost_usd") or 0 for r in runs)
    lines = [f"{len(runs)} runs, ${spend:.2f}", ""]
    lines += ["| started | step | outcome | detail |", "|---|---|---|---|"]
    for r in sorted(runs, key=lambda r: r.get("started", ""), reverse=True)[:10]:
        lines.append(f"| {r.get('started', '')[:16]} | {r['step']} | {r['outcome']} | {r.get('detail', '')} |")
    if parked:
        lines += ["", "## Waiting on the owner", ""]
        lines += [f"- {r['step']}: {r.get('detail') or 'no reason given'}" for r in parked]
        lines += ["", "Resume one with `python -m tools.plan.driver release STEP`."]
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.driver")
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run")
    defaults = Options()
    run.add_argument("--max-steps", type=int, default=defaults.max_steps)
    run.add_argument("--only", default="")
    run.add_argument("--dry-run", action="store_true")
    run.add_argument("--budget", type=float, default=defaults.budget)
    run.add_argument("--timeout", type=int, default=defaults.timeout)
    run.add_argument("--model", default="")
    run.add_argument("--permission-mode", default=defaults.permission_mode)
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
        budget=args.budget, timeout=args.timeout, permission_mode=args.permission_mode,
        model=args.model, max_steps=args.max_steps, only=args.only, dry_run=args.dry_run,
    )
    try:
        records = run_loop(Path(sessionlog.main_checkout()), options)
    except PreflightError as err:
        print(f"preflight failed: {err}", file=sys.stderr)
        return 2
    print(status(records))
    return 1 if records and all(r["outcome"] in FAILED for r in records) else 0


if __name__ == "__main__":
    sys.exit(main())
