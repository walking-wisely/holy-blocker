import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path
from textwrap import dedent

from tools.plan import ledger, todos


def _steps_toml(steps: list[dict]) -> str:
    """Render a list of step dicts as TOML."""
    lines = []
    for step in steps:
        lines.append("[[step]]")
        for k, v in step.items():
            if isinstance(v, bool):
                lines.append(f'{k} = {"true" if v else "false"}')
            elif isinstance(v, (list, tuple)):
                lines.append(f'{k} = {list(v)!r}'.replace("'", '"'))
            elif isinstance(v, int):
                lines.append(f"{k} = {v}")
            else:
                lines.append(f'{k} = "{v}"')
        lines.append("")
    return "\n".join(lines)


_STEPS_TOML = _steps_toml([
    {"id": "p.one", "title": "first", "status": "done", "evidence": "pr #1"},
    {"id": "p.two", "title": "second", "status": "pending"},
    {"id": "p.three", "title": "third", "status": "pending"},
])


def _write_package(root: Path, rel: str, steps: list[dict]) -> Path:
    package = root / rel
    (package / "steps").mkdir(parents=True, exist_ok=True)
    for step in steps:
        body = "\n".join(f'{k} = "{v}"' for k, v in step.items()) + "\n"
        (package / "steps" / f"{step['id']}.toml").write_text(body)
    return package


class DiscoverManifestsTest(unittest.TestCase):
    def test_finds_engineering_and_components(self):
        tmp_path = Path(tempfile.mkdtemp())
        _write_package(tmp_path, "docs/engineering", [])
        _write_package(tmp_path, "docs/components/x", [])
        found = todos.discover_manifests(tmp_path)
        self.assertEqual(sorted(label for _, label in found), ["engineering", "x"])

    def test_skips_a_component_with_no_manifest(self):
        tmp_path = Path(tempfile.mkdtemp())
        (tmp_path / "docs/components/y").mkdir(parents=True)
        self.assertEqual(todos.discover_manifests(tmp_path), [])

    def test_finds_a_legacy_manifest_in_another_worktree(self):
        tmp_path = Path(tempfile.mkdtemp())
        legacy = tmp_path / "docs/components/old"
        legacy.mkdir(parents=True)
        (legacy / "steps.toml").write_text(_STEPS_TOML)
        found = todos.discover_manifests(tmp_path)
        self.assertEqual([label for _, label in found], ["old"])
        self.assertEqual(len(todos.load_steps(found[0][0], "old")), 3)


class LoadStepsTest(unittest.TestCase):
    def test_parses_step_files(self):
        package = _write_package(Path(tempfile.mkdtemp()), "p", [
            {"id": "p.one", "title": "first", "status": "done", "evidence": "pr #1"},
            {"id": "p.two", "title": "second", "status": "pending"},
        ])
        steps = {s.id: s for s in todos.load_steps(package, "p")}
        self.assertEqual(steps["p.one"].status, "done")
        self.assertEqual(steps["p.two"].status, "pending")
        self.assertEqual(steps["p.one"].kind, "feature")

    def test_reads_kind_from_step_file(self):
        package = _write_package(Path(tempfile.mkdtemp()), "p", [
            {"id": "p.bug", "title": "a bug", "status": "pending", "kind": "bug"},
        ])
        self.assertEqual(todos.load_steps(package, "p")[0].kind, "bug")

    def test_returns_empty_on_missing_directory(self):
        self.assertEqual(todos.load_steps(Path("/nonexistent/pkg"), "x"), [])

    def test_returns_empty_on_bad_toml(self):
        package = Path(tempfile.mkdtemp())
        (package / "steps").mkdir()
        (package / "steps" / "x.a.toml").write_text("not toml {{{")
        self.assertEqual(todos.load_steps(package, "x"), [])


class LoadStepsToleranceTest(unittest.TestCase):
    def test_non_iterable_depends_on_does_not_abort_the_scan(self):
        package = Path(tempfile.mkdtemp())
        (package / "steps").mkdir()
        (package / "steps" / "p.a.toml").write_text('id = "p.a"\nstatus = "pending"\ndepends_on = 3\n')
        self.assertEqual(todos.load_steps(package, "p"), [])


