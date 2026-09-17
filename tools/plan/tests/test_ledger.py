import unittest

from tools.plan import ledger


class ExtractMarkersTest(unittest.TestCase):
    def test_finds_step_ids_in_order(self):
        text = "a <!-- step: p.one --> b\n<!--step:p.two-->"
        self.assertEqual(ledger.extract_markers(text), ["p.one", "p.two"])

    def test_ignores_unrelated_comments(self):
        self.assertEqual(ledger.extract_markers("<!-- TODO: x -->"), [])


class ValidateTest(unittest.TestCase):
    def _validate(self, steps, markers):
        return ledger.validate(steps, markers)

    def test_clean_manifest_has_no_problems(self):
        steps = [ledger.Step("p.a", "a", "done", "observed on emulator")]
        self.assertEqual(self._validate(steps, ["p.a"]), [])

    def test_pending_needs_no_evidence(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertEqual(self._validate(steps, ["p.a"]), [])

    def test_duplicate_manifest_id(self):
        steps = [ledger.Step("p.a", "a", "pending", ""), ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("duplicate manifest id" in p for p in self._validate(steps, ["p.a"])))

    def test_invalid_status(self):
        steps = [ledger.Step("p.a", "a", "finished", "")]
        self.assertTrue(any("invalid status" in p for p in self._validate(steps, ["p.a"])))

    def test_done_without_evidence(self):
        steps = [ledger.Step("p.a", "a", "done", "  ")]
        self.assertTrue(any("done without evidence" in p for p in self._validate(steps, ["p.a"])))

    def test_manifest_entry_without_marker(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("no <!-- step" in p for p in self._validate(steps, [])))

    def test_marker_without_manifest_entry(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("no manifest entry" in p for p in self._validate(steps, ["p.a", "p.ghost"])))

    def test_duplicate_marker(self):
        steps = [ledger.Step("p.a", "a", "pending", "")]
        self.assertTrue(any("duplicate marker" in p for p in self._validate(steps, ["p.a", "p.a"])))


class RenderTest(unittest.TestCase):
    def test_renders_status_table(self):
        steps = [ledger.Step("p.a", "a", "done", "commit abc")]
        rendered = ledger.render(steps)
        self.assertIn("| Step | Status | Evidence |", rendered)
        self.assertIn("| `p.a` | done | commit abc |", rendered)


if __name__ == "__main__":
    unittest.main()