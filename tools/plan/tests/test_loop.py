import contextlib
import io
import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.plan import ledger, loop

MANIFESTS = [
    loop.Governed(
        package="docs/components/net-shield",
        paths=("packages/net-shield", "packages/net-shield-ffi"),
        step_ids=("net-shield.dns", "net-shield.sni"),
    ),
    loop.Governed(
        package="docs/components/desktop",
        paths=("apps/desktop",),
        step_ids=("desktop.ipc",),
    ),
]

COMPLETE_BODY = """Summary.

Step: `net-shield.dns`

## Assumption audit
All claims settle now.

## Adversarial review
N/A — docs-only wording change.
"""


def check(body, changed):
    return loop.check(body, changed, MANIFESTS)


class OwningTest(unittest.TestCase):
    def test_file_under_a_governed_path_matches(self):
        self.assertEqual(
            [m.package for m in loop.governing(["packages/net-shield/src/lib.rs"], MANIFESTS)],
            ["docs/components/net-shield"],
        )

    def test_sibling_directory_with_shared_prefix_does_not_match(self):
        self.assertEqual(loop.governing(["packages/net-shield-extra/a.rs"], MANIFESTS), [])

    def test_exact_path_matches(self):
        self.assertEqual(len(loop.governing(["apps/desktop"], MANIFESTS)), 1)

    def test_trailing_slash_in_manifest_path_is_tolerated(self):
        manifests = [loop.Governed("p", ("apps/desktop/",), ("desktop.ipc",))]
        self.assertEqual(len(loop.governing(["apps/desktop/a.ts"], manifests)), 1)


class CheckTest(unittest.TestCase):
    def test_no_governed_file_passes_with_empty_body(self):
        self.assertEqual(check("", ["docs/README.md", "CLAUDE.md"]), [])

    def test_governed_file_with_empty_body_fails_on_both_counts(self):
        problems = check("", ["packages/net-shield/src/lib.rs"])
        self.assertTrue(any("step id" in p for p in problems))
        self.assertTrue(any("Assumption audit" in p for p in problems))
        self.assertTrue(any("Adversarial review" in p for p in problems))

    def test_unknown_step_id_fails(self):
        body = COMPLETE_BODY.replace("net-shield.dns", "net-shield.bogus")
        problems = check(body, ["packages/net-shield/src/lib.rs"])
        self.assertTrue(any("step id" in p for p in problems))

    def test_id_that_is_only_a_prefix_of_a_real_one_fails(self):
        body = COMPLETE_BODY.replace("net-shield.dns", "net-shield.dns-extra")
        self.assertTrue(any("step id" in p for p in check(body, ["apps/desktop/a.ts"])))

    def test_missing_audit_heading_fails(self):
        body = COMPLETE_BODY.replace("## Assumption audit\nAll claims settle now.\n", "")
        problems = check(body, ["packages/net-shield/src/lib.rs"])
        self.assertEqual(len(problems), 1)
        self.assertIn("Assumption audit", problems[0])

    def test_missing_review_heading_fails(self):
        body = COMPLETE_BODY.replace(
            "## Adversarial review\nN/A — docs-only wording change.\n", ""
        )
        problems = check(body, ["packages/net-shield/src/lib.rs"])
        self.assertEqual(len(problems), 1)
        self.assertIn("Adversarial review", problems[0])

    def test_complete_body_passes(self):
        self.assertEqual(check(COMPLETE_BODY, ["packages/net-shield/src/lib.rs"]), [])

    def test_one_line_not_applicable_section_passes(self):
        body = "`desktop.ipc`\n\n## Assumption audit\nN/A — rename.\n\n## Adversarial review\nN/A — rename.\n"
        self.assertEqual(check(body, ["apps/desktop/src/a.ts"]), [])

    def test_heading_with_no_content_fails(self):
        body = "`desktop.ipc`\n\n## Assumption audit\n\n## Adversarial review\nN/A — rename.\n"
        problems = check(body, ["apps/desktop/src/a.ts"])
        self.assertEqual(len(problems), 1)
        self.assertIn("empty", problems[0])

    def test_heading_followed_only_by_the_next_heading_is_empty(self):
        body = "`desktop.ipc`\n## Assumption audit\n## Adversarial review\nN/A — x.\n"
        self.assertTrue(any("empty" in p for p in check(body, ["apps/desktop/a.ts"])))

    def test_commented_out_heading_does_not_count(self):
        body = (
            "`desktop.ipc`\n<!-- ## Assumption audit\nstuff -->\n"
            "## Adversarial review\nN/A — x.\n"
        )
        problems = check(body, ["apps/desktop/a.ts"])
        self.assertTrue(any("Assumption audit" in p for p in problems))

    def test_heading_inside_code_fence_does_not_count(self):
        body = (
            "`desktop.ipc`\n```\n## Assumption audit\nx\n```\n"
            "## Adversarial review\nN/A — x.\n"
        )
        self.assertTrue(any("Assumption audit" in p for p in check(body, ["apps/desktop/a.ts"])))

    def test_nested_heading_level_does_not_count(self):
        body = "`desktop.ipc`\n### Assumption audit\nx\n## Adversarial review\nN/A — x.\n"
        self.assertTrue(any("Assumption audit" in p for p in check(body, ["apps/desktop/a.ts"])))

    def test_crlf_body_is_accepted(self):
        body = COMPLETE_BODY.replace("\n", "\r\n")
        self.assertEqual(check(body, ["packages/net-shield/src/lib.rs"]), [])

    def test_step_id_may_belong_to_a_different_manifest(self):
        body = COMPLETE_BODY.replace("net-shield.dns", "desktop.ipc")
        self.assertEqual(check(body, ["packages/net-shield/src/lib.rs"]), [])


