import contextlib
import io
import tempfile
import unittest
from pathlib import Path

from tools.plan import ledger


def write(root: Path, rel: str, text: str) -> Path:
    path = Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def step_file(step_id: str, status: str = "pending", extra: str = "") -> str:
    return f'id = "{step_id}"\ntitle = "t"\nstatus = "{status}"\nevidence = ""\n{extra}'


class LoadPerStepTest(unittest.TestCase):
    def test_reads_one_step_per_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a", "done", 'kind = "bug"\n'))
            write(tmp, "steps/p.b.toml", step_file("p.b"))
            steps = {s.id: s for s in ledger.load_manifest(Path(tmp))}
            self.assertEqual(set(steps), {"p.a", "p.b"})
            self.assertEqual(steps["p.a"].status, "done")
            self.assertEqual(steps["p.a"].kind, "bug")

    def test_markers_do_not_affect_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            for step_id in ("p.a", "p.b", "p.c"):
                write(tmp, f"steps/{step_id}.toml", step_file(step_id))
            write(tmp, "plan.md", "<!-- step: p.c -->\n<!-- step: p.a -->\n<!-- step: p.b -->")
            self.assertEqual(
                [s.id for s in ledger.load_manifest(Path(tmp))], ["p.a", "p.b", "p.c"]
            )

    def test_depends_on_orders_before_id(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a", extra='depends_on = ["p.b"]\n'))
            write(tmp, "steps/p.b.toml", step_file("p.b"))
            step, _ = ledger.next_plan(ledger.load_manifest(Path(tmp)))
            self.assertEqual(step.id, "p.b")

    def test_group_key_is_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a", extra='group = "g1"\n'))
            self.assertEqual(ledger.load_manifest(Path(tmp))[0].group, "g1")

    def test_paths_come_from_package_toml(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "package.toml", 'paths = ["packages/a", "apps/b"]\n')
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            self.assertEqual(ledger.load_paths(Path(tmp)), ("packages/a", "apps/b"))

    def test_no_package_toml_means_no_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            self.assertEqual(ledger.load_paths(Path(tmp)), ())

    def test_legacy_manifest_is_still_readable(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", 'paths = ["x"]\n[[step]]\nid = "p.a"\nstatus = "pending"\n')
            self.assertEqual([s.id for s in ledger.load_manifest(Path(tmp))], ["p.a"])
            self.assertEqual(ledger.load_paths(Path(tmp)), ("x",))


class LayoutProblemsTest(unittest.TestCase):
    def test_clean_layout_has_no_problems(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            self.assertEqual(ledger.layout_problems(Path(tmp)), [])

    def test_stem_must_equal_id(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.other.toml", step_file("p.a"))
            problems = ledger.layout_problems(Path(tmp))
            self.assertTrue(any("p.other.toml" in p and "p.a" in p for p in problems))

    def test_legacy_manifest_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", '[[step]]\nid = "p.a"\nstatus = "pending"\n')
            self.assertTrue(any("legacy" in p for p in ledger.layout_problems(Path(tmp))))

    def test_legacy_beside_new_layout_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", "")
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            self.assertTrue(any("legacy" in p for p in ledger.layout_problems(Path(tmp))))

    def test_unknown_key_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a", extra='notes = "x"\n'))
            self.assertTrue(any("unknown key" in p for p in ledger.layout_problems(Path(tmp))))

    def test_step_table_in_a_step_file_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", '[[step]]\nid = "p.a"\nstatus = "pending"\n')
            self.assertTrue(any("unknown key" in p for p in ledger.layout_problems(Path(tmp))))

    def test_invalid_toml_is_a_problem_not_a_crash(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", "not toml {{{")
            self.assertTrue(any("p.a.toml" in p for p in ledger.layout_problems(Path(tmp))))

    def test_missing_id_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", 'status = "pending"\n')
            self.assertTrue(any("no id" in p for p in ledger.layout_problems(Path(tmp))))

    def test_package_toml_unknown_key_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "package.toml", 'pahts = ["x"]\n')
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            self.assertTrue(any("package.toml" in p and "pahts" in p for p in ledger.layout_problems(Path(tmp))))

    def test_stray_entries_in_steps_are_problems(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            write(tmp, "steps/notes.md", "x")
            write(tmp, "steps/p.a.toml.bak", "x")
            write(tmp, "steps/sub/p.b.toml", step_file("p.b"))
            problems = " ".join(ledger.layout_problems(Path(tmp)))
            for name in ("notes.md", "p.a.toml.bak", "sub"):
                self.assertIn(name, problems)

    def test_symlinked_step_file_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            real = write(tmp, "elsewhere.toml", step_file("p.a"))
            (Path(tmp) / "steps").mkdir()
            (Path(tmp) / "steps" / "p.a.toml").symlink_to(real)
            self.assertTrue(any("symlink" in p for p in ledger.layout_problems(Path(tmp))))

    def test_string_where_a_list_belongs_is_a_problem(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "package.toml", 'paths = "x"\n')
            write(tmp, "steps/p.a.toml", step_file("p.a", extra='depends_on = "p.b"\n'))
            problems = ledger.layout_problems(Path(tmp))
            self.assertTrue(any("paths" in p and "list" in p for p in problems))
            self.assertTrue(any("depends_on" in p and "list" in p for p in problems))

    def test_validate_command_reports_layout_problems(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps/p.other.toml", step_file("p.a"))
            write(tmp, "plan.md", "<!-- step: p.a -->")
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = ledger.main(["validate", tmp])
            self.assertEqual(code, 1)
            self.assertIn("p.other.toml", stderr.getvalue())


class DiscoverTest(unittest.TestCase):
    def test_finds_components_and_engineering_in_either_layout(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "docs/engineering/steps/e.a.toml", step_file("e.a"))
            write(tmp, "docs/components/new/steps/n.a.toml", step_file("n.a"))
            write(tmp, "docs/components/old/steps.toml", "")
            write(tmp, "docs/components/none/plan.md", "")
            found = {p.name for p in ledger.discover(Path(tmp))}
            self.assertEqual(found, {"engineering", "new", "old"})


class MigrateTest(unittest.TestCase):
    LEGACY = (
        "# header line one\n#\n# header line two\n\n"
        'paths = [\n  "packages/a",\n  "apps/b",\n]\n\n'
        '[[step]]\nid = "p.a"\ntitle = "first \\"quoted\\" — dash"\nstatus = "done"\n'
        'evidence = "PR #1"\nacceptance = "code"\ndepends_on = ["p.b"]\nverify = "x && y"\n\n'
        '[[step]]\nid = "p.b"\ntitle = "second"\nstatus = "pending"\nevidence = ""\n'
        'kind = "bug"\nregressed_step = "p.a"\ndifficulty = "hard"\n'
    )

    def _migrate(self, tmp: str):
        write(tmp, "steps.toml", self.LEGACY)
        write(tmp, "plan.md", "<!-- step: p.a -->\n<!-- step: p.b -->")
        before = ledger.load_manifest(Path(tmp))
        ledger.migrate(Path(tmp))
        return before

    def test_round_trips_every_field(self):
        with tempfile.TemporaryDirectory() as tmp:
            before = self._migrate(tmp)
            self.assertEqual(ledger.load_manifest(Path(tmp)), before)

    def test_round_trips_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            self._migrate(tmp)
            self.assertEqual(ledger.load_paths(Path(tmp)), ("packages/a", "apps/b"))

    def test_removes_the_legacy_file_and_names_files_by_id(self):
        with tempfile.TemporaryDirectory() as tmp:
            self._migrate(tmp)
            self.assertFalse(Path(tmp, "steps.toml").exists())
            self.assertEqual(
                sorted(p.name for p in Path(tmp, "steps").iterdir()), ["p.a.toml", "p.b.toml"]
            )

    def test_keeps_the_header_comment_in_package_toml(self):
        with tempfile.TemporaryDirectory() as tmp:
            self._migrate(tmp)
            text = Path(tmp, "package.toml").read_text(encoding="utf-8")
            self.assertTrue(text.startswith("# header line one\n#\n# header line two\n"))

    def test_migrated_layout_is_clean(self):
        with tempfile.TemporaryDirectory() as tmp:
            self._migrate(tmp)
            self.assertEqual(ledger.layout_problems(Path(tmp)), [])

    def test_refuses_to_overwrite_existing_step_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", self.LEGACY)
            write(tmp, "steps/p.a.toml", step_file("p.a"))
            with self.assertRaises(FileExistsError):
                ledger.migrate(Path(tmp))
            self.assertTrue(Path(tmp, "steps.toml").exists())

    def test_migrate_without_paths_writes_no_paths_key(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", '[[step]]\nid = "p.a"\nstatus = "pending"\n')
            ledger.migrate(Path(tmp))
            self.assertEqual(ledger.load_paths(Path(tmp)), ())

    def test_rejects_an_id_that_is_not_a_safe_file_name(self):
        for bad in ("../escaped", "/abs/x", "a b", "a/b", ".", ".."):
            with tempfile.TemporaryDirectory() as tmp:
                write(tmp, "steps.toml", f'[[step]]\nid = "{bad}"\nstatus = "pending"\n')
                with self.assertRaises(ValueError, msg=bad):
                    ledger.migrate(Path(tmp))
                self.assertTrue(Path(tmp, "steps.toml").exists(), bad)
                self.assertFalse(Path(tmp, "steps").exists(), bad)

    def test_migrate_command_reports_an_unsafe_id_without_a_traceback(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", '[[step]]\nid = "../x"\nstatus = "pending"\n')
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                code = ledger.main(["migrate", tmp])
            self.assertEqual(code, 1)
            self.assertIn("cannot migrate", stderr.getvalue())

    def _assert_refused(self, text, exc=ValueError):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", text)
            with self.assertRaises(exc):
                ledger.migrate(Path(tmp))
            self.assertTrue(Path(tmp, "steps.toml").exists())
            self.assertFalse(Path(tmp, "steps").exists())
            self.assertFalse(Path(tmp, "package.toml").exists())

    def test_refuses_duplicate_ids(self):
        self._assert_refused(
            '[[step]]\nid = "p.a"\nstatus = "pending"\n[[step]]\nid = "p.a"\nstatus = "done"\n'
        )

    def test_refuses_ids_that_differ_only_in_case(self):
        self._assert_refused(
            '[[step]]\nid = "p.A"\nstatus = "pending"\n[[step]]\nid = "p.a"\nstatus = "done"\n'
        )

    def test_refuses_a_value_that_would_not_round_trip(self):
        self._assert_refused('[[step]]\nid = "p.a"\nstatus = "pending"\nevidence = 2020-01-01\n')

    def test_delete_character_in_a_title_round_trips(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", '[[step]]\nid = "p.a"\ntitle = "x\\u007fy"\nstatus = "pending"\n')
            before = ledger.load_manifest(Path(tmp))
            ledger.migrate(Path(tmp))
            self.assertEqual(ledger.load_manifest(Path(tmp)), before)

    def test_migrate_command_converts_the_given_directories(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(tmp, "steps.toml", self.LEGACY)
            with contextlib.redirect_stdout(io.StringIO()):
                code = ledger.main(["migrate", tmp])
            self.assertEqual(code, 0)
            self.assertTrue(Path(tmp, "steps", "p.a.toml").exists())


if __name__ == "__main__":
    unittest.main()
