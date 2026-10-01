"""Structural guards for the step-loop skill's review, hand-off and e2e gates.

The skill is prose, so these pin the load-bearing phrases and the files it
leans on: a rewrite that drops the unconditional security and privacy reviews,
the fix-attempt cap, or the demo hand-off fails here instead of silently.
"""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SKILL = (ROOT / ".claude" / "skills" / "step-loop" / "SKILL.md").read_text(encoding="utf-8")
FLAT_SKILL = " ".join(SKILL.split())
TRIAGE = (ROOT / "docs" / "engineering" / "review-triage.md").read_text(encoding="utf-8")


class ReviewGatesTest(unittest.TestCase):
    def test_skill_names_every_review(self):
        for needle in ("adversarial-review", "holy-blocker-security", "privacy-review"):
            self.assertIn(needle, SKILL)

    def test_security_and_privacy_reviews_are_unconditional(self):
        self.assertIn("unconditionally", SKILL)

    def test_skill_carries_the_fix_attempt_cap_and_triage_doc(self):
        self.assertIn("MAX_FIX_ATTEMPTS", SKILL)
        self.assertIn("review-triage.md", SKILL)

    def test_escalation_packet_is_quoted_verbatim(self):
        packet = "N findings. M auto-fixed and reverified. K stopped at the fix-attempt cap."
        self.assertIn(packet, SKILL)
        self.assertIn(packet, TRIAGE)

    def test_old_back_to_gate_one_rule_is_gone(self):
        self.assertNotIn("load-bearing gap, go back to gate 1", SKILL)

    def test_dependencies_exist_on_disk(self):
        for path in (
            ".claude/skills/privacy-review/SKILL.md",
            ".claude/skills/holy-blocker-security/SKILL.md",
            "docs/engineering/review-triage.md",
        ):
            self.assertTrue((ROOT / path).is_file(), path)


class FeatureGatesTest(unittest.TestCase):
    def test_skill_names_the_e2e_gate_demo_and_preflight_verdict(self):
        for needle in ("e2e", "demo", "environment not ready"):
            self.assertIn(needle, FLAT_SKILL)

    def test_e2e_gate_is_conditional_on_the_runner_existing(self):
        self.assertIn("tools/plan/e2e", SKILL)

    def test_observation_steps_no_longer_always_stop_at_a_pr(self):
        self.assertNotIn("The loop never merges these itself", SKILL)

    def test_skill_points_at_the_loop_contract_tooling(self):
        self.assertIn("tools.plan.loop", SKILL)


if __name__ == "__main__":
    unittest.main()
