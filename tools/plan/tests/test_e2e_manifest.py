import tempfile
import unittest
from pathlib import Path

from tools.plan.e2e import manifest

VALID = """
id = "android-smoke"
platform = "android"
covers = ["demos/android", "packages/a"]
proves = ["row-1"]
does_not_prove = "vendor skins"
preflight = "demos/android/preflight.sh"
run = "demos/android/run.sh"
"""


def write(root: Path, rel: str, text: str = "") -> Path:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def crate(root: Path, name: str, deps: dict[str, str] | None = None, table: str = "dependencies"):
    body = f'[package]\nname = "{name}"\nversion = "0.1.0"\n'
    if deps:
        body += f"[{table}]\n" + "".join(
            f'{dep} = {{ path = "{path}" }}\n' for dep, path in deps.items()
        )
    write(root, f"packages/{name}/Cargo.toml", body)


class LoadTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def load(self, text: str):
        return manifest.load(write(self.root, "demos/android/scenario.toml", text))

    def test_loads_a_valid_manifest(self):
        scenario = self.load(VALID)
        self.assertEqual(scenario.id, "android-smoke")
        self.assertEqual(scenario.platform, "android")
        self.assertEqual(scenario.covers, ("demos/android", "packages/a"))
        self.assertIsNone(scenario.interactive)

    def test_missing_field_is_named(self):
        with self.assertRaisesRegex(manifest.ManifestError, "proves"):
            self.load(VALID.replace('proves = ["row-1"]\n', ""))

    def test_unknown_field_is_rejected(self):
        with self.assertRaisesRegex(manifest.ManifestError, "surprise"):
            self.load(VALID + 'surprise = "x"\n')

    def test_unknown_platform_is_rejected(self):
        with self.assertRaisesRegex(manifest.ManifestError, "platform"):
            self.load(VALID.replace('"android"\ncovers', '"beos"\ncovers'))

    def test_empty_covers_is_rejected(self):
        with self.assertRaisesRegex(manifest.ManifestError, "covers"):
            self.load(VALID.replace('["demos/android", "packages/a"]', "[]"))

    def test_covers_must_be_repo_relative(self):
        for bad in ("/etc", "../x", "a/../b"):
            with self.subTest(bad=bad):
                with self.assertRaisesRegex(manifest.ManifestError, "covers"):
                    self.load(VALID.replace("packages/a", bad))

    def test_blank_does_not_prove_is_rejected(self):
        with self.assertRaisesRegex(manifest.ManifestError, "does_not_prove"):
            self.load(VALID.replace('"vendor skins"', '"  "'))

    def test_invalid_toml_is_a_manifest_error(self):
        with self.assertRaises(manifest.ManifestError):
            self.load("id = ")

    def test_covers_trailing_slash_is_normalised(self):
        scenario = self.load(VALID.replace('"packages/a"', '"packages/a/"'))
        self.assertEqual(scenario.covers, ("demos/android", "packages/a"))


class DiscoverTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_no_demos_directory_is_not_a_problem(self):
        self.assertEqual(manifest.problems(self.root), [])

    def test_duplicate_ids_are_reported(self):
        write(self.root, "demos/one/scenario.toml", VALID.replace("demos/android", "demos/one"))
        write(self.root, "demos/two/scenario.toml", VALID.replace("demos/android", "demos/two"))
        found = manifest.problems(self.root)
        self.assertTrue(any("duplicate scenario id" in p for p in found), found)

    def test_invalid_manifest_is_reported_with_its_path(self):
        write(self.root, "demos/bad/scenario.toml", 'id = "x"\n')
        found = manifest.problems(self.root)
        self.assertTrue(any("demos/bad/scenario.toml" in p for p in found), found)


class ClosureTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        write(self.root, "demos/android/run.sh")
        write(self.root, "demos/android/preflight.sh")

    def scenario(self, covers: list[str], **extra):
        text = VALID.replace(
            '["demos/android", "packages/a"]', "[" + ", ".join(f'"{c}"' for c in covers) + "]"
        )
        for key, value in extra.items():
            text += f'{key} = "{value}"\n'
        write(self.root, "demos/android/scenario.toml", text)
        return manifest.problems(self.root)

    def test_closed_covers_has_no_problems(self):
        crate(self.root, "a", {"b": "../b"})
        crate(self.root, "b")
        self.assertEqual(self.scenario(["demos/android", "packages/a", "packages/b"]), [])

    def test_missing_path_dependency_is_named(self):
        crate(self.root, "a", {"b": "../b"})
        crate(self.root, "b")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertEqual(len(found), 1, found)
        self.assertIn("packages/b", found[0])

    def test_closure_is_transitive(self):
        crate(self.root, "a", {"b": "../b"})
        crate(self.root, "b", {"c": "../c"})
        crate(self.root, "c")
        found = self.scenario(["demos/android", "packages/a", "packages/b"])
        self.assertTrue(any("packages/c" in p for p in found), found)

    def test_dev_and_build_dependencies_count(self):
        crate(self.root, "a", {"b": "../b"}, table="dev-dependencies")
        crate(self.root, "b")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("packages/b" in p for p in found), found)

    def test_target_specific_dependencies_count(self):
        write(
            self.root,
            "packages/a/Cargo.toml",
            '[package]\nname = "a"\nversion = "0.1.0"\n'
            '[target.\'cfg(windows)\'.dependencies]\nb = { path = "../b" }\n',
        )
        crate(self.root, "b")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("packages/b" in p for p in found), found)

    def test_a_covered_parent_directory_closes_the_dependency(self):
        crate(self.root, "a", {"b": "../b"})
        crate(self.root, "b")
        self.assertEqual(self.scenario(["demos/android", "packages"]), [])

    def test_prefix_match_respects_path_boundaries(self):
        crate(self.root, "a", {"b": "../ab"})
        crate(self.root, "ab")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("packages/ab" in p for p in found), found)

    def test_file_under_a_crate_resolves_to_that_crate(self):
        crate(self.root, "a", {"b": "../b"})
        crate(self.root, "b")
        write(self.root, "packages/a/src/lib.rs")
        found = self.scenario(["demos/android", "packages/a/src"])
        self.assertTrue(any("packages/b" in p for p in found), found)

    def test_scenario_directory_must_be_covered(self):
        crate(self.root, "a")
        found = self.scenario(["packages/a"])
        self.assertTrue(any("demos/android" in p for p in found), found)

    def test_command_script_outside_covers_is_named(self):
        crate(self.root, "a")
        write(self.root, "scripts/helper.sh")
        found = self.scenario(["demos/android", "packages/a"], interactive="scripts/helper.sh --watch")
        self.assertTrue(any("scripts/helper.sh" in p for p in found), found)

    def test_command_tokens_that_are_not_files_are_ignored(self):
        crate(self.root, "a")
        self.assertEqual(
            self.scenario(["demos/android", "packages/a"], interactive="echo hello --flag"), []
        )

    def test_missing_covered_path_is_reported(self):
        found = self.scenario(["demos/android", "packages/nowhere"])
        self.assertTrue(any("packages/nowhere" in p and "does not exist" in p for p in found), found)

    def test_path_dependency_outside_the_repo_is_reported(self):
        write(
            self.root,
            "packages/a/Cargo.toml",
            '[package]\nname = "a"\nversion = "0.1.0"\n[dependencies]\nx = { path = "../../../x" }\n',
        )
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("outside" in p for p in found), found)


class WorkspaceClosureTest(unittest.TestCase):
    setUp = ClosureTest.setUp
    scenario = ClosureTest.scenario

    def test_patch_path_override_counts(self):
        write(
            self.root,
            "packages/a/Cargo.toml",
            '[package]\nname = "a"\nversion = "0.1.0"\n[patch.crates-io]\nb = { path = "../b" }\n',
        )
        crate(self.root, "b")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("packages/b" in p for p in found), found)

    def test_workspace_inherited_dependency_counts_when_only_the_member_is_covered(self):
        write(
            self.root,
            "packages/Cargo.toml",
            '[workspace]\nmembers = ["a"]\n[workspace.dependencies]\nb = { path = "b" }\n',
        )
        write(
            self.root,
            "packages/a/Cargo.toml",
            '[package]\nname = "a"\nversion = "0.1.0"\n[dependencies]\nb.workspace = true\n',
        )
        crate(self.root, "b")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("packages/b" in p for p in found), found)

    def test_covered_workspace_root_checks_its_members(self):
        write(self.root, "ws/Cargo.toml", '[workspace]\nmembers = ["m"]\n')
        write(
            self.root,
            "ws/m/Cargo.toml",
            '[package]\nname = "m"\nversion = "0.1.0"\n[dependencies]\nb = { path = "../../packages/b" }\n',
        )
        crate(self.root, "b")
        found = self.scenario(["demos/android", "ws"])
        self.assertTrue(any("packages/b" in p for p in found), found)

    def test_malformed_dependency_tables_are_reported_not_raised(self):
        write(self.root, "packages/a/Cargo.toml", '[package]\nname = "a"\ndependencies = 5\n')
        self.assertEqual(self.scenario(["demos/android", "packages/a"]), [])

    def test_unparsable_cargo_toml_is_reported(self):
        write(self.root, "packages/a/Cargo.toml", "[package\n")
        found = self.scenario(["demos/android", "packages/a"])
        self.assertTrue(any("packages/a" in p and "Cargo.toml" in p for p in found), found)


if __name__ == "__main__":
    unittest.main()
