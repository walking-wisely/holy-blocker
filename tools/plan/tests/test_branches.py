import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.plan import branches

TIP = "a" * 40
OTHER = "b" * 40


def pr(state, head="feat/x", oid=TIP, base="master", number=1):
    return branches.PrRecord(number=number, state=state, head=head, head_oid=oid, base=base)


def git(cwd, *args):
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


class ParsePrsTest(unittest.TestCase):
    def test_parses_gh_json(self):
        payload = json.dumps(
            [{"number": 5, "state": "MERGED", "headRefName": "feat/x", "headRefOid": TIP, "baseRefName": "master"}]
        )
        self.assertEqual(branches.parse_prs(payload), [pr("MERGED", number=5)])


class ClassifyTest(unittest.TestCase):
    def classify(self, prs, in_base=False, tip=TIP):
        return branches.classify("feat/x", tip, prs, "master", in_base)[0]

    def test_merged_pr_with_matching_tip(self):
        self.assertEqual(self.classify([pr("MERGED")]), "merged")

    def test_merged_pr_but_tip_moved_on(self):
        self.assertEqual(self.classify([pr("MERGED", oid=OTHER)]), "merged-diverged")

    def test_ancestor_of_base_without_pr(self):
        self.assertEqual(self.classify([], in_base=True), "ancestor")

    def test_open_pr_wins_over_everything(self):
        self.assertEqual(self.classify([pr("MERGED"), pr("OPEN", number=2)], in_base=True), "open")

    def test_reused_name_old_merge_does_not_cover_new_tip(self):
        self.assertEqual(self.classify([pr("MERGED", oid=OTHER)], tip=TIP), "merged-diverged")

    def test_merge_into_another_base_is_not_a_merge_into_master(self):
        self.assertEqual(self.classify([pr("MERGED", base="feat/parent")]), "merged-other-base")

    def test_closed_only(self):
        self.assertEqual(self.classify([pr("CLOSED")]), "closed")

    def test_no_pr(self):
        self.assertEqual(self.classify([]), "none")

    def test_other_branches_prs_are_ignored(self):
        self.assertEqual(self.classify([pr("MERGED", head="feat/other")]), "none")


def row(name, verdict):
    return branches.RemoteBranch(name=name, tip=TIP, verdict=verdict, reason="", pr="")


class SelectTest(unittest.TestCase):
    def setUp(self):
        self.rows = [row("a", "merged"), row("b", "ancestor"), row("c", "closed"), row("d", "open"), row("e", "none")]

    def pick(self, names, discard=False):
        chosen, refused = branches.select(self.rows, names, discard)
        return [r.name for r in chosen], [name for name, _ in refused]

    def test_merged_and_ancestor_qualify(self):
        self.assertEqual(self.pick(["a", "b"]), (["a", "b"], []))

    def test_unmerged_needs_discard(self):
        self.assertEqual(self.pick(["c", "e"]), ([], ["c", "e"]))
        self.assertEqual(self.pick(["c", "e"], discard=True), (["c", "e"], []))

    def test_open_is_never_deletable(self):
        self.assertEqual(self.pick(["d"], discard=True), ([], ["d"]))

    def test_unknown_name_is_refused(self):
        self.assertEqual(self.pick(["zzz"]), ([], ["zzz"]))

    def test_default_branches_are_never_deletable(self):
        self.rows += [row("master", "ancestor"), row("main", "ancestor"), row("develop", "ancestor")]
        chosen, refused = branches.select(self.rows, ["master", "main", "develop"], True, protected={"develop"})
        self.assertEqual(chosen, [])
        self.assertEqual([name for name, _ in refused], ["master", "main", "develop"])


class DeleteAgainstRealGitTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        git(root, "init", "-q", "--bare", "-b", "master", "remote.git")
        self.work = root / "work"
        git(root, "clone", "-q", "remote.git", "work")
        git(self.work, "config", "user.email", "t@example.com")
        git(self.work, "config", "user.name", "t")
        (self.work / "f").write_text("1")
        git(self.work, "add", "f")
        git(self.work, "commit", "-qm", "init")
        git(self.work, "push", "-q", "origin", "HEAD:master")
        git(self.work, "push", "-q", "origin", "HEAD:refs/heads/feat/x")
        git(self.work, "fetch", "-q", "origin")

    def remote_has(self, name):
        return name in git(self.work, "ls-remote", "origin", f"refs/heads/{name}")

    def test_delete_removes_the_remote_branch(self):
        tip = git(self.work, "rev-parse", "origin/feat/x").strip()
        branches.delete(branches.RemoteBranch("feat/x", tip, "merged", "", ""), str(self.work))
        self.assertFalse(self.remote_has("feat/x"))

    def test_delete_refuses_when_the_tip_moved(self):
        stale = branches.RemoteBranch("feat/x", "0" * 40, "merged", "", "")
        with self.assertRaises(subprocess.CalledProcessError):
            branches.delete(stale, str(self.work))
        self.assertTrue(self.remote_has("feat/x"))

    def test_list_remote_skips_head_and_strips_prefix(self):
        names = {name for name, _ in branches.list_remote(str(self.work))}
        self.assertEqual(names, {"master", "feat/x"})

    def test_is_in_base(self):
        self.assertTrue(branches.is_in_base(str(self.work), "master", git(self.work, "rev-parse", "origin/feat/x").strip()))
        (self.work / "g").write_text("2")
        git(self.work, "add", "g")
        git(self.work, "commit", "-qm", "ahead")
        git(self.work, "push", "-q", "origin", "HEAD:refs/heads/feat/ahead")
        git(self.work, "fetch", "-q", "origin")
        ahead = git(self.work, "rev-parse", "origin/feat/ahead").strip()
        self.assertFalse(branches.is_in_base(str(self.work), "master", ahead))


if __name__ == "__main__":
    unittest.main()
