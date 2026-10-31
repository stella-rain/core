"""Tests for the auto-merge.yml caller. Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflow is read as lines. The caller can also be started by
hand for one PR (workflow_dispatch, input number); the shared workflow does the work.
"""

import os
import re
import unittest

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")


def read(*parts):
    with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
        return f.read()


class AutoMergeCaller(unittest.TestCase):
    def setUp(self):
        self.text = read(".github", "workflows", "auto-merge.yml")

    def test_workflow_dispatch_follows_workflow_run_with_a_required_string_number(self):
        m = re.search(
            r"^on:\n  workflow_run:\n(?:    .*\n)+  workflow_dispatch:\n"
            r"    inputs:\n      number:\n((?:        .*\n)+)",
            self.text,
            re.M,
        )
        self.assertIsNotNone(m, "workflow_dispatch with input number must follow workflow_run")
        self.assertIn("        required: true\n", m.group(1))
        self.assertIn("        type: string\n", m.group(1))

    def test_merge_job_permissions_are_exactly_six(self):
        m = re.search(r"^    permissions:\n((?:      .*\n)+)", self.text, re.M)
        self.assertIsNotNone(m)
        grants = sorted(
            line.split("#")[0].strip() for line in m.group(1).splitlines() if line.strip()
        )
        self.assertEqual(
            grants,
            [
                "actions: read",
                "checks: read",
                "contents: write",
                "issues: write",
                "pull-requests: write",
                "statuses: read",
            ],
        )

    def test_the_rest_of_the_caller_is_unchanged(self):
        self.assertIn("\npermissions: {}\n", self.text)
        self.assertRegex(self.text, r"(?m)^    uses: .*auto-merge\.yml@main$")
        self.assertIn(
            'workflows: ["CLAUDE.md check", "Line endings", "Repository check", "Rust", "Scripts"]',
            self.text,
        )
        self.assertIn('prefixes: "claude/ bot/ tune/"', self.text)
        for line in ("rust checks", "eol / LF line endings", "script unit tests",
                     "secrets / no secrets or key files"):
            self.assertIn(f"        {line}\n", self.text)

    def test_no_with_line_mentions_number(self):
        m = re.search(r"^    with:\n((?:      .*\n)+)", self.text, re.M)
        self.assertIsNotNone(m)
        self.assertNotIn("number", m.group(1))

    def test_the_word_android_stays_out(self):
        self.assertNotIn("Android", self.text)


if __name__ == "__main__":
    unittest.main()
