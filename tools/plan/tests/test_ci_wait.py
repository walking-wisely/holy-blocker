import json
import unittest

from tools.plan import ci_wait


def check(name, bucket, started="2026-10-01T10:00:00Z", workflow="CI"):
    return {"name": name, "bucket": bucket, "startedAt": started, "workflow": workflow}


def gate(bucket, **kw):
    return check("CI gate", bucket, **kw)


class EvaluateTest(unittest.TestCase):
    def state(self, checks):
        return ci_wait.evaluate(checks, "CI gate")[0]

    def test_gate_states_map_from_buckets(self):
        self.assertEqual(self.state([gate("pass")]), "pass")
        self.assertEqual(self.state([gate("fail")]), "fail")
        self.assertEqual(self.state([gate("cancel")]), "cancelled")
        self.assertEqual(self.state([gate("pending")]), "pending")

    def test_a_missing_gate_is_pending(self):
        self.assertEqual(self.state([check("other", "pass")]), "pending")
        self.assertEqual(self.state([]), "pending")

    def test_a_skipped_gate_is_a_failure_not_a_pass(self):
        self.assertEqual(self.state([gate("skipping")]), "fail")

    def test_a_passing_gate_cannot_hide_a_failed_job_of_the_same_workflow(self):
        checks = [gate("pass"), check("rust / matrix.crate", "fail"), check("js", "skipping")]
        self.assertEqual(self.state(checks), "fail")

    def test_a_job_of_the_gate_workflow_still_running_keeps_the_wait_going(self):
        self.assertEqual(self.state([gate("pass"), check("rust / matrix.crate", "pending")]), "pending")

    def test_skipped_jobs_of_the_gate_workflow_are_fine(self):
        self.assertEqual(self.state([gate("pass"), check("js", "skipping"), check("rust", "skipping")]), "pass")

    def test_other_workflows_are_advisory_and_only_unfinished_or_failing_ones_are_listed(self):
        checks = [
            gate("pass"),
            check("Analyze (rust)", "pending", workflow="CodeQL SAST"),
            check("Analyze (python)", "fail", workflow="CodeQL SAST"),
            check("Analyze (c-cpp)", "pass", workflow="CodeQL SAST"),
            check("CodeRabbit", "pass", workflow=""),
        ]
        state, advisory = ci_wait.evaluate(checks, "CI gate")
        self.assertEqual(state, "pass")
        self.assertEqual(advisory, {"Analyze (rust)": "pending", "Analyze (python)": "fail"})

    def test_the_latest_run_of_a_check_wins(self):
        old_fail = gate("fail", started="2026-10-01T10:00:00Z")
        new_pass = gate("pass", started="2026-10-01T10:05:00Z")
        self.assertEqual(self.state([old_fail, new_pass]), "pass")
        self.assertEqual(self.state([new_pass, old_fail]), "pass")

    def test_a_queued_rerun_without_a_start_time_counts_as_newer_than_an_old_result(self):
        old = gate("fail", started="2026-10-01T10:00:00Z")
        for queued in (gate("pending", started=""), gate("pending", started="0001-01-01T00:00:00Z")):
            self.assertEqual(self.state([old, queued]), "pending")
            self.assertEqual(self.state([queued, old]), "pending")

    def test_equal_start_times_resolve_to_the_worse_result(self):
        tie_a, tie_b = gate("pass", started=""), gate("fail", started="")
        self.assertEqual(self.state([tie_a, tie_b]), "fail")
        self.assertEqual(self.state([tie_b, tie_a]), "fail")


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


def snap(checks, head="abc", error=""):
    return ci_wait.Snapshot(checks, head, error)


class WaitTest(unittest.TestCase):
    def wait(self, responses, expected="", **kwargs):
        clock, lines = Clock(), []
        queue = list(responses)

        def fetch():
            return queue.pop(0) if len(queue) > 1 else queue[0]

        code = ci_wait.wait(
            fetch, gate="CI gate", timeout=kwargs.get("timeout", 300), interval=10,
            grace=kwargs.get("grace", 60), sleep=clock.sleep, clock=clock, out=lines.append,
            expected_sha=expected,
        )
        return code, lines

    def test_waits_through_pending_then_passes_and_lists_advisory_checks(self):
        code, lines = self.wait([
            snap([gate("pending")]),
            snap([gate("pass"), check("Analyze (rust)", "pending", workflow="CodeQL SAST")]),
        ])
        self.assertEqual(code, 0)
        self.assertIn("Analyze (rust) pending", lines[-1])

    def test_exit_codes_distinguish_failure_and_cancellation(self):
        self.assertEqual(self.wait([snap([gate("fail")])])[0], 1)
        self.assertEqual(self.wait([snap([gate("cancel")])])[0], 4)

    def test_a_gate_that_never_finishes_times_out(self):
        code, lines = self.wait([snap([gate("pending")])], timeout=50)
        self.assertEqual(code, 2)
        self.assertIn("timed out", lines[-1])

    def test_a_gate_that_never_appears_gives_up_after_the_grace_period(self):
        code, lines = self.wait([snap([])], grace=30, timeout=600)
        self.assertEqual(code, 3)
        self.assertIn("never appeared", lines[-1])

    def test_gh_errors_are_reported_instead_of_never_appeared(self):
        code, lines = self.wait([snap([], error="HTTP 401: Bad credentials")], grace=30, timeout=600)
        self.assertEqual(code, 3)
        self.assertIn("gh failed: HTTP 401", lines[-1])

    def test_a_pass_belonging_to_an_older_head_is_not_trusted(self):
        code, lines = self.wait([snap([gate("pass")], head="old")] * 2, expected="new", grace=20)
        self.assertEqual(code, 3)
        self.assertIn("not the pushed", lines[-1])

    def test_the_new_head_is_picked_up_once_it_appears(self):
        code, _ = self.wait([snap([gate("pass")], head="old"), snap([gate("pass")], head="new")], expected="new")
        self.assertEqual(code, 0)

    def test_progress_is_printed_only_when_the_state_changes(self):
        code, lines = self.wait([snap([gate("pending")])] * 4 + [snap([gate("pass")])])
        self.assertEqual(len([l for l in lines if "pending" in l]), 1)


class ParseTest(unittest.TestCase):
    def test_gh_json_is_parsed_and_empty_or_invalid_output_is_no_checks(self):
        self.assertEqual(ci_wait.parse_checks(json.dumps([check("a", "pass")])), [check("a", "pass")])
        self.assertEqual(ci_wait.parse_checks(""), [])
        self.assertEqual(ci_wait.parse_checks("no checks reported"), [])
        self.assertEqual(ci_wait.parse_checks('{"not": "a list"}'), [])


class ArgumentTest(unittest.TestCase):
    def test_only_pr_numbers_and_pull_urls_are_accepted(self):
        self.assertEqual(ci_wait.pr_argument("154"), "154")
        self.assertEqual(
            ci_wait.pr_argument("https://github.com/o/r/pull/9"), "https://github.com/o/r/pull/9"
        )
        for bad in ("--repo=x/y", "-1", "main", "", "154 --web"):
            with self.assertRaises(ci_wait.argparse.ArgumentTypeError):
                ci_wait.pr_argument(bad)


if __name__ == "__main__":
    unittest.main()
