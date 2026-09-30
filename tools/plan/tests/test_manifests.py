"""Repo-wide guard: every component plan is covered by a valid step manifest.

A component that grows a plan.md but no steps.toml is invisible to
``ledger next`` and drifts back into hand-maintained prose. This test fails that
case, and fails any manifest whose steps and plan markers disagree.
"""

import unittest
from pathlib import Path

from tools.plan import ledger

ROOT = Path(__file__).resolve().parents[3]
COMPONENTS = ROOT / "docs" / "components"
ENGINEERING = ROOT / "docs" / "engineering"


class ManifestCoverageTest(unittest.TestCase):
    def _component_dirs(self) -> list[Path]:
        return sorted(
            child
            for child in COMPONENTS.iterdir()
            if child.is_dir() and (child / "plan.md").exists()
        )

    def test_every_component_with_a_plan_has_a_manifest(self):
        missing = [
            child.name
            for child in self._component_dirs()
            if not (child / "steps.toml").exists()
        ]
        self.assertEqual(missing, [], f"components with no steps.toml: {missing}")

    def test_every_manifest_validates_against_its_plan(self):
        problems: list[str] = []
        for child in self._component_dirs():
            manifest = child / "steps.toml"
            if not manifest.exists():
                continue
            plan = (child / "plan.md").read_text(encoding="utf-8")
            steps = ledger.load_manifest(child)
            for problem in ledger.validate(steps, ledger.extract_markers(plan)):
                problems.append(f"{child.name}: {problem}")
        self.assertEqual(problems, [], "\n".join(problems))

    def test_engineering_manifest_validates(self):
        plan = (ENGINEERING / "plan.md").read_text(encoding="utf-8")
        steps = ledger.load_manifest(ENGINEERING)
        problems = ledger.validate(steps, ledger.extract_markers(plan))
        self.assertEqual(problems, [], "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
