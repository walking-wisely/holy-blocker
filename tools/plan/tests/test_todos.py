import io
import sys
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


class DiscoverManifestsTest(unittest.TestCase):
    def test_finds_engineering_and_components(self):
        tmp_path = Path("/tmp/todos-manifest-test")
        eng = tmp_path / "docs/engineering"
        comp = tmp_path / "docs/components/x"
        eng.mkdir(parents=True, exist_ok=True)
        comp.mkdir(parents=True, exist_ok=True)
        (eng / "steps.toml").write_text("")
        (comp / "steps.toml").write_text("")
        found = todos.discover_manifests(tmp_path)
        labels = [label for _, label in found]
        self.assertIn("engineering", labels)
        self.assertIn("x", labels)

    def test_skips_missing_steps_toml(self):
        tmp_path = Path("/tmp/todos-skip-test")
        (tmp_path / "docs/components/y").mkdir(parents=True, exist_ok=True)
        found = todos.discover_manifests(tmp_path)
        self.assertFalse(any(label == "y" for _, label in found))


class LoadStepsTest(unittest.TestCase):
    def test_parses_valid_toml(self):
        tmp_path = Path("/tmp/todos-load-test")
        p = tmp_path / "steps.toml"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(_STEPS_TOML)
        steps = todos.load_steps(p, "p")
        self.assertEqual(len(steps), 3)
        self.assertEqual(steps[0].id, "p.one")
        self.assertEqual(steps[0].status, "done")
        self.assertEqual(steps[1].status, "pending")
        self.assertEqual(steps[0].kind, "feature")

    def test_reads_kind_from_manifest(self):
        tmp_path = Path("/tmp/todos-kind-test")
        p = tmp_path / "steps.toml"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(_steps_toml([
            {"id": "p.bug", "title": "a bug", "status": "pending", "kind": "bug"},
        ]))
        steps = todos.load_steps(p, "p")
        self.assertEqual(steps[0].kind, "bug")

    def test_returns_empty_on_missing_file(self):
        self.assertEqual(todos.load_steps(Path("/nonexistent/steps.toml"), "x"), [])

    def test_returns_empty_on_bad_toml(self):
        tmp_path = Path("/tmp/todos-bad-test")
        p = tmp_path / "steps.toml"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text("not toml {{{")
        self.assertEqual(todos.load_steps(p, "x"), [])


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
    def _todo(self, step_id="p.x", status="pending", verdict="merged", branch="feat/x", path="/w", kind="feature"):
        return todos.Todo(
            step_id=step_id, title="a step", status=status,
            acceptance="code", verify="", evidence="",
            package="p", worktree_path=path, branch=branch,
            worktree_verdict=verdict, worktree_reason="test", kind=kind,
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