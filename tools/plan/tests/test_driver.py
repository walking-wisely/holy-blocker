import json
import tempfile
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

    def test_prompt_names_the_step_and_the_marker_contract(self):
        step = driver.ledger.Step("pkg.x", "Do x", "pending", "")
        prompt = driver.build_prompt(step, "pkg")
        self.assertIn("pkg.x", prompt)
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
            self.root, driver.Options(**options), self.runs, run_step=fake, prepare=lambda root: None
        )
        return fake, records

    def test_runs_until_max_steps_and_records_each_run(self):
        fake, records = self.loop(["merged"] * 5, max_steps=3)
        self.assertEqual(len(fake.steps), 3)
        self.assertEqual(len(driver.read_records(self.runs)), 3)
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


class ExecuteTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.candidate = driver.Candidate(driver.ledger.Step("p.s", "t", "pending", ""), "p")

    def execute(self, result, **options):
        def fake_process(command, cwd, timeout):
            return result
        return driver.execute(
            self.candidate, driver.Options(**options), Path(self.tmp.name), process=fake_process,
            out_dir=Path(self.tmp.name) / "out",
        )

    def test_success_with_marker_is_the_reported_outcome(self):
        payload = {"result": "x\nDRIVER-OUTCOME: merged https://x/pull/2", "is_error": False,
                   "total_cost_usd": 1.25, "num_turns": 40, "duration_ms": 5000,
                   "permission_denials": [{}, {}]}
        record = self.execute(driver.Completed(0, json.dumps(payload), ""))
        self.assertEqual((record["outcome"], record["detail"]), ("merged", "https://x/pull/2"))
        self.assertEqual((record["cost_usd"], record["turns"], record["denials"]), (1.25, 40, 2))

    def test_is_error_is_a_failure_even_with_a_marker(self):
        payload = {"result": "DRIVER-OUTCOME: merged x", "is_error": True}
        self.assertEqual(self.execute(driver.Completed(0, json.dumps(payload), ""))["outcome"], "failed")

    def test_nonzero_exit_or_garbage_output_is_a_failure(self):
        self.assertEqual(self.execute(driver.Completed(1, "", "boom"))["outcome"], "failed")
        self.assertEqual(self.execute(driver.Completed(0, "not json", ""))["outcome"], "failed")

    def test_timeout_is_recorded(self):
        self.assertEqual(self.execute(driver.TimedOut())["outcome"], "timeout")

    def test_raw_output_is_kept_for_investigation(self):
        record = self.execute(driver.Completed(1, "out", "err"))
        saved = (Path(self.tmp.name) / "out" / f"{record['session_id']}.json").read_text()
        self.assertIn("out", saved)


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

    def test_status_with_no_runs(self):
        self.assertIn("no runs", driver.status([]))


if __name__ == "__main__":
    unittest.main()
