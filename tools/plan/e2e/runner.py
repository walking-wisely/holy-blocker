"""Local e2e runner: select scenarios for a diff, run them, decide the pass, record a git note.

See docs/decisions/e2e-scenario-contract.md §2-§4 and §7.
"""

import argparse
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

from tools.plan.e2e import manifest

EXIT_PASS, EXIT_FAIL, EXIT_ENV, EXIT_UNVERIFIED = 0, 1, 2, 3
MAX_LINE = 200
RUN_TIMEOUT_SECONDS = 1800
NOTES_NAMESPACE = "refs/notes/hb-e2e"
FALLBACK_NAME = "hb-e2e"
FALLBACK_EMAIL = "hb-e2e@localhost"

_ASSERT = re.compile(r"^ASSERT (blocked|permitted) (\d+)$", re.MULTILINE)
_ARTIFACT = re.compile(r"^ARTIFACT sha256 ([0-9a-f]{64})$", re.MULTILINE)
_HOST = re.compile(
    r"^HOST ((?:android|macos|windows)-[a-z0-9.]+(?: (?:arm64|x86_64|aarch64|AVD|emulator|simulator))*)$",
    re.MULTILINE,
)
_FINAL = re.compile(r"^[A-Z][A-Z ]{0,30}(?:: [a-z0-9 _-]{1,60})?$")
WITHHELD = "(final line withheld)"
_SCENARIO_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


class RecordRefused(RuntimeError):
    pass


@dataclass(frozen=True)
class Verdict:
    result: str
    artifact: str | None
    host_class: str | None


@dataclass(frozen=True)
class Outcome:
    status: str
    final_line: str
    verdict: Verdict | None = None


def _git(root: Path, *args: str, stdin: str | None = None) -> str:
    done = subprocess.run(
        ["git", *args], cwd=root, input=stdin, capture_output=True, text=True, check=True
    )
    return done.stdout.strip()


def _covered(path: str, covers: tuple[str, ...]) -> bool:
    return any(path == c or path.startswith(c + "/") for c in covers)


def select(scenarios: list[manifest.Scenario], paths: list[str]):
    selected = [s for s in scenarios if any(_covered(p, s.covers) for p in paths)]
    uncovered = sum(1 for p in paths if not any(_covered(p, s.covers) for s in scenarios))
    return selected, uncovered


def coverage_line(selected: list[manifest.Scenario], uncovered: int) -> str:
    line = f"{uncovered} changed paths covered by no scenario"
    if not selected:
        return f"UNVERIFIED: no scenario covers this diff; {line}"
    return line


def final_line(text: str) -> str:
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if not lines:
        return ""
    last = lines[-1][:MAX_LINE]
    return last if _FINAL.match(last) else WITHHELD


def decide(exit_code: int, stdout: str) -> Verdict:
    counts = {"blocked": 0, "permitted": 0}
    for direction, number in _ASSERT.findall(stdout):
        counts[direction] += int(number)
    passed = exit_code == EXIT_PASS and counts["blocked"] > 0 and counts["permitted"] > 0
    artifact = _ARTIFACT.search(stdout)
    host = _HOST.search(stdout)
    return Verdict(
        "pass" if passed else "fail",
        artifact.group(1) if artifact else None,
        host.group(1) if host else None,
    )


def covered_digest(root: Path, covers: tuple[str, ...]) -> str:
    lines = [f"{path}\t{_git(root, 'rev-parse', f'HEAD:{path}')}" for path in sorted(covers)]
    return _git(root, "hash-object", "-w", "--stdin", stdin="\n".join(lines) + "\n")


def dirty(root: Path, covers: tuple[str, ...]) -> bool:
    status = _git(
        root, "status", "--porcelain", "--untracked-files=all", "--ignore-submodules=none", "--", *covers
    )
    hidden = [
        line for line in _git(root, "ls-files", "-v", "--", *covers).splitlines()
        if line[:1] != "H"
    ]
    return bool(status or hidden)


def _ref(scenario_id: str) -> str:
    ref = f"{NOTES_NAMESPACE}/{scenario_id}"
    valid = subprocess.run(
        ["git", "check-ref-format", ref], capture_output=True
    ).returncode == 0
    if not _SCENARIO_ID.match(scenario_id) or not valid:
        raise RecordRefused(f"scenario id {scenario_id!r} is not usable in a ref name")
    return ref


def _identity(root: Path) -> list[str]:
    configured = subprocess.run(
        ["git", "var", "GIT_COMMITTER_IDENT"], cwd=root, capture_output=True, text=True
    )
    if configured.returncode == 0:
        return []
    return ["-c", f"user.name={FALLBACK_NAME}", "-c", f"user.email={FALLBACK_EMAIL}"]


