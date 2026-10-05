import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.plan import inventory, state


def git(cwd, *args):
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


def make_repo(root: Path) -> Path:
    repo = root / "repo"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "master")
    git(repo, "config", "user.email", "t@example.com")
    git(repo, "config", "user.name", "t")
    (repo / ".gitignore").write_text("node_modules/\n.env\n")
    (repo / "a").write_text("1")
    git(repo, "add", ".")
    git(repo, "commit", "-qm", "init")
    return repo


def add_worktree(repo: Path, name: str, *extra: str) -> Path:
    path = repo.parent / name
    git(repo, "worktree", "add", "-q", str(path), *extra)
    return path


def no_pr(branch, cwd):
    return None


class VerdictTest(unittest.TestCase):
    def verdict(self, **kwargs):
        base = dict(prunable=False, uncommitted=(), ignored=(), orphaned=False)
        base.update(kwargs)
        return inventory.verdict_for(**base)

    def test_clean(self):
        self.assertEqual(self.verdict(), "clean")

    def test_prunable(self):
        self.assertEqual(self.verdict(prunable=True), "prunable")

    def test_uncommitted_blocks(self):
        self.assertEqual(self.verdict(uncommitted=("?? x",)), "has-uncommitted")

    def test_ignored_state_blocks(self):
        self.assertEqual(self.verdict(ignored=(".env",)), "has-ignored-state")

    def test_orphaned_head_outranks_everything(self):
        got = self.verdict(orphaned=True, uncommitted=("?? x",), ignored=(".env",))
        self.assertEqual(got, "orphaned-head")

    def test_uncommitted_outranks_ignored(self):
        self.assertEqual(self.verdict(uncommitted=("M a",), ignored=(".env",)), "has-uncommitted")


class NonDisposableIgnoredTest(unittest.TestCase):
    def test_build_output_is_dropped(self):
        got = state.non_disposable_ignored(["node_modules/", "target/", "apps/x/dist/"])
        self.assertEqual(got, [])

    def test_env_file_is_kept(self):
        got = state.non_disposable_ignored(["node_modules/", ".env"])
        self.assertEqual(got, [".env"])


def inv(branch, verdict, path=None, at_risk=False):
    return inventory.Inventory(
        path=path or f"/wt/{branch}",
        branch=branch,
        verdict=verdict,
        uncommitted=("M a",) if verdict == "has-uncommitted" else (),
        ignored=(),
        commits=(),
        local_only=0,
        branch_at_risk=at_risk,
        pr="none",
    )


class SelectTest(unittest.TestCase):
    def setUp(self):
        self.rows = [
            inv("feat/a", "clean"),
            inv("feat/b", "has-uncommitted"),
            inv("master", "clean", path="/repo"),
        ]
        self.protected = {os.path.realpath("/repo")}

    def test_names_pick_only_what_was_named(self):
        chosen, refused = inventory.select(self.rows, ["feat/a"], self.protected, discard=False)
        self.assertEqual([r.branch for r in chosen], ["feat/a"])
        self.assertEqual(refused, [])

    def test_blocked_is_refused_without_discard(self):
        chosen, refused = inventory.select(self.rows, ["feat/b"], self.protected, discard=False)
        self.assertEqual(chosen, [])
        self.assertEqual(refused[0][0], "feat/b")

    def test_discard_allows_blocked(self):
        chosen, _ = inventory.select(self.rows, ["feat/b"], self.protected, discard=True)
        self.assertEqual([r.branch for r in chosen], ["feat/b"])

    def test_protected_is_refused_even_with_discard(self):
        chosen, refused = inventory.select(self.rows, ["master"], self.protected, discard=True)
        self.assertEqual(chosen, [])
        self.assertIn("protected", refused[0][1])

    def test_unknown_name_is_refused(self):
        chosen, refused = inventory.select(self.rows, ["feat/zzz"], self.protected, discard=False)
        self.assertEqual(chosen, [])
        self.assertIn("no such worktree", refused[0][1])

    def test_path_matches_a_detached_worktree(self):
        rows = [inv(None, "clean", path="/wt/detached-1")]
        chosen, _ = inventory.select(rows, ["/wt/detached-1"], set(), discard=False)
        self.assertEqual(len(chosen), 1)


class SummaryTest(unittest.TestCase):
    def test_counts_exclude_protected_and_split_removable_from_blocked(self):
        rows = [inv("a", "clean"), inv("b", "clean"), inv("c", "has-uncommitted"), inv("master", "clean", path="/repo")]
        line = inventory.summary_line(rows, {os.path.realpath("/repo")}, remote_branches=7)
        self.assertIn("3 worktrees", line)
        self.assertIn("2 removable", line)
        self.assertIn("1 blocked", line)
        self.assertIn("7 remote branches", line)

    def test_nothing_to_report_is_empty(self):
        self.assertEqual(inventory.summary_line([], set(), remote_branches=0), "")


