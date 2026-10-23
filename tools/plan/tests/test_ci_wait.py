import json
import unittest

from tools.plan import ci_wait


def check(name, bucket, started="2026-10-01T10:00:00Z"):
    return {"name": name, "bucket": bucket, "startedAt": started}


class EvaluateTest(unittest.TestCase):
    def test_gate_states_map_from_buckets(self):
        for bucket, state in (("pass", "pass"), ("fail", "fail"), ("cancel", "fail"), ("pending", "pending")):
            self.assertEqual(ci_wait.evaluate([check("CI gate", bucket)], "CI gate")[0], state, bucket)

    def test_a_missing_gate_is_pending(self):
        self.assertEqual(ci_wait.evaluate([check("other", "pass")], "CI gate")[0], "pending")
        self.assertEqual(ci_wait.evaluate([], "CI gate")[0], "pending")

    def test_a_skipped_gate_is_a_failure_not_a_pass(self):
        self.assertEqual(ci_wait.evaluate([check("CI gate", "skipping")], "CI gate")[0], "fail")

    def test_the_latest_run_of_the_gate_wins(self):
        checks = [
            check("CI gate", "fail", "2026-10-01T10:00:00Z"),
            check("CI gate", "pass", "2026-10-01T10:05:00Z"),
        ]
        self.assertEqual(ci_wait.evaluate(checks, "CI gate")[0], "pass")
        self.assertEqual(ci_wait.evaluate(checks[::-1], "CI gate")[0], "pass")

    def test_other_checks_are_advisory_and_only_unfinished_or_failing_ones_are_listed(self):
        checks = [
            check("CI gate", "pass"),
            check("CodeQL", "pending"),
            check("Analyze (python)", "fail"),
            check("lint", "pass"),
            check("rust", "skipping"),
        ]
        state, advisory = ci_wait.evaluate(checks, "CI gate")
        self.assertEqual(state, "pass")
        self.assertEqual(advisory, {"CodeQL": "pending", "Analyze (python)": "fail"})


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class WaitTest(unittest.TestCase):
    def wait(self, responses, **kwargs):
        clock, lines = Clock(), []
        queue = list(responses)

        def fetch():
            return queue.pop(0) if len(queue) > 1 else queue[0]

        code = ci_wait.wait(
            fetch, gate="CI gate", timeout=kwargs.get("timeout", 300), interval=10,
            grace=kwargs.get("grace", 60), sleep=clock.sleep, clock=clock, out=lines.append,
        )
        return code, lines

    def test_waits_through_pending_then_passes(self):
        code, lines = self.wait([[check("CI gate", "pending")], [check("CI gate", "pass"), check("CodeQL", "pending")]])
        self.assertEqual(code, 0)
        self.assertIn("pass", lines[-1])
        self.assertIn("CodeQL pending", lines[-1])

    def test_a_failing_gate_exits_one_immediately(self):
        code, lines = self.wait([[check("CI gate", "fail")]])
        self.assertEqual(code, 1)

    def test_a_gate_that_never_finishes_times_out(self):
        code, lines = self.wait([[check("CI gate", "pending")]], timeout=50)
        self.assertEqual(code, 2)
        self.assertIn("timed out", lines[-1])

    def test_a_gate_that_never_appears_gives_up_after_the_grace_period(self):
        code, lines = self.wait([[]], grace=30, timeout=600)
        self.assertEqual(code, 2)
        self.assertIn("never appeared", lines[-1])

    def test_progress_is_printed_only_when_the_state_changes(self):
        code, lines = self.wait(
            [[check("CI gate", "pending")]] * 4 + [[check("CI gate", "pass")]],
        )
        self.assertEqual(len([l for l in lines if "pending" in l]), 1)


class ParseTest(unittest.TestCase):
    def test_gh_json_is_parsed_and_empty_or_invalid_output_is_no_checks(self):
        self.assertEqual(ci_wait.parse_checks(json.dumps([check("a", "pass")])), [check("a", "pass")])
        self.assertEqual(ci_wait.parse_checks(""), [])
        self.assertEqual(ci_wait.parse_checks("no checks reported"), [])
        self.assertEqual(ci_wait.parse_checks('{"not": "a list"}'), [])


if __name__ == "__main__":
    unittest.main()
