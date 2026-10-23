"""Metrics index over Claude Code session transcripts.

Usage:
    python -m tools.plan.sessionlog index
    python -m tools.plan.sessionlog report [--since 7d] [--kind driver|interactive] [--limit N]
    python -m tools.plan.sessionlog show SESSION_ID

A row holds counts and durations only. No prompt or response text is copied out of a
transcript; the row's `path` points at the transcript for anything deeper.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import subprocess
import sys
from collections import Counter
from datetime import datetime, timedelta, timezone
from pathlib import Path

from tools.plan import context

HOME = Path(os.environ.get("HB_LOG_HOME") or Path.home() / ".holy-blocker")
PROJECTS = context.PROJECTS
INDEX = HOME / "sessions" / "index.jsonl"
RUNS = HOME / "driver" / "runs.jsonl"
EDIT_TOOLS = ("Edit", "Write", "NotebookEdit")
DENIAL_MARKERS = ("denied", "doesn't want to proceed")
TOP = 5
IDLE_S = 600


def _time(value: str | None) -> datetime | None:
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except (AttributeError, ValueError):
        return None


def _flatten(content) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(b.get("text", "") for b in content if isinstance(b, dict))
    return ""


def _is_prompt(entry: dict) -> bool:
    if entry.get("isSidechain") or entry.get("isMeta"):
        return False
    content = entry.get("message", {}).get("content")
    if isinstance(content, str):
        return True
    return isinstance(content, list) and any(
        isinstance(b, dict) and b.get("type") == "text" for b in content
    )


def _entries(path: Path):
    with Path(path).open(encoding="utf-8", errors="replace") as lines:
        for line in lines:
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            if isinstance(entry, dict):
                yield entry


def summarize(path: Path) -> dict:
    path = Path(path)
    first = last = last_seen = None
    human_prompts = 0
    wait = longest_wait = active = 0.0
    assistant_msgs = subagents = output_tokens = peak_context = 0
    cost = None
    meta: dict[str, str] = {}
    title = ""
    models: Counter = Counter()
    tool_counts: Counter = Counter()
    tool_errors: Counter = Counter()
    tool_seconds: Counter = Counter()
    bash_heads: Counter = Counter()
    skills: Counter = Counter()
    edited: set[str] = set()
    pending: dict[str, tuple[str, datetime | None]] = {}
    permission_blocks = 0

    for entry in _entries(path):
        kind = entry.get("type")
        stamp = _time(entry.get("timestamp"))
        if kind == "ai-title":
            title = entry.get("aiTitle", title)
        elif kind == "cost-state":
            cost = entry.get("totalCostUSD", cost)
        for key, field in (("entrypoint", "entrypoint"), ("cwd", "cwd"), ("branch", "gitBranch")):
            if key not in meta and entry.get(field):
                meta[key] = entry[field]
        if stamp is None:
            continue
        first = min(first, stamp) if first else stamp
        last = max(last, stamp) if last else stamp

        human = False
        if kind == "user":
            content = entry.get("message", {}).get("content")
            if _is_prompt(entry) and entry.get("origin", {}).get("kind") == "human":
                if human_prompts and last_seen:
                    gap = (stamp - last_seen).total_seconds()
                    wait += gap
                    longest_wait = max(longest_wait, gap)
                human_prompts += 1
                human = True
            for block in content if isinstance(content, list) else []:
                if not isinstance(block, dict) or block.get("type") != "tool_result":
                    continue
                name, started = pending.pop(block.get("tool_use_id"), ("", None))
                if name and started:
                    tool_seconds[name] += (stamp - started).total_seconds()
                if block.get("is_error"):
                    tool_errors[name or "unknown"] += 1
                    text = _flatten(block.get("content")).lower()
                    if any(marker in text for marker in DENIAL_MARKERS):
                        permission_blocks += 1
        elif kind == "assistant":
            message = entry.get("message", {})
            usage = message.get("usage") or {}
            if message.get("model") == context.SYNTHETIC_MODEL:
                continue
            assistant_msgs += 1
            models[message.get("model", "unknown")] += 1
            output_tokens += usage.get("output_tokens") or 0
            if not entry.get("isSidechain"):
                peak_context = max(
                    peak_context, sum(usage.get(key) or 0 for key in context.USAGE_KEYS)
                )
            for block in message.get("content") or []:
                if not isinstance(block, dict) or block.get("type") != "tool_use":
                    continue
                name, args = block.get("name", "unknown"), block.get("input") or {}
                tool_counts[name] += 1
                pending[block.get("id")] = (name, stamp)
                if name == "Agent":
                    subagents += 1
                elif name == "Skill":
                    skills[args.get("skill", "unknown")] += 1
                elif name == "Bash" and str(args.get("command", "")).split():
                    bash_heads[str(args["command"]).split()[0]] += 1
                elif name in EDIT_TOOLS and args.get("file_path"):
                    edited.add(args["file_path"])
        if last_seen and not human:
            gap = (stamp - last_seen).total_seconds()
            if gap <= IDLE_S:
                active += gap
        last_seen = stamp

    duration = (last - first).total_seconds() if first and last else 0.0
    return {
        "session_id": path.stem,
        "kind": "driver" if meta.get("entrypoint", "").startswith("sdk") else "interactive",
        "title": title,
        "cwd": meta.get("cwd", ""),
        "branch": meta.get("branch", ""),
        "start": first.isoformat() if first else "",
        "duration_s": round(duration),
        "wait_s": round(wait),
        "longest_wait_s": round(longest_wait),
        "active_s": round(active),
        "human_prompts": human_prompts,
        "assistant_msgs": assistant_msgs,
        "output_tokens": output_tokens,
        "peak_context": peak_context,
        "cost_usd": cost,
        "models": dict(models),
        "tool_counts": dict(tool_counts),
        "tool_errors": dict(tool_errors),
        "tool_seconds": {k: round(v) for k, v in tool_seconds.items()},
        "permission_blocks": permission_blocks,
        "bash_heads": dict(bash_heads.most_common(10)),
        "skills": dict(skills),
        "subagents": subagents,
        "files_edited": len(edited),
    }


def read_index(index: Path) -> list[dict]:
    try:
        lines = Path(index).read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    rows = []
    for line in lines:
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows


def refresh(projects: Path, prefix: str, index: Path) -> list[dict]:
    """Summarize new or changed transcripts of projects whose directory starts with prefix."""
    known = {row["session_id"]: row for row in read_index(index)}
    for directory in sorted(Path(projects).glob(f"{glob.escape(prefix)}*")):
        for transcript in directory.glob("*.jsonl"):
            mtime = transcript.stat().st_mtime
            row = known.get(transcript.stem)
            if row and row.get("mtime") == mtime:
                continue
            known[transcript.stem] = {**summarize(transcript), "mtime": mtime, "path": str(transcript)}
    rows = sorted(known.values(), key=lambda r: r.get("start", ""))
    Path(index).parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(index).with_suffix(".tmp")
    temporary.write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
    temporary.replace(index)
    return rows


def parse_since(text: str) -> timedelta:
    match = re.fullmatch(r"(\d+)([dh])", text)
    if not match:
        raise ValueError(f"expected a duration like 7d or 12h, got {text!r}")
    amount, unit = int(match.group(1)), match.group(2)
    return timedelta(days=amount) if unit == "d" else timedelta(hours=amount)


def load_runs(runs: Path) -> dict[str, dict]:
    """Driver run records by session id."""
    found = {}
    try:
        lines = Path(runs).read_text(encoding="utf-8").splitlines()
    except OSError:
        return found
    for line in lines:
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get("event") == "run" and record.get("session_id"):
            found[record["session_id"]] = record
    return found


def fmt_duration(seconds: float) -> str:
    seconds = round(seconds)
    hours, rest = divmod(seconds, 3600)
    minutes, secs = divmod(rest, 60)
    if hours:
        return f"{hours}h{minutes:02d}m"
    return f"{minutes}m{secs:02d}s" if minutes else f"{secs}s"


def _top(counter: Counter, unit) -> str:
    return ", ".join(f"{name} {unit(value)}" for name, value in counter.most_common(TOP)) or "none"


def report(rows: list[dict], kind: str | None, runs: dict[str, dict], limit: int = 30) -> str:
    rows = [r for r in rows if kind in (None, r.get("kind"))]
    if not rows:
        return "no sessions\n"
    table = [
        "| start | kind | step / title | branch | active | waiting | cost | tool errors | blocks |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for row in sorted(rows, key=lambda r: r.get("start", ""), reverse=True)[:limit]:
        run = runs.get(row["session_id"])
        label = f"{run['step']} ({run['outcome']})" if run else row.get("title", "")
        cost = row.get("cost_usd")
        table.append(
            f"| {row.get('start', '')[:16]} | {row['kind']} | {label} | {row.get('branch', '')} "
            f"| {fmt_duration(row['active_s'])} | {fmt_duration(row['wait_s'])} "
            f"| {'' if cost is None else f'${cost:.2f}'} | {sum(row['tool_errors'].values())} "
            f"| {row['permission_blocks']} |"
        )
    total = sum(r["active_s"] for r in rows)
    waiting = sum(r["wait_s"] for r in rows)
    spend = sum(r["cost_usd"] or 0 for r in rows)
    seconds: Counter = Counter()
    errors: Counter = Counter()
    for row in rows:
        seconds.update(row["tool_seconds"])
        errors.update(row["tool_errors"])
    longest = sorted(rows, key=lambda r: r["longest_wait_s"], reverse=True)[:TOP]
    noisy = sorted(rows, key=lambda r: sum(r["tool_errors"].values()), reverse=True)[:TOP]
    lines = [
        f"{len(rows)} sessions, {fmt_duration(total)} active, "
        f"{fmt_duration(waiting)} waiting on a human, ${spend:.2f}",
        "",
        *table,
        "",
        "## Friction",
        "",
        "Longest waits: " + ", ".join(
            f"{r['session_id'][:8]} {fmt_duration(r['longest_wait_s'])}" for r in longest
        ),
        "Most tool errors: " + ", ".join(
            f"{r['session_id'][:8]} {sum(r['tool_errors'].values())}" for r in noisy
        ),
        "Slowest tools: " + _top(seconds, fmt_duration),
        "Failing tools: " + _top(errors, str),
        f"Permission blocks: {sum(r['permission_blocks'] for r in rows)}",
        "",
    ]
    return "\n".join(lines)


def main_checkout() -> str:
    common = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        capture_output=True,
        text=True,
    )
    if common.returncode != 0:
        return str(Path.cwd())
    return str(Path(common.stdout.strip()).parent)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.sessionlog")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("index")
    show = sub.add_parser("show")
    show.add_argument("session_id")
    rep = sub.add_parser("report")
    rep.add_argument("--since", default="")
    rep.add_argument("--kind", choices=("driver", "interactive"))
    rep.add_argument("--limit", type=int, default=30)
    args = parser.parse_args(argv)

    rows = refresh(PROJECTS, context.project_slug(main_checkout()), INDEX)
    if args.command == "index":
        print(f"{len(rows)} sessions indexed in {INDEX}")
        return 0
    if args.command == "show":
        matches = [r for r in rows if r["session_id"].startswith(args.session_id)]
        if len(matches) != 1:
            print(f"{len(matches)} sessions match {args.session_id!r}", file=sys.stderr)
            return 1
        print(json.dumps(matches[0], indent=2))
        return 0
    if args.since:
        try:
            cutoff = datetime.now(timezone.utc) - parse_since(args.since)
        except ValueError as err:
            print(err, file=sys.stderr)
            return 2
        rows = [r for r in rows if (_time(r.get("start")) or cutoff) >= cutoff]
    print(report(rows, args.kind, load_runs(RUNS), args.limit))
    return 0


if __name__ == "__main__":
    sys.exit(main())