class RouteTest(unittest.TestCase):
    GOVERNED = [
        loop.Governed(
            package="docs/components/net-shield",
            paths=("packages/net-shield",),
            step_ids=("net-shield.dns", "net-shield.sni"),
            pending_step_ids=("net-shield.sni",),
        ),
        loop.Governed(
            package="docs/components/desktop",
            paths=("apps/desktop",),
            step_ids=("desktop.ipc",),
        ),
    ]

    def test_governed_path_lists_manifest_files_and_pending_steps(self):
        got = loop.route(["packages/net-shield/src/a.rs", "README.md"], self.GOVERNED)
        self.assertEqual([m.package for m, _ in got.governing], ["docs/components/net-shield"])
        self.assertEqual(got.governing[0][1], ("packages/net-shield/src/a.rs",))
        self.assertEqual(got.unclaimed, ())

    def test_unclaimed_code_root_is_reported(self):
        got = loop.route(["packages/new-crate/lib.rs"], self.GOVERNED)
        self.assertEqual(got.governing, ())
        self.assertEqual(got.unclaimed, ("packages/new-crate/lib.rs",))

    def test_render_names_pending_steps_and_requires_a_step(self):
        text = loop.render_route(loop.route(["packages/net-shield/src/a.rs"], self.GOVERNED))
        self.assertIn("governed docs/components/net-shield", text)
        self.assertIn("pending steps: net-shield.sni", text)
        self.assertIn("verdict: step-required", text)

    def test_render_with_no_pending_steps_says_to_add_one(self):
        text = loop.render_route(loop.route(["apps/desktop/main.ts"], self.GOVERNED))
        self.assertIn("pending steps: none", text)

    def test_render_for_ungoverned_paths_is_loop_lite(self):
        text = loop.render_route(loop.route(["docs/README.md"], self.GOVERNED))
        self.assertIn("verdict: no-governing-manifest", text)

    def test_render_for_unclaimed_root_requires_a_claim(self):
        text = loop.render_route(loop.route(["packages/new-crate/lib.rs"], self.GOVERNED))
        self.assertIn("unclaimed packages/new-crate/lib.rs", text)
        self.assertIn("verdict: claim-required", text)

    def test_no_paths_is_nothing_to_route(self):
        self.assertIn("verdict: nothing-to-route", loop.render_route(loop.route([], self.GOVERNED)))


