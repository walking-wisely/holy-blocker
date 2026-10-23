import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.plan.e2e import manifest, runner

DIGEST = "a" * 64


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
        cwd=root, check=True, capture_output=True, text=True,
    ).stdout.strip()


def scenario(**overrides) -> manifest.Scenario:
    fields = dict(
        path=Path("demos/x/scenario.toml"), id="x-smoke", platform="macos",
        covers=("demos/x", "packages/a"), proves=("row",), does_not_prove="n/a",
        preflight="true", run="true", interactive=None,
    )
    fields.update(overrides)
    return manifest.Scenario(**fields)


class RepoCase(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        git(self.root, "init", "-q", "-b", "master")
        self.write("demos/x/run.sh", "x")
        self.write("packages/a/lib.rs", "a")
        self.write("other/file.txt", "o")
        self.commit("init")

    def write(self, rel: str, text: str):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self, message: str):
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", message)


class SelectTest(unittest.TestCase):
    def test_selects_by_prefix_and_counts_uncovered(self):
        a = scenario()
        b = scenario(id="y", covers=("apps/y",))
        picked, uncovered = runner.select([a, b], ["packages/a/lib.rs", "other/f", "docs/x.md"])
        self.assertEqual([s.id for s in picked], ["x-smoke"])
        self.assertEqual(uncovered, 2)

    def test_prefix_is_path_boundary(self):
        picked, uncovered = runner.select([scenario()], ["packages/ab/lib.rs"])
        self.assertEqual(picked, [])
        self.assertEqual(uncovered, 1)

    def test_nothing_selected_is_reported_unverified(self):
        self.assertIn("unverified", runner.coverage_line([], 3).lower())
        self.assertIn("3 changed paths covered by no scenario", runner.coverage_line([], 3))


class DecideTest(unittest.TestCase):
    OK = "ASSERT blocked 2\nASSERT permitted 1\nARTIFACT sha256 " + DIGEST + "\nHOST android-36 arm64 AVD\nSMOKE PASS\n"

    def test_pass_needs_both_directions(self):
        self.assertEqual(runner.decide(0, self.OK).result, "pass")
        self.assertEqual(runner.decide(0, "ASSERT blocked 2\nSMOKE PASS").result, "fail")
        self.assertEqual(runner.decide(0, "ASSERT blocked 0\nASSERT permitted 3").result, "fail")
        self.assertEqual(runner.decide(0, "SMOKE PASS").result, "fail")

    def test_nonzero_exit_fails_whatever_it_prints(self):
        self.assertEqual(runner.decide(1, self.OK).result, "fail")

    def test_run_exit_two_is_a_failure(self):
        self.assertEqual(runner.decide(2, self.OK).result, "fail")

    def test_parses_artifact_and_host(self):
        verdict = runner.decide(0, self.OK)
        self.assertEqual(verdict.artifact, DIGEST)
        self.assertEqual(verdict.host_class, "android-36 arm64 AVD")

    def test_final_line_only(self):
        self.assertEqual(runner.final_line("a\nB\n\n"), "B")
        self.assertEqual(runner.final_line(""), "")
        self.assertEqual(runner.final_line("x" * 500), runner.WITHHELD)

    def test_free_text_and_escapes_are_withheld(self):
        for line in ("token sk-SECRET", "PASS \x1b[2J", "FAIL: /Users/me/x", "FAIL: Token=abc"):
            self.assertEqual(runner.final_line(line), runner.WITHHELD, line)
        self.assertEqual(runner.final_line("SMOKE FAIL: no grant"), "SMOKE FAIL: no grant")


class DigestTest(RepoCase):
    def test_stable_across_unrelated_commit_and_changes_with_covered(self):
        s = scenario()
        before = runner.covered_digest(self.root, s.covers)
        self.write("other/file.txt", "changed")
        self.commit("unrelated")
        self.assertEqual(runner.covered_digest(self.root, s.covers), before)
        self.write("packages/a/lib.rs", "changed")
        self.commit("covered")
        self.assertNotEqual(runner.covered_digest(self.root, s.covers), before)

    def test_dirty_detects_edit_and_untracked(self):
        s = scenario()
        self.assertFalse(runner.dirty(self.root, s.covers))
        self.write("packages/a/lib.rs", "edit")
        self.assertTrue(runner.dirty(self.root, s.covers))
        git(self.root, "checkout", "--", ".")
        self.write("demos/x/new.sh", "n")
        self.assertTrue(runner.dirty(self.root, s.covers))

    def test_dirty_ignores_uncovered_paths(self):
        self.write("other/file.txt", "edit")
        self.assertFalse(runner.dirty(self.root, scenario().covers))


