import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.plan import state


PORCELAIN = """worktree /repo
HEAD 1111111111111111111111111111111111111111
branch refs/heads/master

worktree /repo/.claude/worktrees/a
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feat/a

worktree /repo/.claude/worktrees/b
HEAD 3333333333333333333333333333333333333333
detached
"""


class ParseWorktreesTest(unittest.TestCase):
    def test_parses_paths_and_branches(self):
        trees = state.parse_worktrees(PORCELAIN)
        self.assertEqual([t.path for t in trees], ["/repo", "/repo/.claude/worktrees/a", "/repo/.claude/worktrees/b"])
        self.assertEqual([t.branch for t in trees], ["master", "feat/a", None])

    def test_empty_input(self):
        self.assertEqual(state.parse_worktrees(""), [])

    def test_marks_prunable_worktrees(self):
        text = PORCELAIN + "\nworktree /gone\nHEAD 4444\ndetached\nprunable gitdir file points to non-existent location\n"
        trees = state.parse_worktrees(text)
        self.assertEqual([t.prunable for t in trees], [False, False, False, True])


class SummariseChecksTest(unittest.TestCase):
    def test_empty_is_none(self):
        self.assertEqual(state.summarise_checks([]), "none")

    def test_all_success_is_passing(self):
        rollup = [{"status": "COMPLETED", "conclusion": "SUCCESS"}]
        self.assertEqual(state.summarise_checks(rollup), "passing")

    def test_failure_outranks_pending(self):
        rollup = [
            {"status": "IN_PROGRESS", "conclusion": ""},
            {"status": "COMPLETED", "conclusion": "FAILURE"},
        ]
        self.assertEqual(state.summarise_checks(rollup), "failing")

    def test_in_progress_is_pending(self):
        rollup = [{"status": "IN_PROGRESS", "conclusion": ""}]
        self.assertEqual(state.summarise_checks(rollup), "pending")

    def test_status_context_success(self):
        rollup = [{"state": "SUCCESS"}]
        self.assertEqual(state.summarise_checks(rollup), "passing")


class ClassifyTest(unittest.TestCase):
    def _wt(self, branch="feat/x"):
        return state.Worktree(path="/repo/.claude/worktrees/x", head="abc", branch=branch)

    def _pr(self, state_="OPEN"):
        return state.PullRequest(number=1, state=state_, checks="passing", head_oid="abc", base="master")

    def test_detached_is_not_reapable(self):
        got = state.classify(self._wt(branch=None), None, 0, 0, False)
        self.assertEqual(got.verdict, "detached")
        self.assertFalse(state.reapable(got))

    def test_dirty_beats_pr_state(self):
        got = state.classify(self._wt(), self._pr("MERGED"), 0, 0, True)
        self.assertEqual(got.verdict, "dirty")
        self.assertFalse(state.reapable(got))

    def test_merged_clean_is_reapable(self):
        got = state.classify(self._wt(), self._pr("MERGED"), 0, 0, False)
        self.assertEqual(got.verdict, "merged")
        self.assertTrue(state.reapable(got))

    def test_squash_merged_branch_is_still_reapable(self):
        # A squash merge leaves the branch's original commits ahead of base; the
        # merged PR is what says the work landed, not `base..branch == 0`.
        got = state.classify(self._wt(), self._pr("MERGED"), ahead=2, local_only=0, dirty=False)
        self.assertEqual(got.verdict, "merged")
        self.assertTrue(state.reapable(got))

    def test_local_only_commits_beat_a_merged_pr(self):
        got = state.classify(self._wt(), self._pr("MERGED"), ahead=1, local_only=1, dirty=False)
        self.assertEqual(got.verdict, "local-only")
        self.assertFalse(state.reapable(got))

    def test_open_is_not_reapable(self):
        got = state.classify(self._wt(), self._pr("OPEN"), ahead=1, local_only=0, dirty=False)
        self.assertEqual(got.verdict, "open")
        self.assertFalse(state.reapable(got))

    def test_no_pr_with_commits_is_unpushed(self):
        got = state.classify(self._wt(), None, ahead=3, local_only=0, dirty=False)
        self.assertEqual(got.verdict, "unpushed")
        self.assertFalse(state.reapable(got))

    def test_no_pr_no_commits_is_abandoned_and_reapable(self):
        got = state.classify(self._wt(), None, 0, 0, False)
        self.assertEqual(got.verdict, "abandoned")
        self.assertTrue(state.reapable(got))

    def test_closed_with_commits_is_not_reapable(self):
        got = state.classify(self._wt(), self._pr("CLOSED"), ahead=1, local_only=0, dirty=False)
        self.assertEqual(got.verdict, "closed")
        self.assertFalse(state.reapable(got))

    def test_closed_without_commits_is_reapable(self):
        got = state.classify(self._wt(), self._pr("CLOSED"), 0, 0, False)
        self.assertEqual(got.verdict, "abandoned")
        self.assertTrue(state.reapable(got))