class UnclaimedRootTest(unittest.TestCase):
    def test_new_crate_no_manifest_claims_fails_even_with_a_complete_body(self):
        problems = check(COMPLETE_BODY, ["packages/new-crate/src/lib.rs"])
        self.assertEqual(len(problems), 1)
        self.assertIn("packages/new-crate/src/lib.rs", problems[0])

    def test_each_code_root_is_checked(self):
        for path in ("apps/new/a.ts", "native-modules/new/a.cpp", "machine-learning/a.py"):
            with self.subTest(path=path):
                self.assertEqual(len(check(COMPLETE_BODY, [path])), 1)

    def test_claimed_crate_is_not_reported_as_unclaimed(self):
        self.assertEqual(check(COMPLETE_BODY, ["packages/net-shield/src/lib.rs"]), [])

    def test_sibling_with_shared_prefix_is_unclaimed(self):
        self.assertEqual(len(check(COMPLETE_BODY, ["packages/net-shield-extra/a.rs"])), 1)

    def test_docs_and_root_files_are_not_code_roots(self):
        self.assertEqual(check("", ["docs/README.md", "README.md", "LICENSE"]), [])

    def test_unclaimed_file_next_to_a_claimed_one_is_still_reported(self):
        problems = check(COMPLETE_BODY, ["packages/net-shield/a.rs", "packages/new-crate/b.rs"])
        self.assertEqual(len(problems), 1)
        self.assertIn("packages/new-crate/b.rs", problems[0])


class RepoClaimsTest(unittest.TestCase):
    def setUp(self):
        self.manifests = loop.load_governed(Path(__file__).resolve().parents[3])

    def test_process_surfaces_are_governed(self):
        for path in (
            ".github/workflows/ci.yml",
            ".github/dependabot.yml",
            "CLAUDE.md",
            "AGENTS.md",
            "deny.toml",
            ".gitleaks.toml",
        ):
            with self.subTest(path=path):
                self.assertTrue(loop.governing([path], self.manifests))

    def test_no_tracked_code_file_is_unclaimed(self):
        root = Path(__file__).resolve().parents[3]
        tracked = subprocess.run(
            ["git", "ls-files", *loop.CODE_ROOTS], cwd=root, check=True, capture_output=True, text=True
        ).stdout.split()
        self.assertEqual(loop.unclaimed(tracked, self.manifests), [])


