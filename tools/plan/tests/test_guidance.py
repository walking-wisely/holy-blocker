"""Guards for the agent-facing guidance files.

CLAUDE.md is the one canonical file and carries the router that decides which
loop a request enters. AGENTS.md exists only so a client that reads it alone
still finds CLAUDE.md, and must not grow a second copy of project state.
"""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CLAUDE = (ROOT / "CLAUDE.md").read_text(encoding="utf-8")
AGENTS = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
STEP_LOOP = (ROOT / ".claude" / "skills" / "step-loop" / "SKILL.md").read_text(
    encoding="utf-8"
)

PLAN_INCEPTION = (
    ROOT / ".claude" / "skills" / "plan-inception" / "SKILL.md"
).read_text(encoding="utf-8")

ROUTER_HEADING = "## Routing — which loop applies"


class RouterTest(unittest.TestCase):
    def test_claude_md_has_the_router(self):
        self.assertIn(ROUTER_HEADING, CLAUDE)

    def test_router_names_every_entry_point(self):
        section = CLAUDE.split(ROUTER_HEADING, 1)[1].split("\n## ", 1)[0]
        for name in ("plan-inception", "step-loop", "Loop-lite", "Exempt"):
            self.assertIn(name, section)

    def test_router_precedes_working_rhythm(self):
        self.assertLess(
            CLAUDE.index(ROUTER_HEADING),
            CLAUDE.index("## Working Rhythm"),
        )

    def test_step_loop_points_at_the_router(self):
        self.assertIn("Routing — which loop applies", STEP_LOOP)


class PlanInceptionPublishTest(unittest.TestCase):
    def test_does_not_forbid_the_worktree_or_the_pr(self):
        self.assertNotIn("fork a worktree, or open a PR", PLAN_INCEPTION)

    def test_publishes_through_the_working_rhythm(self):
        step = PLAN_INCEPTION.split("## Step 6", 1)[1].split("\n## ", 1)[0]
        for needle in ("Working Rhythm", "worktree", "open the PR"):
            self.assertIn(needle, step)


class AgentsPointerTest(unittest.TestCase):
    def test_agents_md_names_claude_md(self):
        self.assertIn("CLAUDE.md", AGENTS)

    def test_agents_md_keeps_no_state_of_its_own(self):
        self.assertNotIn("planned but not yet created", AGENTS)
        self.assertNotIn("| Package", AGENTS)

    def test_agents_md_is_one_screen(self):
        self.assertLessEqual(len(AGENTS.splitlines()), 40)


if __name__ == "__main__":
    unittest.main()
