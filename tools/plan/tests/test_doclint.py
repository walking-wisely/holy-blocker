import tempfile
import unittest
from pathlib import Path

from tools.plan import doclint


class SlugTest(unittest.TestCase):
    def test_lowercases_and_hyphenates(self):
        self.assertEqual(doclint.github_slug("Layer 1 — network path"), "layer-1--network-path")

    def test_drops_punctuation_but_keeps_hyphens_and_underscores(self):
        self.assertEqual(doclint.github_slug("Why `net-shield`? (a_b)"), "why-net-shield-a_b")

    def test_strips_markdown_emphasis_and_links(self):
        self.assertEqual(doclint.github_slug("The [FFI](x.md) **dependency**"), "the-ffi-dependency")


class AnchorTest(unittest.TestCase):
    def test_collects_headings_and_numbers_duplicates(self):
        text = "# A\n## B\n## B\n"
        self.assertEqual(doclint.extract_anchors(text), {"a", "b", "b-1"})

    def test_ignores_headings_inside_code_fences(self):
        text = "# Real\n```\n# not a heading\n```\n"
        self.assertEqual(doclint.extract_anchors(text), {"real"})

    def test_step_markers_do_not_leak_into_the_slug(self):
        text = "### 1. Foo <!-- step: x.y -->\n"
        self.assertEqual(doclint.extract_anchors(text), {"1-foo"})


class LinkTest(unittest.TestCase):
    def test_extracts_relative_links_with_line_numbers(self):
        text = "x\nsee [a](a.md) and [b](b.md#frag)\n"
        self.assertEqual(doclint.extract_links(text), [(2, "a.md"), (2, "b.md#frag")])

    def test_skips_external_and_mailto_and_absolute(self):
        text = "[a](https://x.y) [b](mailto:a@b.c) [c](/abs.md)\n"
        self.assertEqual(doclint.extract_links(text), [])

    def test_skips_links_in_code_fences_and_inline_code(self):
        text = "```\n[a](a.md)\n```\n`[b](b.md)`\n"
        self.assertEqual(doclint.extract_links(text), [])

    def test_ignores_angle_bracket_template_placeholders(self):
        text = "[x](docs/<package>/plan.md)\n"
        self.assertEqual(doclint.extract_links(text), [])


class FenceTest(unittest.TestCase):
    def test_shorter_inner_fence_does_not_close_a_longer_block(self):
        text = "````\n```\n[a](a.md)\n```\n````\n[b](b.md)\n"
        self.assertEqual(doclint.extract_links(text), [(6, "b.md")])

    def test_other_marker_character_does_not_close_a_block(self):
        text = "```\n~~~\n[a](a.md)\n```\n[b](b.md)\n"
        self.assertEqual(doclint.extract_links(text), [(5, "b.md")])

    def test_closing_fence_with_trailing_text_does_not_close(self):
        text = "```\n``` not a close\n[a](a.md)\n```\n[b](b.md)\n"
        self.assertEqual(doclint.extract_links(text), [(5, "b.md")])

    def test_longer_closing_fence_closes(self):
        text = "```\n[a](a.md)\n`````\n[b](b.md)\n"
        self.assertEqual(doclint.extract_links(text), [(4, "b.md")])


class ReferenceLinkTest(unittest.TestCase):
    def test_reference_definitions_are_extracted_as_links(self):
        text = "see [a][ref] and [b]\n\n[ref]: a.md\n[b]: b.md#frag \"title\"\n"
        self.assertEqual(
            doclint.extract_links(text), [(3, "a.md"), (4, "b.md#frag")]
        )

    def test_external_reference_definition_is_skipped(self):
        self.assertEqual(doclint.extract_links("[x]: https://x.y\n"), [])

    def test_definition_inside_a_fence_is_skipped(self):
        self.assertEqual(doclint.extract_links("```\n[x]: a.md\n```\n"), [])


class BrokenLinkTest(unittest.TestCase):
    def test_missing_reference_destination_is_reported(self):
        page = self.write("a.md", "[x][r]\n\n[r]: gone.md\n")
        self.assertEqual(len(doclint.broken_links(page)), 1)

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def write(self, rel: str, text: str) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def test_flags_missing_file(self):
        page = self.write("a.md", "[x](missing.md)\n")
        self.assertEqual(len(doclint.broken_links(page)), 1)

    def test_accepts_existing_file_and_directory(self):
        self.write("b.md", "# B\n")
        self.write("d/x.md", "# X\n")
        page = self.write("a.md", "[x](b.md) [d](d/)\n")
        self.assertEqual(doclint.broken_links(page), [])

    def test_flags_missing_anchor_in_other_file(self):
        self.write("b.md", "# Real\n")
        page = self.write("a.md", "[x](b.md#nope)\n")
        problems = doclint.broken_links(page)
        self.assertEqual(len(problems), 1)
        self.assertIn("#nope", problems[0])

    def test_accepts_present_anchor_and_same_file_anchor(self):
        self.write("b.md", "## Some Heading\n")
        page = self.write("a.md", "# Top\n[x](b.md#some-heading) [y](#top)\n")
        self.assertEqual(doclint.broken_links(page), [])

    def test_anchor_into_non_markdown_is_not_checked(self):
        self.write("data.toml", "x = 1\n")
        page = self.write("a.md", "[x](data.toml#L3)\n")
        self.assertEqual(doclint.broken_links(page), [])


class ComponentIndexTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.components = Path(self._tmp.name) / "components"
        self.components.mkdir()
        self.addCleanup(self._tmp.cleanup)

    def component(self, name: str, readme: bool = True) -> None:
        (self.components / name).mkdir()
        if readme:
            (self.components / name / "README.md").write_text("# x\n", encoding="utf-8")

    def index(self, text: str) -> None:
        (self.components / "README.md").write_text(text, encoding="utf-8")

    def test_clean_when_every_component_has_readme_and_index_row(self):
        self.component("a")
        self.index("[a](a/README.md)\n")
        self.assertEqual(doclint.component_problems(self.components), [])

    def test_flags_component_without_readme(self):
        self.component("a", readme=False)
        self.index("[a](a/README.md)\n")
        problems = doclint.component_problems(self.components)
        self.assertTrue(any("a" in p and "README.md" in p for p in problems))

    def test_flags_component_missing_from_index(self):
        self.component("a")
        self.component("b")
        self.index("[a](a/README.md)\n")
        problems = doclint.component_problems(self.components)
        self.assertEqual(len(problems), 1)
        self.assertIn("b", problems[0])

    def test_flags_missing_index(self):
        self.component("a")
        self.assertEqual(len(doclint.component_problems(self.components)), 1)


if __name__ == "__main__":
    unittest.main()