class GitDiffTest(unittest.TestCase):
    GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name) / "repo"
        self.root.mkdir()
        self.git("init", "-q")
        self.manifest("a", "a.one", "packages/a", "policy.toml")
        self.write("packages/a/x.rs", "fn main() {}\n")
        self.write("policy.toml", "v = 1\n")
        self.git("add", "-A")
        self.git("commit", "-qm", "base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.run(
            [*self.GIT, *args], cwd=self.root, check=True, capture_output=True, text=True
        ).stdout

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def manifest(self, name, step_id, *claimed):
        quoted = ", ".join(f'"{c}"' for c in claimed)
        paths = f"paths = [{quoted}]\n" if claimed else ""
        self.write(
            f"docs/components/{name}/steps.toml",
            f'{paths}[[step]]\nid = "{step_id}"\nstatus = "pending"\n',
        )
        self.write(f"docs/components/{name}/plan.md", f"<!-- step: {step_id} -->")

    def commit(self):
        self.git("add", "-A")
        self.git("commit", "-qm", "head")

    def check(self, body=""):
        body_file = Path(self._tmp.name) / "body.md"
        body_file.write_text(body, encoding="utf-8")
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = loop.main(
                ["check", "--root", str(self.root), "--body-file", str(body_file),
                 f"--base={self.base}", "--head", "HEAD"]
            )
        return code, err.getvalue()

    def test_empty_diff_fails_instead_of_passing_vacuously(self):
        code, err = self.check()
        self.assertEqual(code, 1)
        self.assertIn("no changed files", err)

    def route(self, *paths):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = loop.main(["route", "--root", str(self.root), *paths])
        return code, out.getvalue()

    def test_route_reads_the_real_manifests(self):
        code, out = self.route("packages/a/x.rs")
        self.assertEqual(code, 0)
        self.assertIn("pending steps: a.one", out)
        self.assertIn("verdict: step-required", out)

    def test_route_from_a_diff(self):
        self.write("packages/a/y.rs", "fn y() {}\n")
        self.commit()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = loop.main(["route", "--root", str(self.root), f"--base={self.base}", "--head", "HEAD"])
        self.assertEqual(code, 0)
        self.assertIn("packages/a/y.rs", out.getvalue())

    def test_rename_out_of_a_governed_path_is_still_governed(self):
        self.git("mv", "policy.toml", "notes.md")
        self.commit()
        code, err = self.check()
        self.assertEqual(code, 1)
        self.assertIn("Assumption audit", err)

    def test_dropping_a_claim_in_the_same_pr_does_not_release_the_path(self):
        self.manifest("a", "a.one", "packages/a")
        self.write("policy.toml", "v = 2\n")
        self.commit()
        code, _ = self.check()
        self.assertEqual(code, 1)

    def test_deleting_a_manifest_does_not_release_its_paths(self):
        self.git("rm", "-rq", "docs/components/a")
        self.write("policy.toml", "v = 2\n")
        self.commit()
        code, _ = self.check()
        self.assertEqual(code, 1)

    def test_new_crate_may_claim_itself_in_the_same_pr(self):
        self.manifest("b", "b.one", "packages/b")
        self.write("packages/b/y.rs", "fn y() {}\n")
        self.commit()
        body = "Step: `b.one`\n\n## Assumption audit\nN/A — new crate.\n\n## Adversarial review\nN/A — new crate.\n"
        code, err = self.check(body)
        self.assertEqual((code, err), (0, ""))

    def test_new_crate_without_a_claim_fails(self):
        self.write("packages/b/y.rs", "fn y() {}\n")
        self.commit()
        code, err = self.check()
        self.assertEqual(code, 1)
        self.assertIn("packages/b/y.rs", err)

    def test_non_ascii_path_under_a_claimed_root_is_still_governed(self):
        self.write("packages/a/é.rs", "fn e() {}\n")
        self.commit()
        code, err = self.check()
        self.assertEqual(code, 1)
        self.assertIn("Assumption audit", err)

    def test_non_ascii_path_in_an_unclaimed_crate_fails(self):
        self.write("packages/é/z.rs", "fn z() {}\n")
        self.commit()
        code, err = self.check()
        self.assertEqual(code, 1)
        self.assertIn("é", err)

    def test_option_shaped_base_is_rejected_without_side_effects(self):
        target = Path(self._tmp.name) / "pwn"
        self.base = f"--output={target}"
        code, _ = self.check()
        self.assertEqual(code, 1)
        self.assertFalse(target.exists())

    def test_base_without_manifests_fails_closed(self):
        original = self.base
        self.git("checkout", "-q", "--orphan", "fresh")
        self.git("rm", "-rfq", ".")
        self.write("README.md", "x\n")
        self.git("add", "-A")
        self.git("commit", "-qm", "root")
        self.base = self.git("rev-parse", "HEAD").strip()
        self.git("checkout", original, "--", ".")
        self.write("policy.toml", "v = 2\n")
        self.commit()
        code, err = self.check()
        self.assertEqual(code, 1)
        self.assertIn("base manifests", err)


class OpenTest(unittest.TestCase):
    STEP = ledger.Step(
        id="net-shield.dns",
        title="dns path",
        status="pending",
        evidence="",
        acceptance="code",
        verify="cargo test",
    )

    def test_rendered_body_names_the_step_and_both_sections(self):
        body = loop.render_body(self.STEP)
        self.assertIn("`net-shield.dns`", body)
        self.assertIn("cargo test", body)
        self.assertIn("## Assumption audit", body)
        self.assertIn("## Adversarial review", body)

    def test_unfilled_template_fails_the_check(self):
        body = loop.render_body(self.STEP)
        problems = check(body, ["packages/net-shield/src/lib.rs"])
        self.assertTrue(problems)

    def test_filled_template_passes_the_check(self):
        body = loop.render_body(self.STEP)
        body = body.replace("## Assumption audit\n", "## Assumption audit\nN/A — x.\n")
        body = body.replace("## Adversarial review\n", "## Adversarial review\nN/A — x.\n")
        self.assertEqual(check(body, ["packages/net-shield/src/lib.rs"]), [])