class ClassifyLandedTest(unittest.TestCase):
    def _wt(self):
        return state.Worktree(path="/repo/.claude/worktrees/x", head="abc", branch="feat/x")

    def _classify(self, pull_request=None, ahead=3, local_only=0, dirty=False, landed=True):
        return state.classify(self._wt(), pull_request, ahead, local_only, dirty, landed=landed)

    def test_landed_without_pr_is_reapable(self):
        got = self._classify()
        self.assertEqual(got.verdict, "landed")
        self.assertTrue(state.reapable(got))

    def test_landed_beats_closed_pr(self):
        pr = state.PullRequest(number=1, state="CLOSED", checks="none", head_oid="abc", base="master")
        self.assertEqual(self._classify(pr).verdict, "landed")

    def test_landed_beats_local_only(self):
        self.assertEqual(self._classify(local_only=2).verdict, "landed")

    def test_dirty_beats_landed(self):
        self.assertEqual(self._classify(dirty=True).verdict, "dirty")

    def test_open_pr_beats_landed(self):
        pr = state.PullRequest(number=1, state="OPEN", checks="none", head_oid="abc", base="master")
        self.assertEqual(self._classify(pr).verdict, "open")

    def test_not_landed_keeps_unpushed(self):
        self.assertEqual(self._classify(landed=False).verdict, "unpushed")


def _git(cwd, *args):
    subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    )


class ContentLandedTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.repo = self._tmp.name
        _git(self.repo, "init", "-q", "-b", "master")
        self._write("base.txt", "one\ntwo\nthree\n")
        _git(self.repo, "add", ".")
        _git(self.repo, "commit", "-q", "-m", "base")

    def _write(self, name, text):
        Path(self.repo, name).write_text(text)

    def _commit(self, message):
        _git(self.repo, "add", ".")
        _git(self.repo, "commit", "-q", "-m", message)

    def _feature_with_two_commits(self):
        _git(self.repo, "checkout", "-q", "-b", "feat/x")
        self._write("a.txt", "a\n")
        self._commit("a")
        self._write("b.txt", "b\n")
        self._commit("b")

    def test_unmerged_branch_has_not_landed(self):
        self._feature_with_two_commits()
        self.assertFalse(state.content_landed("master", "feat/x", self.repo))

    def test_squash_merge_has_landed(self):
        self._feature_with_two_commits()
        _git(self.repo, "checkout", "-q", "master")
        _git(self.repo, "merge", "-q", "--squash", "feat/x")
        self._commit("squash")
        self.assertTrue(state.content_landed("master", "feat/x", self.repo))

    def test_rebased_cherry_pick_has_landed(self):
        self._feature_with_two_commits()
        _git(self.repo, "checkout", "-q", "master")
        self._write("other.txt", "x\n")
        self._commit("unrelated")
        _git(self.repo, "cherry-pick", "feat/x~1", "feat/x")
        self.assertTrue(state.content_landed("master", "feat/x", self.repo))

    def test_partial_landing_has_not_landed(self):
        self._feature_with_two_commits()
        _git(self.repo, "checkout", "-q", "master")
        _git(self.repo, "cherry-pick", "feat/x~1")
        self.assertFalse(state.content_landed("master", "feat/x", self.repo))

    def test_conflicting_branch_has_not_landed(self):
        _git(self.repo, "checkout", "-q", "-b", "feat/x")
        self._write("base.txt", "one\nTWO\nthree\n")
        self._commit("branch edit")
        _git(self.repo, "checkout", "-q", "master")
        self._write("base.txt", "one\n2\nthree\n")
        self._commit("master edit")
        self.assertFalse(state.content_landed("master", "feat/x", self.repo))

    def test_change_reverted_on_master_has_not_landed(self):
        self._feature_with_two_commits()
        _git(self.repo, "checkout", "-q", "master")
        _git(self.repo, "merge", "-q", "--squash", "feat/x")
        self._commit("squash")
        _git(self.repo, "rm", "-q", "a.txt")
        self._commit("revert a")
        self.assertFalse(state.content_landed("master", "feat/x", self.repo))

    def test_residual_lists_files_still_unlanded(self):
        self._feature_with_two_commits()
        _git(self.repo, "checkout", "-q", "master")
        _git(self.repo, "cherry-pick", "feat/x~1")
        self.assertEqual(state.residual_files("master", "feat/x", self.repo), ["b.txt"])

    def test_residual_is_empty_when_landed(self):
        self._feature_with_two_commits()
        _git(self.repo, "checkout", "-q", "master")
        _git(self.repo, "merge", "-q", "--squash", "feat/x")
        self._commit("squash")
        self.assertEqual(state.residual_files("master", "feat/x", self.repo), [])

    def test_residual_is_none_on_conflict(self):
        _git(self.repo, "checkout", "-q", "-b", "feat/x")
        self._write("base.txt", "one\nTWO\nthree\n")
        self._commit("branch edit")
        _git(self.repo, "checkout", "-q", "master")
        self._write("base.txt", "one\n2\nthree\n")
        self._commit("master edit")
        self.assertIsNone(state.residual_files("master", "feat/x", self.repo))


class ParsePrTest(unittest.TestCase):
    def test_empty_list_is_none(self):
        self.assertIsNone(state.parse_pr("[]"))

    def test_reads_first_with_checks(self):
        payload = '[{"number": 48, "state": "OPEN", "statusCheckRollup": [{"status": "COMPLETED", "conclusion": "SUCCESS"}]}]'
        pr = state.parse_pr(payload)
        self.assertEqual((pr.number, pr.state, pr.checks), (48, "OPEN", "passing"))


class MatchingPrTest(unittest.TestCase):
    def _wt(self, head="abc"):
        return state.Worktree(path="/w", head=head, branch="feat/x")

    def _pr(self, head_oid="abc", base="master"):
        return state.PullRequest(number=1, state="MERGED", checks="passing", head_oid=head_oid, base=base)

    def test_matches_on_head_and_base(self):
        self.assertIsNotNone(state.matching_pr(self._wt(), self._pr(), "master"))

    def test_rejects_reused_branch_name(self):
        # A recreated branch with a stale merged PR of the same name must not match.
        self.assertIsNone(state.matching_pr(self._wt(head="newtip"), self._pr(head_oid="oldtip"), "master"))

    def test_rejects_pr_targeting_another_base(self):
        self.assertIsNone(state.matching_pr(self._wt(), self._pr(base="release"), "master"))

    def test_none_stays_none(self):
        self.assertIsNone(state.matching_pr(self._wt(), None, "master"))


