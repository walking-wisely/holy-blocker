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


class ManifestPathsTest(unittest.TestCase):
    def _manifests(self) -> list[Path]:
        return sorted(COMPONENTS.glob("*/steps.toml")) + [ENGINEERING / "steps.toml"]

    def test_paths_are_relative_and_inside_the_repo(self):
        for manifest in self._manifests():
            for path in ledger.load_paths(manifest.parent):
                self.assertFalse(path.startswith("/"), f"{manifest}: {path}")
                self.assertNotIn("..", Path(path).parts, f"{manifest}: {path}")

    def test_no_two_manifests_claim_the_same_root(self):
        owners: dict[str, Path] = {}
        for manifest in self._manifests():
            for path in ledger.load_paths(manifest.parent):
                key = path.strip("/")
                self.assertNotIn(key, owners, f"{key} claimed by {owners.get(key)} and {manifest}")
                owners[key] = manifest

    def test_every_existing_code_root_is_claimed(self):
        claimed = {
            path.strip("/")
            for manifest in self._manifests()
            for path in ledger.load_paths(manifest.parent)
        }
        unclaimed = [
            f"{top}/{child.name}"
            for top in ("apps", "packages", "native-modules")
            for child in sorted((ROOT / top).iterdir())
            if child.is_dir() and f"{top}/{child.name}" not in claimed
        ]
        self.assertEqual(unclaimed, [], f"code roots no manifest governs: {unclaimed}")

    def test_ci_path_filter_covers_every_governed_root(self):
        workflow = (ROOT / ".github" / "workflows" / "ci-plan.yml").read_text(encoding="utf-8")
        missing = [
            path
            for manifest in self._manifests()
            for path in ledger.load_paths(manifest.parent)
            if f'- "{path.strip("/")}/**"' not in workflow
        ]
        self.assertEqual(missing, [], f"roots missing from the ci-plan path filter: {missing}")
