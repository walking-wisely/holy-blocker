import contextlib
import io
import json
import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from tools.plan import context


def _line(kind, sidechain=False, model="m", **usage):
    return json.dumps(
        {"type": kind, "isSidechain": sidechain, "message": {"model": model, "usage": usage}}
    )


class LastAssistantTokensTest(unittest.TestCase):
    def _transcript(self, lines):
        tmp = tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False)
        self.addCleanup(os.unlink, tmp.name)
        tmp.write("\n".join(lines) + "\n")
        tmp.close()
        return Path(tmp.name)

    def test_sums_the_three_input_counters_of_the_last_assistant_line(self):
        path = self._transcript(
            [
                _line("assistant", input_tokens=1, cache_creation_input_tokens=1, cache_read_input_tokens=1),
                _line("assistant", input_tokens=2, cache_creation_input_tokens=30, cache_read_input_tokens=400),
                _line("user"),
            ]
        )
        self.assertEqual(context.last_assistant_tokens(path), 432)

    def test_missing_counters_count_as_zero(self):
        path = self._transcript([_line("assistant", input_tokens=7)])
        self.assertEqual(context.last_assistant_tokens(path), 7)

    def test_output_tokens_are_not_counted(self):
        path = self._transcript([_line("assistant", input_tokens=7, output_tokens=999)])
        self.assertEqual(context.last_assistant_tokens(path), 7)

    def test_no_assistant_line_is_none(self):
        self.assertIsNone(context.last_assistant_tokens(self._transcript([_line("user")])))

    def test_sidechain_lines_are_skipped(self):
        path = self._transcript(
            [_line("assistant", input_tokens=250_000), _line("assistant", sidechain=True, input_tokens=9)]
        )
        self.assertEqual(context.last_assistant_tokens(path), 250_000)

    def test_synthetic_lines_are_skipped(self):
        path = self._transcript(
            [_line("assistant", input_tokens=250_000), _line("assistant", model="<synthetic>")]
        )
        self.assertEqual(context.last_assistant_tokens(path), 250_000)

    def test_trailing_zero_usage_line_does_not_erase_a_real_count(self):
        path = self._transcript(
            [_line("assistant", input_tokens=250_000), _line("assistant", input_tokens=0)]
        )
        self.assertEqual(context.last_assistant_tokens(path), 250_000)

    def test_non_numeric_usage_is_skipped(self):
        path = self._transcript(
            [_line("assistant", input_tokens=5), _line("assistant", input_tokens="7")]
        )
        self.assertEqual(context.last_assistant_tokens(path), 5)

    def test_zero_usage_is_no_data(self):
        path = self._transcript([_line("assistant", input_tokens=0)])
        self.assertIsNone(context.last_assistant_tokens(path))

    def test_malformed_lines_are_skipped(self):
        path = self._transcript([_line("assistant", input_tokens=5), '{"type":"assistant" broken'])
        self.assertEqual(context.last_assistant_tokens(path), 5)


class VerdictTest(unittest.TestCase):
    def test_threshold_is_100k_absolute(self):
        self.assertEqual(context.THRESHOLD, 100_000)

    def test_at_threshold_is_under(self):
        self.assertEqual(context.verdict(100_000), "under")

    def test_above_threshold_is_over(self):
        self.assertEqual(context.verdict(100_001), "over")


class ProjectSlugTest(unittest.TestCase):
    def test_non_alphanumerics_become_dashes(self):
        self.assertEqual(
            context.project_slug("/Users/dutov/Life/Personal/holy-blocker"),
            "-Users-dutov-Life-Personal-holy-blocker",
        )

    def test_dots_and_underscores_become_dashes(self):
        self.assertEqual(context.project_slug("/a/.claude/b_c"), "-a--claude-b-c")


class NewestTranscriptTest(unittest.TestCase):
    def test_picks_the_most_recently_modified_jsonl(self):
        with tempfile.TemporaryDirectory() as tmp:
            old, new = Path(tmp, "old.jsonl"), Path(tmp, "new.jsonl")
            old.write_text("")
            new.write_text("")
            os.utime(old, (1, 1))
            os.utime(new, (2, 2))
            Path(tmp, "newer.txt").write_text("")
            self.assertEqual(context.newest_transcript([Path(tmp)]), new)

    def test_searches_every_candidate_directory(self):
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            Path(a, "x.jsonl").write_text("")
            target = Path(b, "y.jsonl")
            target.write_text("")
            os.utime(Path(a, "x.jsonl"), (1, 1))
            self.assertEqual(context.newest_transcript([Path(a), Path(b)]), target)

    def test_no_transcript_is_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(context.newest_transcript([Path(tmp), Path(tmp, "missing")]))


