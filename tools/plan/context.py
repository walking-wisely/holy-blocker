"""Context-budget gate: how many tokens the current session has used.

Usage:
    python -m tools.plan.context [transcript.jsonl]

Prints the token count and an `under`/`over` verdict against THRESHOLD. Exit
code 0 is under, 1 is over, 2 is no readable transcript. With no path, this session's
transcript (CLAUDE_CODE_SESSION_ID) under ~/.claude/projects/ is used. Only
when the variable is unset does it fall back to the newest transcript for this checkout.
A subagent inherits its parent's session id and so measures the parent: run the gate from
the main loop session only.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import subprocess
import sys
from pathlib import Path

THRESHOLD = 100_000
USAGE_KEYS = ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens")
SYNTHETIC_MODEL = "<synthetic>"
PROJECTS = Path.home() / ".claude" / "projects"


def last_assistant_tokens(path: Path) -> int | None:
    """Input-side tokens of the last main-thread assistant line, or None when there is no real count."""
    used = None
    with Path(path).open(encoding="utf-8") as lines:
        for line in lines:
            if "assistant" not in line:
                continue
            try:
                entry = json.loads(line)
                if entry["type"] != "assistant" or entry.get("isSidechain"):
                    continue
                message = entry["message"]
                if message.get("model") == SYNTHETIC_MODEL:
                    continue
                usage = message["usage"]
                total = sum(usage.get(key) or 0 for key in USAGE_KEYS)
            except (ValueError, KeyError, TypeError, AttributeError):
                continue
            if total:
                used = total
    return used


def verdict(tokens: int, threshold: int = THRESHOLD) -> str:
    return "over" if tokens > threshold else "under"


def project_slug(path: str) -> str:
    return re.sub(r"[^A-Za-z0-9]", "-", path)


def newest_transcript(directories: list[Path]) -> Path | None:
    found = [p for d in directories if d.is_dir() for p in d.glob("*.jsonl")]
    return max(found, key=lambda p: p.stat().st_mtime, default=None)


def session_transcript(session_id: str, projects: Path) -> Path | None:
    matches = projects.glob(f"*/{glob.escape(session_id)}.jsonl")
    return max(matches, key=lambda p: p.stat().st_mtime, default=None)


def _checkout_roots() -> list[str]:
    """This directory and the main checkout, since a session may start in either."""
    roots = [str(Path.cwd())]
    common = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        capture_output=True,
        text=True,
    )
    if common.returncode == 0:
        roots.append(str(Path(common.stdout.strip()).parent))
    return roots


def _project_dirs() -> list[Path]:
    return [PROJECTS / project_slug(root) for root in _checkout_roots()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m tools.plan.context")
    parser.add_argument("transcript", nargs="?")
    args = parser.parse_args(argv)

    if args.transcript:
        transcript: Path | None = Path(args.transcript)
    else:
        session_id = os.environ.get("CLAUDE_CODE_SESSION_ID")
        if session_id:
            transcript = session_transcript(session_id, PROJECTS)
            if transcript is None:
                print(f"no transcript for session {session_id}", file=sys.stderr)
                return 2
        else:
            print("CLAUDE_CODE_SESSION_ID unset; using the newest transcript", file=sys.stderr)
            transcript = newest_transcript(_project_dirs())
    if transcript is None:
        print("no transcript found", file=sys.stderr)
        return 2
    try:
        tokens = last_assistant_tokens(transcript)
    except OSError as err:
        print(f"cannot read {transcript}: {err}", file=sys.stderr)
        return 2
    if tokens is None:
        print(f"no assistant message in {transcript}", file=sys.stderr)
        return 2
    state = verdict(tokens)
    print(f"{tokens} {state} (threshold {THRESHOLD})")
    return 1 if state == "over" else 0


if __name__ == "__main__":
    sys.exit(main())
