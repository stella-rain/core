"""Tests for .github/workflows/candidate.yml. Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflow is read as lines.
"""

import os
import re
import unittest

WORKFLOW = os.path.join(os.path.dirname(__file__), "..", "workflows", "candidate.yml")


def job_block(text, job_id):
    m = re.search(rf"^  {re.escape(job_id)}:\n((?:(?:   .*)?\n)*)", text, re.M)
    assert m, f"no job {job_id}"
    return m.group(1)


class CleanupKeepsTheBranchOnFailure(unittest.TestCase):
    def setUp(self):
        with open(WORKFLOW, encoding="utf-8") as f:
            self.cleanup = job_block(f.read(), "cleanup")
        m = re.search(r"^    if: (.*)$", self.cleanup, re.M)
        self.assertIsNotNone(m, "the delete job has no `if`")
        self.condition = m.group(1)

    def test_deletes_only_after_the_tag_job_succeeded(self):
        self.assertIn("needs.tag.result == 'success'", self.condition)

    def test_always_is_not_in_the_condition(self):
        self.assertNotIn("always()", self.condition)

    def test_waits_for_the_tag_job(self):
        m = re.search(r"^    needs: \[(.*)\]$", self.cleanup, re.M)
        self.assertIsNotNone(m)
        self.assertIn("tag", [n.strip() for n in m.group(1).split(",")])


if __name__ == "__main__":
    unittest.main()
