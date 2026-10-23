import io
import json
import os
import stat
import tempfile
import time
import unittest
from pathlib import Path

from tools.plan import driver


def _write_package(root, package, steps):
    directory = Path(root) / "docs" / "components" / package
    (directory / "steps").mkdir(parents=True)
    for step in steps:
        lines = [f'{k} = {json.dumps(v)}' for k, v in step.items()]
        (directory / "steps" / f"{step['id']}.toml").write_text("\n".join(lines) + "\n")
    return directory


def _step(step_id, **fields):
    return {"id": step_id, "title": step_id, "status": "pending", "evidence": "", **fields}


class CandidatesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def ids(self, **kwargs):
        return [c.step.id for c in driver.candidates(self.root, **kwargs)]

    def test_only_pending_steps_with_done_dependencies_are_ready(self):
        _write_package(self.root, "a", [
            _step("a.one", status="done"),
            _step("a.two", depends_on=["a.one"]),
            _step("a.three", depends_on=["a.two"]),
            _step("a.four", status="at-pr"),
        ])
        self.assertEqual(self.ids(), ["a.two"])

    def test_bugs_then_milestone_steps_then_the_rest_across_packages(self):
        _write_package(self.root, "a", [_step("a.plain")])
        _write_package(self.root, "b", [_step("b.mvp", milestone="mvp")])
        _write_package(self.root, "c", [_step("c.bug", kind="bug")])
        self.assertEqual(self.ids(), ["c.bug", "b.mvp", "a.plain"])

    def test_skipped_ids_are_left_out(self):
        _write_package(self.root, "a", [_step("a.one"), _step("a.two")])
        self.assertEqual(self.ids(skip={"a.one"}), ["a.two"])

    def test_a_broken_manifest_does_not_hide_other_packages(self):
        _write_package(self.root, "a", [_step("a.one", depends_on="nope")])
        _write_package(self.root, "b", [_step("b.one")])
        self.assertEqual(self.ids(), ["b.one"])


def _run(step, outcome, **extra):
    return {"event": "run", "step": step, "outcome": outcome, "session_id": step, **extra}


class SkipIdsTest(unittest.TestCase):
    def test_settled_outcomes_are_skipped(self):
        records = [_run("a", "merged"), _run("b", "pr-open"), _run("c", "parked")]
        self.assertEqual(driver.skip_ids(records), {"a", "b", "c"})

    def test_one_failure_is_retried_and_two_are_not(self):
        self.assertEqual(driver.skip_ids([_run("a", "failed")]), set())
        self.assertEqual(driver.skip_ids([_run("a", "failed"), _run("a", "timeout")]), {"a"})

    def test_release_clears_a_parked_step_and_its_failures(self):
        records = [_run("a", "parked"), {"event": "release", "step": "a"}]
        self.assertEqual(driver.skip_ids(records), set())
        records = [_run("a", "failed"), _run("a", "failed"), {"event": "release", "step": "a"}]
        self.assertEqual(driver.skip_ids(records), set())


class ParseOutcomeTest(unittest.TestCase):
    def test_last_marker_line_wins(self):
        text = "work\nDRIVER-OUTCOME: parked: early\nmore\nDRIVER-OUTCOME: pr-open https://x/pull/1\n"
        self.assertEqual(driver.parse_outcome(text), ("pr-open", "https://x/pull/1"))

    def test_parked_keeps_its_reason(self):
        self.assertEqual(
            driver.parse_outcome("DRIVER-OUTCOME: parked: needs an owner decision on X"),
            ("parked", "needs an owner decision on X"),
        )

    def test_missing_or_unknown_marker_is_unreported(self):
        self.assertEqual(driver.parse_outcome("all done!")[0], "unreported")
        self.assertEqual(driver.parse_outcome("DRIVER-OUTCOME: great")[0], "unreported")
        self.assertEqual(driver.parse_outcome("")[0], "unreported")


