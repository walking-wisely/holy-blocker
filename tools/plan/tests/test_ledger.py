import contextlib
import io
import tempfile
import unittest
from pathlib import Path

from tools.plan import ledger


class ExtractMarkersTest(unittest.TestCase):
    def test_finds_step_ids_in_order(self):
        text = "a <!-- step: p.one --> b\n<!--step:p.two-->"
        self.assertEqual(ledger.extract_markers(text), ["p.one", "p.two"])

    def test_ignores_unrelated_comments(self):
        self.assertEqual(ledger.extract_markers("<!-- TODO: x -->"), [])


class ValidateTest(unittest.TestCase):
    def _validate(self, steps, markers):
        return ledger.validate(steps, markers)

    def test_clean_manifest_has_no_problems(self):
        steps = [ledger.Step("p.a", "a", "done", "observed on emulator")]
        self.assertEqual(self._validate(steps, ["p.a"]), [])

    def test_pending_needs_no_evidence(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertEqual(self._validate(steps, ["p.a"]), [])

    def test_duplicate_manifest_id(self):
        steps = [ledger.Step("p.a", "a", "pending", ""), ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("duplicate manifest id" in p for p in self._validate(steps, ["p.a"])))

    def test_invalid_status(self):
        steps = [ledger.Step("p.a", "a", "finished", "")]
        self.assertTrue(any("invalid status" in p for p in self._validate(steps, ["p.a"])))

    def test_done_without_evidence(self):
        steps = [ledger.Step("p.a", "a", "done", "  ")]
        self.assertTrue(any("done without evidence" in p for p in self._validate(steps, ["p.a"])))

    def test_at_pr_status_is_valid(self):
        steps = [ledger.Step("p.a", "a", "at-pr", "PR #12")]
        self.assertEqual(self._validate(steps, ["p.a"]), [])

    def test_manifest_entry_without_marker(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("no <!-- step" in p for p in self._validate(steps, [])))

    def test_marker_without_manifest_entry(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("no manifest entry" in p for p in self._validate(steps, ["p.a", "p.ghost"])))

    def test_duplicate_marker(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("duplicate marker" in p for p in self._validate(steps, ["p.a", "p.a"])))


class RenderTest(unittest.TestCase):
    def test_renders_status_table(self):
        steps = [ledger.Step("p.a", "a", "done", "commit abc")]
        rendered = ledger.render(steps)
        self.assertIn("| Step | Status | Evidence |", rendered)
        self.assertIn("| `p.a` | done | commit abc |", rendered)


class NextPlanTest(unittest.TestCase):
    def test_all_done_is_distinct_from_blocked(self):
        step, reason = ledger.next_plan([ledger.Step("p.a", "a", "done", "obs")])
        self.assertIsNone(step)
        self.assertEqual(reason, "all steps are done")

    def test_blocked_reports_the_unmet_dependency(self):
        steps = [ledger.Step("p.a", "a", "pending", "", depends_on=("p.ghost",))]
        step, reason = ledger.next_plan(steps)
        self.assertIsNone(step)
        self.assertIn("blocked", reason)
        self.assertIn("p.ghost", reason)

    def test_actionable_step_has_empty_reason(self):
        step, reason = ledger.next_plan([ledger.Step("p.a", "a", "pending", "")])
        self.assertEqual(step.id, "p.a")
        self.assertEqual(reason, "")

    def test_at_pr_step_is_not_actionable(self):
        step, reason = ledger.next_plan([ledger.Step("p.a", "a", "at-pr", "PR #12")])
        self.assertIsNone(step)
        self.assertIn("awaiting merge", reason)

    def test_at_pr_is_not_a_satisfied_dependency(self):
        steps = [
            ledger.Step("p.a", "a", "at-pr", "PR #12"),
            ledger.Step("p.b", "b", "pending", "", depends_on=("p.a",)),
        ]
        step, reason = ledger.next_plan(steps)
        self.assertIsNone(step)
        self.assertIn("waits on", reason)
        self.assertIn("p.a", reason)

    def test_pending_is_offered_with_an_at_pr_sibling(self):
        steps = [
            ledger.Step("p.a", "a", "at-pr", "PR #12"),
            ledger.Step("p.b", "b", "pending", ""),
        ]
        step, _reason = ledger.next_plan(steps)
        self.assertEqual(step.id, "p.b")


class NextStepTest(unittest.TestCase):
    def test_returns_first_pending_with_satisfied_dependencies(self):
        steps = [
            ledger.Step("p.a", "a", "done", "obs"),
            ledger.Step("p.b", "b", "pending", ""),
            ledger.Step("p.c", "c", "pending", "", depends_on=("p.b",)),
        ]
        self.assertEqual(ledger.next_step(steps).id, "p.b")

    def test_skips_pending_whose_dependency_is_not_done(self):
        steps = [
            ledger.Step("p.a", "a", "pending", ""),
            ledger.Step("p.b", "b", "pending", "", depends_on=("p.a",)),
        ]
        self.assertEqual(ledger.next_step(steps).id, "p.a")

    def test_returns_none_when_all_done(self):
        steps = [ledger.Step("p.a", "a", "done", "obs")]
        self.assertIsNone(ledger.next_step(steps))

    def test_returns_none_when_only_blocked_pending_remain(self):
        steps = [
            ledger.Step("p.b", "b", "pending", "", depends_on=("p.c",)),
            ledger.Step("p.c", "c", "pending", "", depends_on=("p.b",)),
        ]
        self.assertIsNone(ledger.next_step(steps))


class AcceptanceTest(unittest.TestCase):
    def test_default_acceptance_is_observation(self):
        step = ledger.Step("p.a", "a", "pending", "")
        self.assertEqual(step.acceptance, "observation")

    def test_code_acceptance_is_valid(self):
        steps = [ledger.Step("p.a", "a", "pending", "", acceptance="code")]
        self.assertEqual(ledger.validate(steps, ["p.a"]), [])

    def test_product_acceptance_is_valid(self):
        steps = [ledger.Step("p.a", "a", "pending", "", acceptance="product")]
        self.assertEqual(ledger.validate(steps, ["p.a"]), [])

    def test_invalid_acceptance(self):
        steps = [ledger.Step("p.a", "a", "pending", "", acceptance="maybe")]
        problems = ledger.validate(steps, ["p.a"])
        self.assertTrue(any("invalid acceptance" in p for p in problems))


class DependencyValidationTest(unittest.TestCase):
    def test_unknown_dependency(self):
        steps = [ledger.Step("p.a", "a", "pending", "", depends_on=("p.ghost",))]
        problems = ledger.validate(steps, ["p.a"])
        self.assertTrue(any("unknown dependency" in p for p in problems))

    def test_self_dependency(self):
        steps = [ledger.Step("p.a", "a", "pending", "", depends_on=("p.a",))]
        problems = ledger.validate(steps, ["p.a"])
        self.assertTrue(any("depends on itself" in p for p in problems))

    def test_done_with_unmet_dependency(self):
        steps = [
            ledger.Step("p.a", "a", "pending", ""),
            ledger.Step("p.b", "b", "done", "obs", depends_on=("p.a",)),
        ]
        problems = ledger.validate(steps, ["p.a", "p.b"])
        self.assertTrue(any("dependency 'p.a' is not" in p for p in problems))

    def test_done_with_at_pr_dependency(self):
        steps = [
            ledger.Step("p.a", "a", "at-pr", "PR #12"),
            ledger.Step("p.b", "b", "done", "obs", depends_on=("p.a",)),
        ]
        problems = ledger.validate(steps, ["p.a", "p.b"])
        self.assertTrue(any("dependency 'p.a' is not" in p for p in problems))

    def test_cycle_is_reported(self):
        steps = [
            ledger.Step("p.a", "a", "pending", "", depends_on=("p.b",)),
            ledger.Step("p.b", "b", "pending", "", depends_on=("p.a",)),
        ]
        problems = ledger.validate(steps, ["p.a", "p.b"])
        self.assertTrue(any("dependency cycle" in p for p in problems))


class WriteTodoTest(unittest.TestCase):
    def test_writes_rendered_table_to_todo_md(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = Path(tmp)
            steps = [ledger.Step("p.a", "a", "done", "obs")]
            path = ledger.write_todo(package, steps)
            self.assertEqual(path, package / "TODO.md")
            content = path.read_text(encoding="utf-8")
            self.assertIn("p.a", content)
            self.assertIn("Generated by", content)

    def test_overwrites_existing_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = Path(tmp)
            (package / "TODO.md").write_text("stale", encoding="utf-8")
            ledger.write_todo(package, [ledger.Step("p.a", "a", "pending", "")])
            self.assertNotIn("stale", (package / "TODO.md").read_text(encoding="utf-8"))

    def test_render_write_flag_writes_file_and_suppresses_stdout(self):
        with tempfile.TemporaryDirectory() as tmp:
            package = Path(tmp)
            (package / "steps.toml").write_text(
                '[[step]]\nid = "p.a"\ntitle = "a"\nstatus = "pending"\n',
                encoding="utf-8",
            )
            (package / "plan.md").write_text("<!-- step: p.a -->", encoding="utf-8")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = ledger.main(["render", "--write", str(package)])
            self.assertEqual(code, 0)
            self.assertTrue((package / "TODO.md").exists())
            self.assertEqual(stdout.getvalue(), "")


class MissingPathTest(unittest.TestCase):
    def test_non_directory_path_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            good = Path(tmp) / "good"
            good.mkdir()
            (good / "steps.toml").write_text("", encoding="utf-8")
            selected, _skipped, missing = ledger._packages([str(good), str(Path(tmp) / "nope")])
            self.assertEqual(selected, [good])
            self.assertEqual(missing, [Path(tmp) / "nope"])

    def test_main_fails_on_unexpanded_glob(self):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = ledger.main(["validate", "docs/components/*/"])
        self.assertEqual(code, 1)
        self.assertIn("no such directory", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()