class ReachesBaseTest(unittest.TestCase):
    def _pr(self, state_="MERGED", base="master"):
        return state.PullRequest(number=1, state=state_, checks="none", head_oid="abc", base=base)

    def test_direct_merge_into_base(self):
        self.assertTrue(state.reaches_base(self._pr(), "master", lambda branch: None))

    def test_stacked_chain_reaching_base(self):
        parents = {"mid": self._pr(base="top"), "top": self._pr(base="master")}
        self.assertTrue(state.reaches_base(self._pr(base="mid"), "master", parents.get))

    def test_unmerged_pr_does_not_reach(self):
        self.assertFalse(state.reaches_base(self._pr("OPEN"), "master", lambda branch: None))

    def test_parent_closed_breaks_the_chain(self):
        parents = {"mid": self._pr("CLOSED", base="master")}
        self.assertFalse(state.reaches_base(self._pr(base="mid"), "master", parents.get))

    def test_missing_parent_breaks_the_chain(self):
        self.assertFalse(state.reaches_base(self._pr(base="mid"), "master", lambda branch: None))

    def test_cycle_terminates(self):
        parents = {"a": self._pr(base="b"), "b": self._pr(base="a")}
        self.assertFalse(state.reaches_base(self._pr(base="a"), "master", parents.get))


class IgnoredTest(unittest.TestCase):
    def test_parses_ignored_paths(self):
        text = "?? tracked-untracked.txt\n!! node_modules/\n!! packages/x/target/\n"
        self.assertEqual(state.parse_ignored(text), ["node_modules/", "packages/x/target/"])

    def test_build_output_is_disposable(self):
        self.assertTrue(state.is_disposable_ignored(["node_modules/", "packages/x/target/", ".husky/_/"]))

    def test_env_file_is_not_disposable(self):
        self.assertFalse(state.is_disposable_ignored([".env"]))

    def test_empty_is_disposable(self):
        self.assertTrue(state.is_disposable_ignored([]))


class SelectReapableTest(unittest.TestCase):
    def _row(self, path, verdict, ignored_state=False):
        return state.Row(path=path, branch="b", verdict=verdict, reason="", ignored_state=ignored_state)

    def test_selects_only_reapable_and_unprotected(self):
        rows = [
            self._row("/main", "merged"),
            self._row("/w/a", "merged"),
            self._row("/w/b", "open"),
            self._row("/w/c", "abandoned"),
        ]
        got = state.select_reapable(rows, protected={"/main"})
        self.assertEqual([r.path for r in got], ["/w/a", "/w/c"])

    def test_open_is_never_selected(self):
        rows = [self._row("/w/a", "open")]
        self.assertEqual(state.select_reapable(rows, protected=set()), [])

    def test_ignored_state_blocks_merge_until_forced(self):
        rows = [self._row("/w/a", "merged", ignored_state=True)]
        self.assertEqual(state.select_reapable(rows, protected=set()), [])
        self.assertEqual([r.path for r in state.select_reapable(rows, protected=set(), force=True)], ["/w/a"])

    def test_force_does_not_override_the_verdict(self):
        rows = [self._row("/w/a", "open", ignored_state=True)]
        self.assertEqual(state.select_reapable(rows, protected=set(), force=True), [])


class ParsePrTest(unittest.TestCase):
    def test_empty_list_is_none(self):
        self.assertIsNone(state.parse_pr("[]"))

    def test_reads_first_with_checks_and_refs(self):
        payload = (
            '[{"number": 48, "state": "OPEN", "baseRefName": "master", "headRefOid": "deadbeef",'
            ' "statusCheckRollup": [{"status": "COMPLETED", "conclusion": "SUCCESS"}]}]'
        )
        pr = state.parse_pr(payload)
        self.assertEqual((pr.number, pr.state, pr.checks, pr.head_oid, pr.base), (48, "OPEN", "passing", "deadbeef", "master"))


if __name__ == "__main__":
    unittest.main()