class CommandTest(unittest.TestCase):
    def test_command_carries_the_session_id_budget_and_mode(self):
        options = driver.Options(budget=7.5, permission_mode="acceptEdits", model="opus")
        command = driver.build_command("PROMPT", "sid", options)
        self.assertEqual(command[:3], ["claude", "-p", "PROMPT"])
        for flag, value in (
            ("--session-id", "sid"),
            ("--max-budget-usd", "7.5"),
            ("--permission-mode", "acceptEdits"),
            ("--model", "opus"),
            ("--output-format", "json"),
        ):
            self.assertEqual(command[command.index(flag) + 1], value)

    def test_model_is_omitted_when_unset(self):
        self.assertNotIn("--model", driver.build_command("p", "s", driver.Options()))

    def test_prompt_names_the_step_the_acceptance_kind_and_existing_work(self):
        step = driver.ledger.Step("pkg.x", "Do x", "pending", "", acceptance="product")
        prompt = driver.build_prompt(step, "pkg")
        self.assertIn("pkg.x", prompt)
        self.assertIn("acceptance kind is product", prompt)
        self.assertIn("continue from it", prompt)
        self.assertIn("tools.plan.ci_wait", prompt)
        self.assertIn("step-loop", prompt)
        self.assertIn("DRIVER-OUTCOME:", prompt)


class FakeRun:
    def __init__(self, outcomes):
        self.outcomes = list(outcomes)
        self.steps = []

    def __call__(self, candidate, options, root):
        self.steps.append(candidate.step.id)
        outcome = self.outcomes.pop(0)
        return {"event": "run", "step": candidate.step.id, "outcome": outcome,
                "session_id": candidate.step.id, "cost_usd": 1.0, "detail": ""}


class LoopTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / "repo"
        self.runs = Path(self.tmp.name) / "runs.jsonl"
        _write_package(self.root, "a", [_step(f"a.s{i}") for i in range(5)])

    def loop(self, outcomes, **options):
        fake = FakeRun(outcomes)
        records = driver.run_loop(
            self.root, driver.Options(**options), self.runs, run_step=fake,
            prepare=lambda root: None, pr_state=lambda url: "OPEN", in_flight=lambda ids: set(),
        )
        return fake, records

    def test_runs_until_max_steps_and_records_each_run(self):
        fake, records = self.loop(["merged"] * 5, max_steps=3)
        self.assertEqual(len(fake.steps), 3)
        stored = driver.read_records(self.runs)
        self.assertEqual([r["event"] for r in stored].count("run"), 3)
        self.assertEqual([r["event"] for r in stored].count("start"), 3)
        self.assertEqual(len(records), 3)

    def test_two_consecutive_failures_stop_the_run(self):
        fake, _ = self.loop(["failed", "timeout", "merged"], max_steps=5)
        self.assertEqual(len(fake.steps), 2)

    def test_a_success_resets_the_failure_streak(self):
        fake, _ = self.loop(["failed", "merged", "failed", "merged", "merged"], max_steps=5)
        self.assertEqual(len(fake.steps), 5)

    def test_parked_steps_are_not_picked_again_and_do_not_stop_the_run(self):
        fake, _ = self.loop(["parked", "merged"], max_steps=2)
        self.assertEqual(fake.steps, ["a.s0", "a.s1"])

    def test_only_runs_exactly_that_step(self):
        fake, _ = self.loop(["merged"], max_steps=5, only="a.s3")
        self.assertEqual(fake.steps, ["a.s3"])

    def test_an_unknown_only_step_runs_nothing(self):
        fake, _ = self.loop([], max_steps=5, only="nope")
        self.assertEqual(fake.steps, [])

    def test_dry_run_starts_nothing(self):
        fake, records = self.loop([], max_steps=2, dry_run=True)
        self.assertEqual(fake.steps, [])
        self.assertEqual(records, [])

    def test_prepare_failure_stops_before_any_step(self):
        def refuse(root):
            raise driver.PreflightError("dirty")
        fake = FakeRun(["merged"])
        with self.assertRaises(driver.PreflightError):
            driver.run_loop(self.root, driver.Options(), self.runs, run_step=fake, prepare=refuse)
        self.assertEqual(fake.steps, [])

    def test_preflight_failing_after_a_step_ends_the_run_with_its_records(self):
        calls = []

        def refuse_second(root):
            calls.append(root)
            if len(calls) > 1:
                raise driver.PreflightError("dirty")
        fake = FakeRun(["merged", "merged"])
        records = driver.run_loop(
            self.root, driver.Options(max_steps=3), self.runs, run_step=fake,
            prepare=refuse_second, pr_state=lambda url: "OPEN", in_flight=lambda ids: set(),
        )
        self.assertEqual(len(records), 1)

    def test_steps_named_by_an_open_pr_are_not_started(self):
        fake = FakeRun(["merged"])
        driver.run_loop(
            self.root, driver.Options(max_steps=1), self.runs, run_step=fake,
            prepare=lambda root: None, pr_state=lambda url: "OPEN",
            in_flight=lambda ids: {"a.s0", "a.s1"},
        )
        self.assertEqual(fake.steps, ["a.s2"])

    def test_total_budget_stops_the_run(self):
        fake, records = self.loop(["merged"] * 5, max_steps=5, total_budget=2.5)
        self.assertEqual(len(records), 3)

    def test_a_pr_closed_unmerged_releases_its_step(self):
        driver.append_record(self.runs, {
            "event": "run", "step": "a.s0", "outcome": "pr-open",
            "detail": "https://github.com/o/r/pull/9", "session_id": "x",
        })
        fake = FakeRun(["merged"])
        driver.run_loop(
            self.root, driver.Options(max_steps=1), self.runs, run_step=fake,
            prepare=lambda root: None, pr_state=lambda url: "CLOSED", in_flight=lambda ids: set(),
        )
        self.assertEqual(fake.steps, ["a.s0"])

    def test_a_pr_still_open_keeps_its_step_skipped(self):
        driver.append_record(self.runs, {
            "event": "run", "step": "a.s0", "outcome": "pr-open",
            "detail": "https://github.com/o/r/pull/9", "session_id": "x",
        })
        fake, _ = self.loop(["merged"], max_steps=1)
        self.assertEqual(fake.steps, ["a.s1"])

    def test_run_records_are_owner_only(self):
        self.loop(["merged"], max_steps=1)
        self.assertEqual(stat.S_IMODE(self.runs.stat().st_mode), 0o600)