class SessionTranscriptTest(unittest.TestCase):
    def test_finds_the_session_in_any_project_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            mine = Path(tmp, "-some-other-slug", "abc.jsonl")
            mine.parent.mkdir()
            mine.write_text("")
            other = Path(tmp, "-another", "zzz.jsonl")
            other.parent.mkdir()
            other.write_text("")
            os.utime(mine, (1, 1))
            os.utime(other, (2, 2))
            self.assertEqual(context.session_transcript("abc", Path(tmp)), mine)

    def test_duplicate_session_ids_pick_the_newest_copy(self):
        with tempfile.TemporaryDirectory() as tmp:
            old, new = Path(tmp, "a", "s1.jsonl"), Path(tmp, "b", "s1.jsonl")
            for path in (old, new):
                path.parent.mkdir()
                path.write_text("")
            os.utime(old, (1, 1))
            os.utime(new, (2, 2))
            self.assertEqual(context.session_transcript("s1", Path(tmp)), new)

    def test_glob_characters_in_the_id_match_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "a").mkdir()
            Path(tmp, "a", "s1.jsonl").write_text("")
            self.assertIsNone(context.session_transcript("*", Path(tmp)))

    def test_unknown_session_is_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(context.session_transcript("abc", Path(tmp)))


class MainTest(unittest.TestCase):
    def _run(self, tokens):
        tmp = tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False)
        self.addCleanup(os.unlink, tmp.name)
        tmp.write(_line("assistant", input_tokens=tokens) + "\n")
        tmp.close()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = context.main([tmp.name])
        return code, out.getvalue()

    def test_under_prints_count_and_verdict_and_exits_zero(self):
        code, out = self._run(50_000)
        self.assertEqual((code, out.split()[:2]), (0, ["50000", "under"]))

    def test_over_exits_one(self):
        code, out = self._run(150_000)
        self.assertEqual((code, out.split()[:2]), (1, ["150000", "over"]))

    def test_session_id_from_environment_pins_the_transcript(self):
        with tempfile.TemporaryDirectory() as tmp:
            mine = Path(tmp, "slug", "mine.jsonl")
            mine.parent.mkdir()
            mine.write_text(_line("assistant", input_tokens=150_000) + "\n")
            os.utime(mine, (1, 1))
            Path(tmp, "slug", "other.jsonl").write_text(_line("assistant", input_tokens=5) + "\n")
            out = io.StringIO()
            with mock.patch.dict(os.environ, {"CLAUDE_CODE_SESSION_ID": "mine"}), mock.patch.object(
                context, "PROJECTS", Path(tmp)
            ), contextlib.redirect_stdout(out):
                code = context.main([])
            self.assertEqual((code, out.getvalue().split()[:2]), (1, ["150000", "over"]))

    def test_set_but_unmatched_session_id_exits_two_without_falling_back(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "slug").mkdir()
            Path(tmp, "slug", "other.jsonl").write_text(_line("assistant", input_tokens=5) + "\n")
            err = io.StringIO()
            with mock.patch.dict(os.environ, {"CLAUDE_CODE_SESSION_ID": "mine"}), mock.patch.object(
                context, "PROJECTS", Path(tmp)
            ), contextlib.redirect_stderr(err):
                self.assertEqual(context.main([]), 2)
            self.assertIn("mine", err.getvalue())

    def test_unset_session_id_falls_back_to_newest_in_checkout_dirs(self):
        with tempfile.TemporaryDirectory() as tmp:
            newest = Path(tmp, "t.jsonl")
            newest.write_text(_line("assistant", input_tokens=5) + "\n")
            env = {k: v for k, v in os.environ.items() if k != "CLAUDE_CODE_SESSION_ID"}
            out = io.StringIO()
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(
                context, "_project_dirs", return_value=[Path(tmp)]
            ), contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(context.main([]), 0)

    def test_unreadable_transcript_exits_two(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = context.main(["/nonexistent/t.jsonl"])
        self.assertEqual(code, 2)