class RecordTest(RepoCase):
    def verdict(self):
        return runner.decide(0, DecideTest.OK)

    def test_record_and_read_back(self):
        s = scenario()
        digest = runner.record(self.root, s, self.verdict(), "2026-10-24")
        note = runner.read_note(self.root, s.id, digest)
        self.assertEqual(note["result"], "pass")
        self.assertEqual(note["date"], "2026-10-24")
        self.assertEqual(note["host_class"], "android-36 arm64 AVD")
        self.assertEqual(note["artifact_sha256"], DIGEST)
        self.assertEqual(set(note), {"result", "date", "host_class", "artifact_sha256"})

    def test_note_survives_unrelated_commit(self):
        s = scenario()
        digest = runner.record(self.root, s, self.verdict(), "2026-10-24")
        self.write("other/file.txt", "changed")
        self.commit("unrelated")
        self.assertIsNotNone(runner.read_note(self.root, s.id, runner.covered_digest(self.root, s.covers)))
        self.assertEqual(digest, runner.covered_digest(self.root, s.covers))

    def test_refuses_dirty_tree(self):
        s = scenario()
        self.write("packages/a/lib.rs", "edit")
        with self.assertRaises(runner.RecordRefused):
            runner.record(self.root, s, self.verdict(), "2026-10-24")

    def test_refuses_missing_artifact_or_host(self):
        s = scenario()
        with self.assertRaises(runner.RecordRefused):
            runner.record(self.root, s, runner.decide(0, "ASSERT blocked 1\nASSERT permitted 1"), "2026-10-24")

    def test_refuses_failed_verdict(self):
        with self.assertRaises(runner.RecordRefused):
            runner.record(self.root, scenario(), runner.decide(1, DecideTest.OK), "2026-10-24")

    def test_no_note_reads_none(self):
        self.assertIsNone(runner.read_note(self.root, "x-smoke", "0" * 40))

    def test_hostile_host_class_is_dropped(self):
        out = DecideTest.OK.replace("android-36 arm64 AVD", "MacBook-Pro")
        self.assertIsNone(runner.decide(0, out).host_class)


class RunScenarioTest(RepoCase):
    def make(self, preflight: str, run: str) -> manifest.Scenario:
        return scenario(preflight=preflight, run=run)

    def test_preflight_exit_two_is_environment_not_ready(self):
        outcome = runner.run_scenario(self.root, self.make("sh -c 'exit 2'", "true"))
        self.assertEqual(outcome.status, "env")

    def test_preflight_other_failure_is_fail(self):
        outcome = runner.run_scenario(self.root, self.make("sh -c 'exit 1'", "true"))
        self.assertEqual(outcome.status, "fail")

    def test_run_is_not_started_when_environment_not_ready(self):
        marker = self.root / "ran"
        outcome = runner.run_scenario(self.root, self.make("sh -c 'exit 2'", f"touch {marker}"))
        self.assertEqual(outcome.status, "env")
        self.assertFalse(marker.exists())

    def test_script_pass_line_alone_does_not_pass(self):
        outcome = runner.run_scenario(self.root, self.make("true", "echo SCENARIO PASS"))
        self.assertEqual(outcome.status, "fail")

    def test_pass_with_assertions(self):
        run = "printf 'ASSERT blocked 1\\nASSERT permitted 1\\nARTIFACT sha256 %s\\nHOST h\\nDONE\\n' " + DIGEST
        outcome = runner.run_scenario(self.root, self.make("true", run))
        self.assertEqual(outcome.status, "pass")
        self.assertEqual(outcome.final_line, "DONE")

    def test_only_final_line_is_exposed(self):
        outcome = runner.run_scenario(self.root, self.make("true", "sh -c \"printf 'secret\\\\nLAST\\\\n'; exit 1\""))
        self.assertEqual(outcome.final_line, "LAST")
        self.assertNotIn("secret", repr(outcome))


class HardeningTest(RepoCase):
    def test_assume_unchanged_edit_is_dirty(self):
        git(self.root, "update-index", "--assume-unchanged", "packages/a/lib.rs")
        self.write("packages/a/lib.rs", "edit")
        self.assertTrue(runner.dirty(self.root, scenario().covers))

    def test_skip_worktree_is_dirty(self):
        git(self.root, "update-index", "--skip-worktree", "packages/a/lib.rs")
        self.assertTrue(runner.dirty(self.root, scenario().covers))

    def test_digest_moved_during_run_is_refused(self):
        with self.assertRaises(runner.RecordRefused):
            runner.record(self.root, scenario(), RecordTest.verdict(self), "2026-10-24", expected="0" * 40)

    def test_invalid_ref_id_is_refused(self):
        for bad in ("a..b", "x.lock", "a.", "a b"):
            with self.assertRaises(runner.RecordRefused, msg=bad):
                runner.record(self.root, scenario(id=bad), RecordTest.verdict(self), "2026-10-24")

    def test_missing_cover_is_refused_not_crashed(self):
        with self.assertRaises(runner.RecordRefused):
            runner.record(self.root, scenario(covers=("gone",)), RecordTest.verdict(self), "2026-10-24")

    def test_rename_out_of_covers_selects_scenario(self):
        git(self.root, "checkout", "-q", "-b", "feat")
        git(self.root, "mv", "packages/a/lib.rs", "other/moved.rs")
        self.commit("move")
        self.assertIn("packages/a/lib.rs", runner._changed(self.root, "master", "HEAD"))

    def test_broken_manifest_fails_the_run(self):
        self.write("demos/y/scenario.toml", "not = [valid")
        self.commit("broken")
        lines = []
        code = runner.main(["select", "--base", "HEAD", "--head", "HEAD"], root=self.root, out=lines.append)
        self.assertEqual(code, runner.EXIT_FAIL)


class MainTest(RepoCase):
    def test_unverified_when_no_scenario_selected(self):
        git(self.root, "checkout", "-q", "-b", "feat")
        self.write("other/file.txt", "changed")
        self.commit("change")
        lines = []
        code = runner.main(["select", "--base", "master", "--head", "HEAD"], root=self.root, out=lines.append)
        text = "\n".join(lines)
        self.assertEqual(code, runner.EXIT_UNVERIFIED)
        self.assertIn("unverified", text.lower())
        self.assertIn("1 changed paths covered by no scenario", text)


if __name__ == "__main__":
    unittest.main()