class LoadTest(unittest.TestCase):
    def _package(self, root, name, toml, plan):
        d = Path(root) / "docs" / "components" / name
        d.mkdir(parents=True)
        (d / "steps.toml").write_text(toml, encoding="utf-8")
        (d / "plan.md").write_text(plan, encoding="utf-8")

    def test_reads_paths_and_ids_from_every_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            self._package(
                tmp,
                "a",
                'paths = ["packages/a"]\n[[step]]\nid = "a.one"\nstatus = "pending"\n',
                "<!-- step: a.one -->",
            )
            self._package(
                tmp,
                "b",
                '[[step]]\nid = "b.one"\nstatus = "pending"\n',
                "<!-- step: b.one -->",
            )
            found = {g.package.rsplit("/", 1)[-1]: g for g in loop.load_governed(Path(tmp))}
            self.assertEqual(found["a"].paths, ("packages/a",))
            self.assertEqual(found["a"].step_ids, ("a.one",))
            self.assertEqual(found["b"].paths, ())

    def test_repo_manifests_load(self):
        root = Path(__file__).resolve().parents[3]
        governed = loop.load_governed(root)
        self.assertGreaterEqual(len(governed), 18)


class CliTest(unittest.TestCase):
    def _run(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = loop.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_check_exits_zero_when_nothing_is_governed(self):
        root = Path(__file__).resolve().parents[3]
        with tempfile.TemporaryDirectory() as tmp:
            body = Path(tmp, "body.md")
            body.write_text("", encoding="utf-8")
            files = Path(tmp, "files.txt")
            files.write_text("README.md\n", encoding="utf-8")
            code, _, _ = self._run(
                ["check", "--root", str(root), "--body-file", str(body), "--changed-files", str(files)]
            )
        self.assertEqual(code, 0)

    def test_check_exits_one_and_names_problems_for_governed_change(self):
        root = Path(__file__).resolve().parents[3]
        with tempfile.TemporaryDirectory() as tmp:
            body = Path(tmp, "body.md")
            body.write_text("", encoding="utf-8")
            files = Path(tmp, "files.txt")
            files.write_text("packages/net-shield/src/lib.rs\n", encoding="utf-8")
            code, _, err = self._run(
                ["check", "--root", str(root), "--body-file", str(body), "--changed-files", str(files)]
            )
        self.assertEqual(code, 1)
        self.assertIn("Assumption audit", err)

    def test_check_with_unreadable_body_fails_closed(self):
        root = Path(__file__).resolve().parents[3]
        with tempfile.TemporaryDirectory() as tmp:
            files = Path(tmp, "files.txt")
            files.write_text("README.md\n", encoding="utf-8")
            code, _, err = self._run(
                [
                    "check",
                    "--root",
                    str(root),
                    "--body-file",
                    str(Path(tmp, "missing.md")),
                    "--changed-files",
                    str(files),
                ]
            )
        self.assertEqual(code, 1)
        self.assertIn("body", err)

    def test_open_prints_a_body_for_a_known_step(self):
        root = Path(__file__).resolve().parents[3]
        code, out, _ = self._run(["open", "engineering.loop-enforcement", "--root", str(root)])
        self.assertEqual(code, 0)
        self.assertIn("## Assumption audit", out)

    def test_open_rejects_an_unknown_step(self):
        root = Path(__file__).resolve().parents[3]
        code, _, err = self._run(["open", "nope.nope", "--root", str(root)])
        self.assertEqual(code, 1)
        self.assertIn("nope.nope", err)


if __name__ == "__main__":
    unittest.main()