class FormatTest(unittest.TestCase):
    def test_lists_files_and_commits_and_risk(self):
        row = inventory.Inventory(
            path="/wt/a", branch="feat/a", verdict="has-uncommitted",
            uncommitted=("?? new.txt", "M a"), ignored=(".env",),
            commits=("abc123 add thing",), local_only=1, branch_at_risk=True, pr="none",
        )
        text = "\n".join(inventory.format_inventory(row))
        for needle in ("feat/a", "has-uncommitted", "?? new.txt", ".env", "abc123 add thing", "branch-at-risk"):
            self.assertIn(needle, text)

    def test_long_lists_are_capped(self):
        row = inv("a", "has-uncommitted")
        row = inventory.Inventory(**{**row.__dict__, "uncommitted": tuple(f"?? f{i}" for i in range(50))})
        text = "\n".join(inventory.format_inventory(row))
        self.assertIn("more", text)
        self.assertLess(len(text.splitlines()), 30)


class BuildAgainstRealGitTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = make_repo(Path(self.tmp.name))

    def build(self, path):
        trees = {os.path.realpath(t.path): t for t in state.list_worktrees(str(self.repo))}
        return inventory.build(trees[os.path.realpath(path)], "master", str(self.repo), pr_lookup=no_pr)

    def test_fresh_worktree_is_clean(self):
        wt = add_worktree(self.repo, "wt-clean", "-b", "feat/clean")
        self.assertEqual(self.build(wt).verdict, "clean")

    def test_untracked_file_blocks(self):
        wt = add_worktree(self.repo, "wt-untracked", "-b", "feat/u")
        (wt / "scratch.txt").write_text("x")
        got = self.build(wt)
        self.assertEqual(got.verdict, "has-uncommitted")
        self.assertTrue(any("scratch.txt" in line for line in got.uncommitted))

    def test_modified_tracked_file_blocks(self):
        wt = add_worktree(self.repo, "wt-mod", "-b", "feat/m")
        (wt / "a").write_text("changed")
        self.assertEqual(self.build(wt).verdict, "has-uncommitted")

    def test_ignored_env_blocks_but_node_modules_does_not(self):
        wt = add_worktree(self.repo, "wt-ign", "-b", "feat/i")
        (wt / "node_modules").mkdir()
        (wt / "node_modules" / "x").write_text("x")
        self.assertEqual(self.build(wt).verdict, "clean")
        (wt / ".env").write_text("SECRET=1")
        got = self.build(wt)
        self.assertEqual(got.verdict, "has-ignored-state")
        self.assertEqual(got.ignored, (".env",))

    def test_commits_ahead_are_listed(self):
        wt = add_worktree(self.repo, "wt-ahead", "-b", "feat/ahead")
        (wt / "b").write_text("2")
        git(wt, "add", "b")
        git(wt, "commit", "-qm", "add b")
        got = self.build(wt)
        self.assertEqual(got.verdict, "clean")
        self.assertEqual(len(got.commits), 1)
        self.assertIn("add b", got.commits[0])
        self.assertTrue(got.branch_at_risk)

    def test_detached_head_on_no_ref_is_orphaned(self):
        wt = add_worktree(self.repo, "wt-orphan", "--detach")
        (wt / "b").write_text("2")
        git(wt, "add", "b")
        git(wt, "commit", "-qm", "orphan work")
        self.assertEqual(self.build(wt).verdict, "orphaned-head")

    def test_detached_head_held_by_a_branch_is_clean(self):
        wt = add_worktree(self.repo, "wt-detached", "--detach")
        self.assertEqual(self.build(wt).verdict, "clean")


class RemoveTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = make_repo(Path(self.tmp.name))

    def test_remove_keeps_the_branch(self):
        wt = add_worktree(self.repo, "wt-keep", "-b", "feat/keep")
        row = inv("feat/keep", "clean", path=str(wt))
        inventory.remove(row, str(self.repo), discard=False)
        self.assertFalse(wt.exists())
        self.assertIn("feat/keep", git(self.repo, "branch", "--list", "feat/keep"))

    def test_remove_refuses_dirty_tree_without_discard(self):
        wt = add_worktree(self.repo, "wt-dirty", "-b", "feat/dirty")
        (wt / "x").write_text("x")
        row = inv("feat/dirty", "clean", path=str(wt))
        with self.assertRaises(subprocess.CalledProcessError):
            inventory.remove(row, str(self.repo), discard=False)
        self.assertTrue(wt.exists())

    def test_remove_discards_dirty_tree_when_asked(self):
        wt = add_worktree(self.repo, "wt-discard", "-b", "feat/discard")
        (wt / "x").write_text("x")
        row = inv("feat/discard", "has-uncommitted", path=str(wt))
        inventory.remove(row, str(self.repo), discard=True)
        self.assertFalse(wt.exists())


if __name__ == "__main__":
    unittest.main()
