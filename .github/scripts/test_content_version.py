"""Tests for content_version.py and the way content-publish.yml calls it.
Run: python3 -m unittest discover -s .github/scripts

The publish pull request sets content's VERSION: kept when it is above content's latest final
`v*` tag (a release is pending), else that tag's next patch.
"""

import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "content_version.py")
WORKFLOW = os.path.join(HERE, "..", "workflows", "content-publish.yml")
sys.path.insert(0, HERE)

BAD_VERSIONS = ["", "0.0", "v0.0.8", "0.0.8-rc.1", "1.2.3.4", "01.2.3"]


def cli(*args, stdin=""):
    return subprocess.run(
        [sys.executable, SCRIPT, *args], input=stdin, capture_output=True, text=True, check=False
    )


def version_file(text):
    f = tempfile.NamedTemporaryFile("w", suffix="VERSION", delete=False)
    f.write(text)
    f.close()
    return f.name


class Next(unittest.TestCase):
    def test_values(self):
        from content_version import next as nxt

        for version, tags, want in [
            ("0.0.8", ["v0.0.7"], "0.0.8"),
            ("0.0.7", ["v0.0.7"], "0.0.8"),
            ("0.0.6", ["v0.0.5", "v0.0.7", "v0.0.6"], "0.0.8"),
            ("0.0.6", [], "0.0.6"),
            ("0.0.6", ["v0.0.7-rc.1", "foo", "v1.2", "v01.2.3", "vx.y.z"], "0.0.6"),
            ("0.0.9", ["v0.0.10"], "0.0.11"),
            ("0.0.10", ["v0.0.9"], "0.0.10"),
            ("0.1.0", ["v0.0.9"], "0.1.0"),
            ("1.0.0", ["v0.9.9"], "1.0.0"),
            ("0.0.7", ["v0.0.7-rc.1"], "0.0.7"),
            (" 0.0.8\n", ["v0.0.7"], "0.0.8"),
        ]:
            with self.subTest(version=version, tags=tags):
                self.assertEqual(nxt(version, tags), want)

    def test_refused(self):
        from content_version import next as nxt

        for bad in BAD_VERSIONS:
            with self.subTest(version=bad):
                with self.assertRaises(ValueError):
                    nxt(bad, [])


class Cli(unittest.TestCase):
    STDIN = "refs/tags/v0.0.7\nrefs/tags/v0.0.7-rc.1\n\n"

    def test_bumps_below_the_latest_tag(self):
        r = cli(version_file("0.0.6\n"), stdin=self.STDIN)
        self.assertEqual((r.stdout, r.returncode), ("0.0.8\n", 0))

    def test_keeps_above_the_latest_tag(self):
        r = cli(version_file("0.0.8\n"), stdin=self.STDIN)
        self.assertEqual((r.stdout, r.returncode), ("0.0.8\n", 0))

    def test_bad_file(self):
        r = cli(version_file("abc\n"), stdin=self.STDIN)
        self.assertEqual((r.stdout, r.returncode), ("", 1))
        self.assertTrue(r.stderr.startswith("content_version.py: "))

    def test_missing_file(self):
        r = cli("/nonexistent/VERSION", stdin=self.STDIN)
        self.assertEqual((r.stdout, r.returncode), ("", 1))
        self.assertTrue(r.stderr.startswith("content_version.py: "))

    def test_no_argument(self):
        r = cli(stdin=self.STDIN)
        self.assertEqual((r.stdout, r.returncode), ("", 1))
        self.assertIn("usage", r.stderr)


class Workflow(unittest.TestCase):
    """content-publish.yml calls the script and carries VERSION into the commit and the PR."""

    @classmethod
    def setUpClass(cls):
        with open(WORKFLOW, encoding="utf-8") as f:
            cls.text = f.read()
        cls.lines = cls.text.splitlines()

    def test_calls_the_script(self):
        self.assertIn("content_version.py", self.text)

    def test_reads_every_final_tag_page_by_page(self):
        line = next((l for l in self.lines if "matching-refs/tags/v" in l), "")
        self.assertIn("gh api", line)
        self.assertIn("--paginate", line)

    def test_script_comes_from_the_workflows_own_commit(self):
        self.assertIn("ref: ${{ github.sha }}", self.text)
        self.assertIn("sparse-checkout: .github/scripts\n", self.text)  # a directory: cone mode

    def test_version_is_part_of_the_diff_and_the_commit(self):
        diff = next((l for l in self.lines if "git diff --quiet HEAD --" in l), "")
        add = next((l for l in self.lines if l.strip().startswith("git add ")), "")
        self.assertIn("VERSION", diff)
        self.assertIn("VERSION", add)

    def test_commit_and_pr_name_the_value(self):
        self.assertGreaterEqual(self.text.count("$CONTENT_VERSION"), 3)

    def test_permissions_stay_empty(self):
        self.assertIn("\npermissions: {}\n", self.text)
