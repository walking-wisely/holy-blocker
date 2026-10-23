import json
import os
import stat
import tempfile
import unittest
from pathlib import Path

from tools.plan import sessionlog


def _ts(second):
    return f"2026-10-01T10:{second // 60:02d}:{second % 60:02d}.000Z"


def _prompt(second, text="go", origin="human", **extra):
    return {
        "type": "user",
        "timestamp": _ts(second),
        "origin": {"kind": origin},
        "message": {"role": "user", "content": text},
        **extra,
    }


def _assistant(second, blocks, output_tokens=10, input_tokens=100, model="m", message_id=None):
    return {
        "type": "assistant",
        "timestamp": _ts(second),
        "message": {
            "id": message_id or f"msg-{second}",
            "model": model,
            "content": blocks,
            "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens},
        },
    }


def _tool_use(call_id, name, **tool_input):
    return {"type": "tool_use", "id": call_id, "name": name, "input": tool_input}


def _tool_result(second, call_id, content="ok", is_error=False):
    return {
        "type": "user",
        "timestamp": _ts(second),
        "message": {
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": call_id, "content": content, "is_error": is_error}
            ],
        },
    }


def _write(directory, name, entries):
    path = Path(directory) / f"{name}.jsonl"
    path.write_text("\n".join(json.dumps(e) for e in entries) + "\n", encoding="utf-8")
    return path


class SummarizeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def summarize(self, entries, name="abc"):
        return sessionlog.summarize(_write(self.tmp.name, name, entries))

    def test_waiting_on_a_human_is_the_gap_before_each_later_human_prompt(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(10, [{"type": "text", "text": "done"}]),
            _prompt(70),
            _assistant(80, [{"type": "text", "text": "done"}]),
            _prompt(500),
        ])
        self.assertEqual(summary["human_prompts"], 3)
        self.assertEqual(summary["wait_s"], 60 + 420)
        self.assertEqual(summary["longest_wait_s"], 420)
        self.assertEqual(summary["duration_s"], 500)
        self.assertEqual(summary["active_s"], 20)

    def test_sdk_prompts_are_not_waits(self):
        summary = self.summarize([
            _prompt(0, origin="sdk"),
            _assistant(30, [{"type": "text", "text": "x"}]),
            _prompt(900, origin="sdk"),
        ])
        self.assertEqual(summary["human_prompts"], 0)
        self.assertEqual(summary["wait_s"], 0)

    def test_tool_results_are_not_prompts(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(5, [_tool_use("t1", "Bash", command="git status")]),
            _tool_result(9, "t1"),
        ])
        self.assertEqual(summary["human_prompts"], 1)
        self.assertEqual(summary["tool_counts"], {"Bash": 1})

    def test_tool_seconds_pair_use_with_result(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(5, [_tool_use("t1", "Bash", command="cargo test")]),
            _tool_result(65, "t1"),
        ])
        self.assertEqual(summary["tool_seconds"], {"Bash": 60})
        self.assertEqual(summary["bash_heads"], {"cargo": 1})

    def test_errors_and_permission_blocks_are_counted(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(1, [_tool_use("t1", "Bash", command="x"), _tool_use("t2", "Edit", file_path="/a")]),
            _tool_result(2, "t1", "Permission to use Bash has been denied", is_error=True),
            _tool_result(3, "t2", "boom", is_error=True),
        ])
        self.assertEqual(summary["tool_errors"], {"Bash": 1, "Edit": 1})
        self.assertEqual(summary["permission_blocks"], 1)

    def test_skills_subagents_and_edited_files(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(1, [
                _tool_use("a", "Skill", skill="step-loop"),
                _tool_use("b", "Agent", subagent_type="Explore"),
                _tool_use("c", "Edit", file_path="/r/a.py"),
                _tool_use("d", "Write", file_path="/r/a.py"),
                _tool_use("e", "Write", file_path="/r/b.py"),
            ]),
        ])
        self.assertEqual(summary["skills"], {"step-loop": 1})
        self.assertEqual(summary["subagents"], 1)
        self.assertEqual(summary["files_edited"], 2)

    def test_tokens_cost_and_peak_context(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(1, [], output_tokens=5, input_tokens=1000),
            _assistant(2, [], output_tokens=7, input_tokens=4000),
            {"type": "cost-state", "totalCostUSD": 1.5},
            {"type": "cost-state", "totalCostUSD": 2.25},
        ])
        self.assertEqual(summary["output_tokens"], 12)
        self.assertEqual(summary["peak_context"], 4000)
        self.assertEqual(summary["cost_usd"], 2.25)

    def test_kind_and_branch(self):
        summary = self.summarize([_prompt(0, entrypoint="sdk-cli", gitBranch="feat/x")])
        self.assertEqual(summary["kind"], "driver")
        self.assertEqual(summary["branch"], "feat/x")

    def test_titles_and_working_directories_are_not_stored(self):
        summary = self.summarize([
            _prompt(0, cwd="/Users/someone/private"),
            {"type": "ai-title", "aiTitle": "a sensitive title"},
        ])
        self.assertNotIn("title", summary)
        self.assertNotIn("cwd", summary)
        self.assertNotIn("sensitive", json.dumps(summary))
        self.assertNotIn("someone", json.dumps(summary))

    def test_command_heads_skills_and_tool_names_are_bucketed_unless_plain_identifiers(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(1, [
                _tool_use("a", "Bash", command="API_TOKEN=sk-abc123 curl https://x"),
                _tool_use("b", "Bash", command="some-site.example/path --flag"),
                _tool_use("c", "Bash", command="cargo test"),
                _tool_use("d", "Skill", skill="a b/c"),
                _tool_use("e", "mcp__server__tool", x=1),
            ]),
        ])
        self.assertEqual(summary["bash_heads"], {"other": 2, "cargo": 1})
        self.assertEqual(summary["skills"], {"other": 1})
        self.assertEqual(summary["tool_counts"]["mcp__server__tool"], 1)
        self.assertNotIn("sk-abc123", json.dumps(summary))

    def test_one_message_split_across_lines_is_counted_once(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(1, [], output_tokens=5, message_id="m1"),
            _assistant(2, [], output_tokens=40, message_id="m1"),
            _assistant(3, [], output_tokens=7, message_id="m2"),
        ])
        self.assertEqual(summary["assistant_msgs"], 2)
        self.assertEqual(summary["output_tokens"], 47)

    def test_a_long_tool_run_stays_active_time(self):
        summary = self.summarize([
            _prompt(0),
            _assistant(5, [_tool_use("t1", "Bash", command="cargo build")]),
            _tool_result(5 + 3 * sessionlog.IDLE_S, "t1"),
        ])
        self.assertEqual(summary["active_s"], 5 + 3 * sessionlog.IDLE_S)

    def test_interactive_is_the_default_kind(self):
        self.assertEqual(self.summarize([_prompt(0, entrypoint="cli")])["kind"], "interactive")

    def test_unreadable_lines_are_skipped(self):
        path = Path(self.tmp.name) / "bad.jsonl"
        path.write_text("not json\n" + json.dumps(_prompt(0)) + "\n[]\n", encoding="utf-8")
        self.assertEqual(sessionlog.summarize(path)["human_prompts"], 1)

    def test_no_prompt_text_is_stored(self):
        summary = self.summarize([_prompt(0, text="a secret sentence")])
        self.assertNotIn("secret", json.dumps(summary))

    def test_session_id_is_the_file_stem(self):
        self.assertEqual(self.summarize([_prompt(0)], name="sid-1")["session_id"], "sid-1")


class IndexTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.projects = self.root / "projects"
        self.index = self.root / "sessions" / "index.jsonl"

    def transcript(self, project, name, entries):
        directory = self.projects / project
        directory.mkdir(parents=True, exist_ok=True)
        return _write(directory, name, entries)

    def test_worktree_projects_are_included_and_other_repos_are_not(self):
        self.transcript("-r-holy-blocker", "a", [_prompt(0)])
        self.transcript("-r-holy-blocker--claude-worktrees-x", "b", [_prompt(0)])
        self.transcript("-r-other", "c", [_prompt(0)])
        rows = sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        self.assertEqual(sorted(r["session_id"] for r in rows), ["a", "b"])

    def test_subagent_transcripts_are_not_sessions(self):
        self.transcript("-r-holy-blocker", "a", [_prompt(0)])
        nested = self.projects / "-r-holy-blocker" / "a" / "subagents"
        nested.mkdir(parents=True)
        _write(nested, "agent-1", [_prompt(0)])
        rows = sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        self.assertEqual([r["session_id"] for r in rows], ["a"])

    def test_a_sibling_project_with_a_longer_name_is_not_included(self):
        self.transcript("-r-holy-blocker", "a", [_prompt(0)])
        self.transcript("-r-holy-blocker-other", "b", [_prompt(0)])
        rows = sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        self.assertEqual([r["session_id"] for r in rows], ["a"])

    def test_index_is_owner_only_and_rows_past_retention_are_dropped(self):
        self.transcript("-r-holy-blocker", "new", [_prompt(0)])
        old = self.transcript("-r-holy-blocker", "old", [_prompt(0)])
        sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        rows = sessionlog.read_index(self.index)
        for row in rows:
            if row["session_id"] == "old":
                row["start"] = "2020-01-01T00:00:00+00:00"
        self.index.write_text("".join(json.dumps(r) + "\n" for r in rows))
        os.unlink(old)
        kept = sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        self.assertEqual([r["session_id"] for r in kept], ["new"])
        self.assertEqual(stat.S_IMODE(self.index.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.index.parent.stat().st_mode), 0o700)

    def test_write_private_replaces_a_planted_symlink_instead_of_following_it(self):
        victim = self.root / "victim"
        victim.write_text("keep")
        self.index.parent.mkdir(parents=True)
        target = self.index.parent / "index.tmp"
        target.symlink_to(victim)
        sessionlog.write_private(self.index, "x\n")
        self.assertEqual(victim.read_text(), "keep")

    def test_unchanged_transcripts_are_not_resummarized(self):
        self.transcript("-r-holy-blocker", "a", [_prompt(0)])
        sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        calls = []
        original = sessionlog.summarize
        sessionlog.summarize = lambda path: calls.append(path) or original(path)
        self.addCleanup(setattr, sessionlog, "summarize", original)
        sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        self.assertEqual(calls, [])

    def test_index_persists_between_refreshes(self):
        self.transcript("-r-holy-blocker", "a", [_prompt(0)])
        sessionlog.refresh(self.projects, "-r-holy-blocker", self.index)
        self.assertEqual(len(sessionlog.read_index(self.index)), 1)


class ReportTest(unittest.TestCase):
    def rows(self):
        base = {
            "kind": "interactive", "title": "t", "branch": "b", "start": "2026-10-01T10:00:00+00:00",
            "duration_s": 600, "active_s": 100, "wait_s": 500, "longest_wait_s": 400,
            "cost_usd": 1.0, "tool_counts": {"Bash": 3}, "tool_errors": {"Bash": 2},
            "tool_seconds": {"Bash": 90}, "permission_blocks": 1, "human_prompts": 4,
        }
        return [
            {**base, "session_id": "s1"},
            {**base, "session_id": "s2", "kind": "driver", "wait_s": 0, "longest_wait_s": 0},
        ]

    def test_report_filters_by_kind(self):
        text = sessionlog.report(self.rows(), kind="driver", runs={})
        self.assertIn("s2", text)
        self.assertNotIn("s1", text)

    def test_report_never_prints_a_title(self):
        rows = [{**r, "title": "leaky title"} for r in self.rows()]
        self.assertNotIn("leaky", sessionlog.report(rows, kind=None, runs={}))

    def test_report_joins_driver_runs(self):
        runs = {"s2": {"step": "pkg.step", "outcome": "merged"}}
        text = sessionlog.report(self.rows(), kind=None, runs=runs)
        self.assertIn("pkg.step", text)
        self.assertIn("merged", text)

    def test_report_has_friction_section(self):
        text = sessionlog.report(self.rows(), kind=None, runs={})
        self.assertIn("Friction", text)
        self.assertIn("Longest waits", text)

    def test_since_parses_days_and_hours(self):
        self.assertEqual(sessionlog.parse_since("7d").total_seconds(), 7 * 86400)
        self.assertEqual(sessionlog.parse_since("12h").total_seconds(), 12 * 3600)
        with self.assertRaises(ValueError):
            sessionlog.parse_since("soon")


if __name__ == "__main__":
    unittest.main()