PR = "https://github.com/o/r/pull/2"


class ExecuteTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name) / "out"

    def execute(self, result, state="MERGED", acceptance="code", **options):
        candidate = driver.Candidate(
            driver.ledger.Step("p.s", "t", "pending", "", acceptance=acceptance), "p"
        )
        return driver.execute(
            candidate, driver.Options(**options), Path(self.tmp.name),
            process=lambda command, cwd, timeout, on_tick=None: result, out_dir=self.out,
            pr_state=lambda url: state, announce=lambda *a, **k: None,
        )

    def answer(self, text, **extra):
        return driver.Completed(0, json.dumps({"result": text, "is_error": False, **extra}), "")

    def test_a_verified_claim_is_the_reported_outcome_with_its_metrics(self):
        result = self.answer(
            f"x\nDRIVER-OUTCOME: merged {PR}", total_cost_usd=1.25, num_turns=40,
            duration_ms=5000, permission_denials=[{}, {}],
        )
        record = self.execute(result)
        self.assertEqual((record["outcome"], record["detail"]), ("merged", PR))
        self.assertEqual((record["cost_usd"], record["turns"], record["denials"]), (1.25, 40, 2))

    def test_the_prs_real_state_overrides_the_claim(self):
        record = self.execute(self.answer(f"DRIVER-OUTCOME: merged {PR}"), state="OPEN")
        self.assertEqual(record["outcome"], "pr-open")

    def test_a_closed_pr_is_a_failure(self):
        record = self.execute(self.answer(f"DRIVER-OUTCOME: pr-open {PR}"), state="CLOSED")
        self.assertEqual(record["outcome"], "failed")

    def test_a_claim_without_a_pr_url_is_unreported(self):
        self.assertEqual(self.execute(self.answer("DRIVER-OUTCOME: merged"))["outcome"], "unreported")

    def test_an_unreadable_pr_is_unreported(self):
        record = self.execute(self.answer(f"DRIVER-OUTCOME: merged {PR}"), state=None)
        self.assertEqual(record["outcome"], "unreported")

    def test_merging_a_product_step_is_a_violation(self):
        record = self.execute(self.answer(f"DRIVER-OUTCOME: merged {PR}"), acceptance="product")
        self.assertEqual(record["outcome"], "violation")

    def test_parked_needs_no_pr(self):
        record = self.execute(self.answer("DRIVER-OUTCOME: parked: which signing key?"))
        self.assertEqual((record["outcome"], record["detail"]), ("parked", "which signing key?"))

    def test_is_error_is_a_failure_even_with_a_marker(self):
        result = driver.Completed(
            0, json.dumps({"result": f"DRIVER-OUTCOME: merged {PR}", "is_error": True}), ""
        )
        self.assertEqual(self.execute(result)["outcome"], "failed")

    def test_nonzero_exit_or_garbage_output_is_a_failure(self):
        self.assertEqual(self.execute(driver.Completed(1, "", "boom"))["outcome"], "failed")
        self.assertEqual(self.execute(driver.Completed(0, "not json", ""))["outcome"], "failed")

    def test_timeout_is_recorded_and_its_partial_output_kept_owner_only(self):
        record = self.execute(driver.TimedOut("partial", ""))
        self.assertEqual(record["outcome"], "timeout")
        saved = self.out / f"{record['session_id']}.json"
        self.assertIn("partial", saved.read_text())
        self.assertEqual(stat.S_IMODE(saved.stat().st_mode), 0o600)


