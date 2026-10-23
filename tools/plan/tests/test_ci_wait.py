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

    def test_a_gate_without_a_workflow_name_does_not_make_other_workflows_blocking(self):
        checks = [gate("pass", workflow=""), check("Analyze (rust)", "fail", workflow="")]
        self.assertEqual(self.state(checks), "pass")

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

    def test_an_unreadable_head_is_never_trusted_when_a_commit_is_expected(self):
        code, lines = self.wait([snap([gate("pass")], head="", error="HTTP 502")], expected="new", grace=20)
        self.assertEqual(code, 3)
        self.assertIn("could not read the PR head", lines[-1])

    def test_progress_is_printed_only_when_the_state_changes(self):
        code, lines = self.wait([snap([gate("pending")])] * 4 + [snap([gate("pass")])])
        self.assertEqual(len([l for l in lines if "pending" in l]), 1)


class FakeGh:
    def __init__(self, heads, checks_out="[]", checks_err="", checks_code=0):
        self.heads, self.checks = list(heads), (checks_out, checks_err, checks_code)

    def __call__(self, *args):
        if args[:2] == ("pr", "view"):
            out, err, code = (self.heads.pop(0) if len(self.heads) > 1 else self.heads[0]), "", 0
            if out is None:
                out, err, code = "", "HTTP 502", 1
        else:
            out, err, code = self.checks
        return ci_wait.subprocess.CompletedProcess(args, code, out, err)


class FetchSnapshotTest(unittest.TestCase):
    def fetch(self, gh):
        original = ci_wait._gh
        ci_wait._gh = gh
        self.addCleanup(setattr, ci_wait, "_gh", original)
        return ci_wait.fetch_snapshot("7")

    def test_checks_and_a_stable_head_are_returned_together(self):
        snapshot = self.fetch(FakeGh(["abc\n"], json.dumps([gate("pass")])))
        self.assertEqual((snapshot.head, snapshot.error, len(snapshot.checks)), ("abc", "", 1))

    def test_a_head_that_moves_while_reading_is_unknown(self):
        snapshot = self.fetch(FakeGh(["old", "new"], json.dumps([gate("pass")])))
        self.assertEqual(snapshot.head, "")
        self.assertTrue(snapshot.error)

    def test_a_failing_head_lookup_is_reported(self):
        snapshot = self.fetch(FakeGh([None], json.dumps([gate("pass")])))
        self.assertEqual(snapshot.head, "")
        self.assertIn("502", snapshot.error)

    def test_no_checks_yet_is_not_an_error_but_an_auth_failure_is(self):
        quiet = self.fetch(FakeGh(["abc"], "", "no checks reported on the 'x' branch", 1))
        self.assertEqual(quiet.error, "")
        broken = self.fetch(FakeGh(["abc"], "", "HTTP 401: Bad credentials", 1))
        self.assertIn("401", broken.error)


class MainTest(unittest.TestCase):
    def test_without_a_known_pushed_commit_it_refuses_instead_of_trusting_the_checks(self):
        original = ci_wait.local_head
        ci_wait.local_head = lambda: ""
        self.addCleanup(setattr, ci_wait, "local_head", original)
        self.assertEqual(ci_wait.main(["7"]), 3)


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