def record(
    root: Path, scenario: manifest.Scenario, verdict: Verdict, date: str, expected: str | None = None
) -> str:
    if verdict.result != "pass":
        raise RecordRefused("only a passing run is recorded")
    if not verdict.artifact or not verdict.host_class:
        raise RecordRefused("the run reported no artifact digest or host class")
    ref = _ref(scenario.id)
    if dirty(root, scenario.covers):
        raise RecordRefused("covered paths have uncommitted changes")
    try:
        digest = covered_digest(root, scenario.covers)
    except subprocess.CalledProcessError as error:
        raise RecordRefused("covered paths are not in HEAD") from error
    if expected is not None and digest != expected:
        raise RecordRefused("covered paths changed while the scenario ran")
    body = (
        f"result: pass\ndate: {date}\nhost_class: {verdict.host_class}\n"
        f"artifact_sha256: {verdict.artifact}\n"
    )
    try:
        _git(root, *_identity(root), "notes", f"--ref={ref}", "add", "-f", "-m", body, digest)
    except subprocess.CalledProcessError as error:
        raise RecordRefused("git refused the note") from error
    return digest


def read_note(root: Path, scenario_id: str, digest: str) -> dict[str, str] | None:
    try:
        text = _git(root, "notes", f"--ref={_ref(scenario_id)}", "show", digest)
    except subprocess.CalledProcessError:
        return None
    pairs = (line.split(": ", 1) for line in text.splitlines() if ": " in line)
    return {key: value for key, value in pairs}


def _execute(root: Path, command: str) -> tuple[int, str]:
    try:
        done = subprocess.run(
            shlex.split(command), cwd=root, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
            timeout=RUN_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        return EXIT_FAIL, "FAIL: timeout"
    except (OSError, ValueError):
        return EXIT_FAIL, "FAIL: command could not start"
    return done.returncode, done.stdout


def run_scenario(root: Path, scenario: manifest.Scenario) -> Outcome:
    code, out = _execute(root, scenario.preflight)
    if code == EXIT_ENV:
        return Outcome("env", final_line(out))
    if code != EXIT_PASS:
        return Outcome("fail", final_line(out))
    code, out = _execute(root, scenario.run)
    verdict = decide(code, out)
    return Outcome(verdict.result, final_line(out), verdict)


def _discover(root: Path) -> tuple[list[manifest.Scenario], list[str]]:
    found, skipped = [], []
    for path in sorted((root / "demos").glob("*/scenario.toml")):
        try:
            found.append(manifest.load(path))
        except manifest.ManifestError as error:
            skipped.append(path.relative_to(root).as_posix())
            print(f"unreadable manifest {skipped[-1]}: {error}", file=sys.stderr)
    return found, skipped


def _changed(root: Path, base: str, head: str) -> list[str]:
    raw = _git(root, "diff", "--name-only", "--no-renames", "-z", f"{base}...{head}")
    return [path for path in raw.split("\0") if path]


def main(argv: list[str], root: Path | None = None, out: Callable[[str], None] = print) -> int:
    parser = argparse.ArgumentParser(prog="tools.plan.e2e")
    parser.add_argument("command", choices=("select", "run"))
    parser.add_argument("--base", default="master")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--scenario")
    args = parser.parse_args(argv)
    root = root or Path(_git(Path.cwd(), "rev-parse", "--show-toplevel"))

    scenarios, skipped = _discover(root)
    if args.scenario:
        selected = [s for s in scenarios if s.id == args.scenario]
        uncovered = 0
        if not selected:
            out(f"unknown scenario {args.scenario}")
            return EXIT_FAIL
    else:
        try:
            selected, uncovered = select(scenarios, _changed(root, args.base, args.head))
        except subprocess.CalledProcessError:
            out(f"cannot diff {args.base}...{args.head}")
            return EXIT_FAIL
    out(coverage_line(selected, uncovered))
    for path in skipped:
        out(f"unreadable manifest {path}")
    base_code = EXIT_FAIL if skipped else EXIT_UNVERIFIED if not selected else EXIT_PASS
    if args.command == "select":
        for scenario in selected:
            out(scenario.id)
        return base_code

    exit_code = base_code
    date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    for scenario in selected:
        try:
            before = covered_digest(root, scenario.covers)
        except subprocess.CalledProcessError:
            out(f"{scenario.id}: FAIL: covered paths are not in HEAD")
            exit_code = EXIT_FAIL
            continue
        outcome = run_scenario(root, scenario)
        if outcome.status == "env":
            out(f"{scenario.id}: environment not ready: {outcome.final_line}")
            exit_code = EXIT_ENV if exit_code in (EXIT_PASS, EXIT_UNVERIFIED) else exit_code
        elif outcome.status == "fail":
            out(f"{scenario.id}: FAIL: {outcome.final_line}")
            exit_code = EXIT_FAIL
        else:
            try:
                digest = record(root, scenario, outcome.verdict, date, before)
                out(f"{scenario.id} @ {digest}: pass: {outcome.final_line}")
            except RecordRefused as refusal:
                out(f"{scenario.id}: pass, not recorded: {refusal}")
                exit_code = EXIT_FAIL
    return exit_code