class ProcessTest(unittest.TestCase):
    def test_timeout_kills_the_whole_process_group(self):
        started = time.monotonic()
        result = driver.run_process(["sh", "-c", "sleep 30 & sleep 30"], Path.cwd(), 1)
        self.assertIsInstance(result, driver.TimedOut)
        self.assertLess(time.monotonic() - started, 10)

    def test_output_and_exit_code_are_returned(self):
        result = driver.run_process(["sh", "-c", "echo hi; exit 3"], Path.cwd(), 10)
        self.assertEqual((result.returncode, result.stdout.strip()), (3, "hi"))


class ProcessSafetyTest(unittest.TestCase):
    def alive(self, pid):
        try:
            os.kill(pid, 0)
        except OSError:
            return False
        return True

    def test_a_failing_progress_hook_does_not_stop_the_run(self):
        def broken(elapsed):
            raise RuntimeError("boom")
        result = driver.run_process(["sleep", "2"], Path.cwd(), 30, on_tick=broken, interval=0.5)
        self.assertIsInstance(result, driver.Completed)

    def test_a_permanently_failing_hook_is_reported_once(self):
        def broken(elapsed):
            raise RuntimeError("boom")
        captured = io.StringIO()
        with driver.contextlib.redirect_stderr(captured):
            driver.run_process(["sleep", "2"], Path.cwd(), 30, on_tick=broken, interval=0.3)
        self.assertEqual(captured.getvalue().count("progress unavailable"), 1)

    def test_an_interrupt_kills_the_child_before_propagating(self):
        with tempfile.TemporaryDirectory() as tmp:
            pidfile = Path(tmp) / "pid"

            def interrupt(elapsed):
                raise KeyboardInterrupt

            with self.assertRaises(KeyboardInterrupt):
                driver.run_process(
                    ["sh", "-c", f"echo $$ > {pidfile}; sleep 30"], Path.cwd(), 30,
                    on_tick=interrupt, interval=1,
                )
            time.sleep(0.2)
            self.assertFalse(self.alive(int(pidfile.read_text())))


