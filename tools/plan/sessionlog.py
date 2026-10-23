"""Metrics index over Claude Code session transcripts.

Usage:
    python -m tools.plan.sessionlog index
    python -m tools.plan.sessionlog report [--since 7d] [--kind driver|interactive] [--limit N]
    python -m tools.plan.sessionlog show SESSION_ID

A row holds counts, durations, the branch name and identifiers that match SAFE_NAME. No prompt
or response text, session title or working directory is copied out of a transcript; the row's
`path` points at the transcript for anything deeper. Files are owner-only and rows older than
INDEX_RETENTION_DAYS are dropped.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import subprocess
import sys
import tempfile
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
INDEX_RETENTION_DAYS = 180
SAFE_NAME = re.compile(r"[A-Za-z][A-Za-z0-9_+-]*")
OTHER = "other"
SAFE_FILE = re.compile(r"[\w.+-]{1,80}")


def safe_name(value) -> str:
    return value if isinstance(value, str) and SAFE_NAME.fullmatch(value) else OTHER


def private_dir(path: Path) -> None:
    path = Path(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.chmod(0o700)


def append_private(path: Path, line: str) -> None:
    private_dir(Path(path).parent)
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    with os.fdopen(descriptor, "a", encoding="utf-8") as out:
        out.write(line)


def write_private(path: Path, text: str) -> None:
    private_dir(Path(path).parent)
    descriptor, temporary = tempfile.mkstemp(dir=Path(path).parent, suffix=".tmp")
    with os.fdopen(descriptor, "w", encoding="utf-8") as out:
        out.write(text)
    os.replace(temporary, path)


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
            if not isinstance(entry, dict):
                continue
            message = entry.get("message")
            if message is not None and not isinstance(message, dict):
                entry["message"] = {}
            elif isinstance(message, dict) and not isinstance(message.get("usage") or {}, dict):
                message["usage"] = {}
            yield entry


def summarize(path: Path) -> dict:
    path = Path(path)
    first = last = last_seen = None
    human_prompts = 0
    wait = longest_wait = active = 0.0
    subagents = peak_context = 0
    cost = None
    meta: dict[str, str] = {}
    models: Counter = Counter()
    seen_messages: set[str] = set()
    message_tokens: dict[str, int] = {}
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
        if kind == "cost-state":
            cost = entry.get("totalCostUSD", cost)
        for key, field in (("entrypoint", "entrypoint"), ("branch", "gitBranch")):
            if key not in meta and entry.get(field):
                meta[key] = entry[field]
        if stamp is None:
            continue
        first = min(first, stamp) if first else stamp
        last = max(last, stamp) if last else stamp

        human = False
        resumed_tool = False
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
                resumed_tool = True
                name, started = pending.pop(block.get("tool_use_id"), ("", None))
                if name and started:
                    tool_seconds[name] += (stamp - started).total_seconds()
                if block.get("is_error"):
                    tool_errors[name or OTHER] += 1
                    text = _flatten(block.get("content")).lower()
                    if any(marker in text for marker in DENIAL_MARKERS):
                        permission_blocks += 1
        elif kind == "assistant":
            message = entry.get("message", {})
            usage = message.get("usage") or {}
            if message.get("model") == context.SYNTHETIC_MODEL:
                continue
            message_id = message.get("id") or f"line-{len(seen_messages)}"
            if message_id not in seen_messages:
                seen_messages.add(message_id)
                models[safe_name(message.get("model", "").replace(".", "-"))] += 1
            message_tokens[message_id] = max(
                message_tokens.get(message_id, 0), usage.get("output_tokens") or 0
            )
            if not entry.get("isSidechain"):
                peak_context = max(
                    peak_context, sum(usage.get(key) or 0 for key in context.USAGE_KEYS)
                )
            for block in message.get("content") or []:
                if not isinstance(block, dict) or block.get("type") != "tool_use":
                    continue
                name, args = safe_name(block.get("name")), block.get("input") or {}
                tool_counts[name] += 1
                pending[block.get("id")] = (name, stamp)
                if name == "Agent":
                    subagents += 1
                elif name == "Skill":
                    skills[safe_name(args.get("skill"))] += 1
                elif name == "Bash" and str(args.get("command", "")).split():
                    bash_heads[safe_name(str(args["command"]).split()[0])] += 1
                elif name in EDIT_TOOLS and args.get("file_path"):
                    edited.add(args["file_path"])
        if last_seen and not human:
            gap = (stamp - last_seen).total_seconds()
            if gap <= IDLE_S or resumed_tool:
                active += gap
        last_seen = stamp

    output_tokens = sum(message_tokens.values())
    duration = (last - first).total_seconds() if first and last else 0.0
    return {
        "session_id": path.stem,
        "kind": "driver" if meta.get("entrypoint", "").startswith("sdk") else "interactive",
        "branch": meta.get("branch", ""),
        "start": first.isoformat() if first else "",
        "duration_s": round(duration),
        "wait_s": round(wait),
        "longest_wait_s": round(longest_wait),
        "active_s": round(active),
        "human_prompts": human_prompts,
        "assistant_msgs": len(seen_messages),
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


def describe_tool(block: dict) -> str:
    """A short label for a tool_use block: the tool, plus a plain identifier or file name."""
    name = safe_name(block.get("name"))
    args = block.get("input") if isinstance(block.get("input"), dict) else {}
    detail = ""
    if name == "Bash" and str(args.get("command", "")).split():
        detail = safe_name(str(args["command"]).split()[0])
    elif name == "Skill":
        detail = safe_name(args.get("skill"))
    elif name == "Agent":
        detail = safe_name(args["subagent_type"]) if args.get("subagent_type") else ""
    elif name in (*EDIT_TOOLS, "Read") and args.get("file_path"):
        base = Path(str(args["file_path"])).name
        detail = base if SAFE_FILE.fullmatch(base) else OTHER
    return f"{name}({detail})" if detail else name


def last_activity(path: Path) -> dict:
    """Tool-call count, the latest tool, and seconds since the last event, for a running session."""
    calls, last_tool, last_stamp = 0, "", None
    for entry in _entries(path):
        stamp = _time(entry.get("timestamp"))
        if stamp:
            last_stamp = stamp
        if entry.get("type") != "assistant":
            continue
        for block in entry.get("message", {}).get("content") or []:
            if isinstance(block, dict) and block.get("type") == "tool_use":
                calls += 1
                last_tool = describe_tool(block)
    idle = (datetime.now(timezone.utc) - last_stamp).total_seconds() if last_stamp else None
    return {"tool_calls": calls, "last_tool": last_tool, "idle_s": None if idle is None else round(idle)}


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
    candidates = sorted(Path(projects).glob(f"{glob.escape(prefix)}*"))
    for directory in candidates:
        if directory.name != prefix and not directory.name.startswith(f"{prefix}--"):
            continue
        for transcript in directory.glob("*.jsonl"):
            mtime = transcript.stat().st_mtime
            row = known.get(transcript.stem)
            if row and row.get("mtime") == mtime:
                continue
            known[transcript.stem] = {**summarize(transcript), "mtime": mtime, "path": str(transcript)}
    cutoff = datetime.now(timezone.utc) - timedelta(days=INDEX_RETENTION_DAYS)
    rows = sorted(
        (r for r in known.values() if (_time(r.get("start")) or cutoff) >= cutoff),
        key=lambda r: r.get("start", ""),
    )
    write_private(index, "".join(json.dumps(r) + "\n" for r in rows))
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
        "| start | kind | step / session | branch | active | waiting | cost | tool errors | blocks |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for row in sorted(rows, key=lambda r: r.get("start", ""), reverse=True)[:limit]:
        run = runs.get(row["session_id"])
        label = f"{run['step']} ({run['outcome']})" if run else row["session_id"][:8]
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