class LoadStepsWarningTest(unittest.TestCase):
    def test_a_broken_package_is_reported_on_stderr(self):
        package = Path(tempfile.mkdtemp())
        (package / "steps").mkdir()
        (package / "steps" / "x.a.toml").write_text('id = "x.a"\n')
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            self.assertEqual(todos.load_steps(package, "x"), [])
        self.assertIn("x", stderr.getvalue())
        self.assertIn("unreadable", stderr.getvalue())


class FormatBlockerFlagTest(unittest.TestCase):
    def test_dirty_gets_warning(self):
        self.assertIn("⚠", todos.format_blocker_flag("dirty"))

    def test_local_only_gets_warning(self):
        self.assertIn("⚠", todos.format_blocker_flag("local-only"))

    def test_open_pr_gets_warning(self):
        self.assertIn("⚠", todos.format_blocker_flag("open"))

    def test_unknown_gets_question(self):
        self.assertIn("?", todos.format_blocker_flag("unknown"))

    def test_merged_has_no_flag(self):
        self.assertEqual(todos.format_blocker_flag("merged"), "")

    def test_abandoned_has_no_flag(self):
        self.assertEqual(todos.format_blocker_flag("abandoned"), "")


class ShowTest(unittest.TestCase):
    def _todo(self, step_id="p.x", status="pending", verdict="merged", branch="feat/x", path="/w", kind="feature", milestone=""):
        return todos.Todo(
            step_id=step_id, title="a step", status=status,
            acceptance="code", verify="", evidence="",
            package="p", worktree_path=path, branch=branch,
            worktree_verdict=verdict, worktree_reason="test", kind=kind, milestone=milestone,
        )

    def _capture(self, todos_list, **kwargs):
        buf = io.StringIO()
        out = sys.stdout
        try:
            sys.stdout = buf
            todos.show(todos_list, **kwargs)
        finally:
            sys.stdout = out
        return buf.getvalue()

    def test_pending_only_by_default(self):
        ts = [self._todo("p.a", "done"), self._todo("p.b", "pending")]
        out = self._capture(ts, show_all=False, blockers_only=False)
        self.assertIn("p.b", out)
        self.assertNotIn("p.a", out)

    def test_all_includes_done(self):
        ts = [self._todo("p.a", "done"), self._todo("p.b", "pending")]
        out = self._capture(ts, show_all=True, blockers_only=False)
        self.assertIn("p.a", out)
        self.assertIn("p.b", out)

    def test_milestone_steps_are_listed_first_and_tagged(self):
        ts = [self._todo("p.a"), self._todo("p.z", milestone="mvp")]
        out = self._capture(ts, show_all=False, blockers_only=False)
        self.assertLess(out.index("p.z"), out.index("p.a"))
        self.assertIn("p.z [mvp]", out)
        self.assertNotIn("p.a [mvp]", out)

    def test_milestone_filter_drops_other_steps(self):
        ts = [self._todo("p.a"), self._todo("p.z", milestone="mvp")]
        out = self._capture(ts, show_all=False, blockers_only=False, milestone="mvp")
        self.assertIn("p.z", out)
        self.assertNotIn("p.a", out)

    def test_blockers_only_filters_clean(self):
        ts = [
            self._todo("p.a", "pending", verdict="dirty", path="/w/dirty"),
            self._todo("p.b", "pending", verdict="merged", path="/w/clean"),
        ]
        out = self._capture(ts, show_all=False, blockers_only=True)
        self.assertIn("dirty", out)
        self.assertNotIn("clean", out)

    def test_blocker_none_shows_message(self):
        ts = [self._todo("p.a", "pending", verdict="merged", path="/w/clean")]
        out = self._capture(ts, show_all=False, blockers_only=True)
        self.assertIn("no worktrees with blockers", out)

    def test_bug_steps_carry_a_distinct_marker(self):
        ts = [self._todo("p.bug", kind="bug"), self._todo("p.feat")]
        out = self._capture(ts, show_all=False, blockers_only=False)
        lines = [l for l in out.splitlines() if "p.bug" in l or "p.feat" in l]
        self.assertTrue(any(todos.BUG_MARKER in l and "p.bug" in l for l in lines))
        self.assertFalse(any(todos.BUG_MARKER in l and "p.feat" in l for l in lines))