class HeartbeatTest(unittest.TestCase):
    def test_a_slow_process_reports_elapsed_time_while_it_runs(self):
        ticks = []
        driver.run_process(["sleep", "2"], Path.cwd(), 30, on_tick=ticks.append, interval=0.5)
        self.assertGreaterEqual(len(ticks), 2)
        self.assertTrue(all(b > a for a, b in zip(ticks, ticks[1:])))

    def test_timeout_still_applies_between_heartbeats(self):
        started = time.monotonic()
        result = driver.run_process(["sleep", "30"], Path.cwd(), 1, on_tick=lambda e: None, interval=0.3)
        self.assertIsInstance(result, driver.TimedOut)
        self.assertLess(time.monotonic() - started, 5)

    def test_execute_announces_the_session_id_and_ticks(self):
        lines = []
        candidate = driver.Candidate(driver.ledger.Step("p.s", "t", "pending", ""), "p")

        def process(command, cwd, timeout, on_tick=None):
            on_tick(31)
            return driver.TimedOut()

        with tempfile.TemporaryDirectory() as tmp:
            record = driver.execute(
                candidate, driver.Options(), Path(tmp), process=process, out_dir=Path(tmp) / "o",
                announce=lambda text, **k: lines.append(text),
                narrate=lambda sid, announce: driver.argparse.Namespace(
                    tick=lambda e: announce(f"tick {e}")),
            )
        self.assertIn(f"session {record['session_id']}", lines[0])
        self.assertEqual(lines[1], "tick 31")


def _tool_use_line(tool_id, name, **tool_input):
    return json.dumps({"type": "assistant", "timestamp": "2026-10-01T10:00:00.000Z", "message": {
        "content": [{"type": "tool_use", "id": tool_id, "name": name, "input": tool_input}]}})


def _tool_error_line(tool_id):
    return json.dumps({"type": "user", "timestamp": "2026-10-01T10:00:01.000Z", "message": {
        "content": [{"type": "tool_result", "tool_use_id": tool_id, "is_error": True,
                     "content": "SECRET failure text"}]}})


class NarratorTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        (Path(self.tmp.name) / "proj").mkdir()
        self.transcript = Path(self.tmp.name) / "proj" / "sid.jsonl"
        original = driver.context.PROJECTS
        driver.context.PROJECTS = Path(self.tmp.name)
        self.addCleanup(setattr, driver.context, "PROJECTS", original)
        self.lines = []
        self.narrator = driver.Narrator("sid", lambda text, **k: self.lines.append(text))

    def write(self, *entries, mode="a"):
        with self.transcript.open(mode) as out:
            out.write("".join(e + "\n" for e in entries))

    def test_new_tool_calls_are_printed_once_in_order(self):
        self.write(_tool_use_line("1", "Bash", command="git status"), _tool_use_line("2", "Skill", skill="step-loop"))
        self.narrator.tick(5)
        self.narrator.tick(10)
        self.write(_tool_use_line("3", "Edit", file_path="/r/tools/plan/loop.py"))
        self.narrator.tick(15)
        self.assertEqual(self.lines, [
            "  5s Bash(git)", "  5s Skill(step-loop)", "  15s Edit(loop.py)",
        ])

    def test_a_failed_tool_is_named_without_its_output(self):
        self.write(_tool_use_line("1", "Bash", command="cargo test"), _tool_error_line("1"))
        self.narrator.tick(5)
        self.assertEqual(self.lines[-1], "  5s ! Bash(cargo) failed")
        self.assertNotIn("SECRET", " ".join(self.lines))

    def test_a_half_written_line_waits_for_its_newline(self):
        line = _tool_use_line("1", "Bash", command="ls")
        self.transcript.write_text(line[:20])
        self.narrator.tick(5)
        self.assertEqual(self.lines, [])
        self.transcript.write_text(line + "\n")
        self.narrator.tick(10)
        self.assertEqual(self.lines, ["  10s Bash(ls)"])

    def test_a_quiet_session_gets_a_heartbeat_not_silence(self):
        self.write(_tool_use_line("1", "Bash", command="ls"))
        self.narrator.tick(5)
        self.narrator.tick(20)
        self.assertEqual(len(self.lines), 1)
        self.narrator.tick(40)
        self.assertIn("idle", self.lines[-1])

    def test_malformed_lines_are_skipped_not_fatal(self):
        self.transcript.write_text("\n".join([
            "[1]", '{"message": "hi"}',
            '{"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "9", "name": "Bash", "input": "x"}]}}',
            _tool_use_line("1", "Bash", command="ls"),
        ]) + "\n")
        self.narrator.tick(5)
        self.assertEqual(self.lines, ["  5s Bash", "  5s Bash(ls)"])

    def test_odd_shapes_never_stop_the_narrator(self):
        odd = (None, 7, "s", [], ["x"], {}, [[]], True)
        entries = []
        for value in odd:
            entries.append({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": value, "name": value, "input": value}]}})
            entries.append({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": value, "is_error": True}]}})
            entries.append({"type": "assistant", "message": {"content": value}})
        entries.append(json.loads(_tool_use_line("9", "Bash", command="ls")))
        self.transcript.write_text("".join(json.dumps(e) + "\n" for e in entries))
        self.narrator.tick(5)
        self.assertIn("  5s Bash(ls)", self.lines)

    def test_a_replaced_shorter_transcript_is_read_from_the_start(self):
        self.write(_tool_use_line("1", "Bash", command="ls"))
        self.narrator.tick(5)
        self.write(_tool_use_line("2", "Skill", skill="x"), mode="w")
        self.narrator.tick(10)
        self.assertEqual(self.lines[-1], "  10s Skill(x)")

    def test_nothing_is_printed_before_the_transcript_exists(self):
        self.narrator.tick(5)
        self.assertEqual(self.lines, [])


class DigestFailureTest(unittest.TestCase):
    def test_a_failing_digest_does_not_abort_the_loop(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            _write_package(root, "a", [_step("a.s0"), _step("a.s1")])
            original = driver.finish_line
            driver.finish_line = lambda sid: (_ for _ in ()).throw(ValueError("bad"))
            self.addCleanup(setattr, driver, "finish_line", original)
            fake = FakeRun(["merged", "merged"])
            records = driver.run_loop(
                root, driver.Options(max_steps=2), Path(tmp) / "runs.jsonl", run_step=fake,
                prepare=lambda r: None, pr_state=lambda u: "OPEN", in_flight=lambda ids: set(),
            )
        self.assertEqual(len(records), 2)


class FinishLineTest(unittest.TestCase):
    def test_summarizes_a_finished_session(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "proj").mkdir()
            (Path(tmp) / "proj" / "sid.jsonl").write_text(
                _tool_use_line("1", "Agent", subagent_type="x") + "\n" + _tool_error_line("1") + "\n"
            )
            original = driver.context.PROJECTS
            driver.context.PROJECTS = Path(tmp)
            self.addCleanup(setattr, driver.context, "PROJECTS", original)
            line = driver.finish_line("sid")
        self.assertIn("1 tool calls", line)
        self.assertIn("1 subagents", line)
        self.assertIn("1 tool errors", line)

    def test_unknown_session_gives_no_line(self):
        self.assertEqual(driver.finish_line("no-such-session"), "")


class ProgressLineTest(unittest.TestCase):
    def test_line_shows_calls_last_tool_and_idle_time(self):
        with tempfile.TemporaryDirectory() as tmp:
            projects = Path(tmp)
            (projects / "proj").mkdir()
            now = driver.datetime.now(driver.timezone.utc).isoformat()
            entry = {"type": "assistant", "timestamp": now, "message": {
                "content": [{"type": "tool_use", "id": "1", "name": "Bash",
                             "input": {"command": "git push -u origin x"}}]}}
            (projects / "proj" / "sid.jsonl").write_text(json.dumps(entry) + "\n")
            original = driver.context.PROJECTS
            driver.context.PROJECTS = projects
            self.addCleanup(setattr, driver.context, "PROJECTS", original)
            line = driver.progress_line("sid", 75)
        self.assertIn("1m15s", line)
        self.assertIn("1 tool calls", line)
        self.assertIn("last Bash(git)", line)
        self.assertIn("idle", line)
        self.assertNotIn("origin", line)

    def test_line_before_the_transcript_exists(self):
        self.assertIn("starting", driver.progress_line("no-such-session", 5))


class TailTest(unittest.TestCase):
    def test_a_session_id_with_path_characters_is_refused(self):
        for bad in ("../../out", "a/b", "x y", ""):
            self.assertEqual(driver.tail_session(bad or "?/", sleep=lambda s: None), 1, bad)


class PidTest(unittest.TestCase):
    def test_reserved_and_malformed_pids_are_not_alive(self):
        for pid in (0, 1, -5, "x", None):
            self.assertFalse(driver.pid_alive(pid), pid)
        self.assertTrue(driver.pid_alive(os.getpid()))


class PruneTest(unittest.TestCase):
    def test_old_outputs_are_removed_and_new_ones_kept(self):
        with tempfile.TemporaryDirectory() as tmp:
            old, new = Path(tmp) / "old.json", Path(tmp) / "new.json"
            old.write_text("{}")
            new.write_text("{}")
            ancient = time.time() - (driver.OUT_RETENTION_DAYS + 1) * 86400
            os.utime(old, (ancient, ancient))
            driver.prune_out(Path(tmp))
            self.assertFalse(old.exists())
            self.assertTrue(new.exists())


class ArgumentTest(unittest.TestCase):
    def test_budgets_must_be_positive_and_finite(self):
        for bad in ("nan", "inf", "-1", "0"):
            with self.assertRaises(driver.argparse.ArgumentTypeError):
                driver._positive(bad)
        self.assertEqual(driver._positive("2.5"), 2.5)

    def test_unattended_modes_exclude_bypass(self):
        self.assertNotIn("bypassPermissions", driver.PERMISSION_MODES)


class StatusTest(unittest.TestCase):
    def test_status_lists_parked_reasons_and_total_cost(self):
        records = [
            {**_run("a", "merged"), "cost_usd": 2.0, "started": "2026-10-01T00:00:00+00:00", "detail": ""},
            {**_run("b", "parked"), "cost_usd": 1.0, "started": "2026-10-01T01:00:00+00:00",
             "detail": "needs owner call"},
        ]
        text = driver.status(records)
        self.assertIn("needs owner call", text)
        self.assertIn("$3.00", text)

    def test_status_lists_steps_that_failed_twice(self):
        records = [
            {**_run("a", "failed"), "started": "2026-10-01T00:00:00+00:00", "detail": "boom"},
            {**_run("a", "timeout"), "started": "2026-10-01T01:00:00+00:00", "detail": "over 60s"},
        ]
        text = driver.status(records)
        self.assertIn("Failed 2 times", text)
        self.assertIn("a (timeout): over 60s", text)

    def test_status_shows_a_live_run_and_an_interrupted_one(self):
        records = [
            {"event": "start", "step": "a", "pid": 1, "started": "2026-10-01T00:00:00+00:00"},
            {"event": "start", "step": "b", "pid": 2, "started": "2026-10-01T00:00:00+00:00"},
            {**_run("c", "merged"), "started": "2026-10-01T00:00:00+00:00"},
        ]
        text = driver.status(records, alive=lambda pid: pid == 1)
        self.assertIn("a: running since", text)
        self.assertIn("b: interrupted", text)

    def test_a_release_clears_an_interrupted_start(self):
        records = [
            {"event": "start", "step": "a", "pid": 2, "started": "2026-10-01T00:00:00+00:00"},
            {"event": "release", "step": "a"},
            {**_run("c", "merged"), "started": "2026-10-01T00:00:00+00:00"},
        ]
        self.assertNotIn("interrupted", driver.status(records, alive=lambda pid: False))

    def test_a_finished_run_clears_its_start(self):
        records = [
            {"event": "start", "step": "a", "pid": 1, "started": "2026-10-01T00:00:00+00:00"},
            {**_run("a", "merged"), "started": "2026-10-01T00:00:00+00:00"},
        ]
        self.assertNotIn("running", driver.status(records, alive=lambda pid: True))

    def test_status_with_no_runs(self):
        self.assertIn("no runs", driver.status([]))


if __name__ == "__main__":
    unittest.main()
