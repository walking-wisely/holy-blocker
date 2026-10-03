import unittest

from tools.plan.mutants_excludes import missing_excludes

CONFIG = 'exclude_re = ["replace \\\\* with . in f", "in g"]\n'


class MissingExcludesTest(unittest.TestCase):
    def test_all_listed(self):
        claim = "| `replace \\* with . in f` | x |\n| `in g` | y |"
        self.assertEqual(missing_excludes(CONFIG, claim), [])

    def test_unlisted_pattern_is_reported(self):
        self.assertEqual(missing_excludes(CONFIG, "| `in g` | y |"), ["replace \\* with . in f"])

    def test_pattern_without_backticks_does_not_count(self):
        self.assertEqual(missing_excludes('exclude_re = ["in g"]', "in g"), ["in g"])

    def test_no_excludes(self):
        self.assertEqual(missing_excludes("", "anything"), [])


if __name__ == "__main__":
    unittest.main()
