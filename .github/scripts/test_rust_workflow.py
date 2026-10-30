"""Tests for rust.yml's test job. Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflow is read as lines. The x86_64 test job also runs the
ignored command-cap measurement, so a rise of the longest bot run fails in the PR that causes it.
"""

import os
import re
import unittest

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
WORKFLOWS = os.path.join(ROOT, ".github", "workflows")
STEP_NAME = "Command cap measurement (longest bot run)"
RELEASE_RUN = "cargo nextest run --locked --release --profile ci"
CAP_RUN = RELEASE_RUN + " --run-ignored only -E 'test(=command_cap::measure_the_longest_run)'"


def read(name):
    with open(os.path.join(WORKFLOWS, name), encoding="utf-8") as f:
        return f.read()


def read_root(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


def job_block(text, job_id):
    m = re.search(rf"^  {re.escape(job_id)}:\n((?:(?:   .*)?\n)*)", text, re.M)
    assert m, f"no job {job_id}"
    return m.group(1)


def step(block, name):
    """The step named `name`, from its `- name:` line to the next step."""
    m = re.search(
        rf"^      - name: {re.escape(name)}\n((?:(?!      - ).*\n)*)", block, re.M
    )
    assert m, f"no step '{name}'"
    # Comments above the next step sit before its `- ` line: not part of this one.
    return re.sub(r"(?:^ *#.*\n)+\Z", "", m.group(0), flags=re.M)


class CommandCapStep(unittest.TestCase):
    def setUp(self):
        self.text = read("rust.yml")
        self.test = job_block(self.text, "test")

    def test_the_test_job_runs_the_command_cap_measurement(self):
        s = step(self.test, STEP_NAME)
        self.assertEqual(re.findall(r"^\s+(?:- )?if:.*$", s, re.M), ["        if: matrix.arch == 'x86_64'"])
        self.assertIn(CAP_RUN, s)
        for banned in ("--all-features", "--run-ignored all", "continue-on-error"):
            self.assertNotIn(banned, s)

    def test_it_reuses_the_release_build(self):
        lines = self.test.splitlines()
        plain = lines.index("      - run: " + RELEASE_RUN)
        cap = next(i for i, l in enumerate(lines) if l == f"      - name: {STEP_NAME}")
        self.assertLess(plain, cap)

    def test_it_is_gated_like_the_other_test_steps(self):
        header = self.test.split("    steps:")[0]
        self.assertIn("needs: changes", header)
        self.assertIn("if: needs.changes.outputs.code == 'true'", header)
        self.assertRegex(job_block(self.text, "checks"), r"needs:.*\btest\b")
        self.assertIn(r"grep -qv '\.md$'", job_block(self.text, "changes"))

    def test_the_failure_tells_the_author_to_re_pin_in_the_pr(self):
        s = step(self.test, STEP_NAME)
        self.assertRegex(s, r"::error[^\n]*MEASURED_MOST")
        self.assertIn("in this PR", s)
        self.assertIn("tests/command_cap.rs", s)
        self.assertTrue(s.rstrip().endswith('exit "$status"'))

    def test_the_junit_files_stay_apart(self):
        s = step(self.test, STEP_NAME)
        first = s.index("mv target/nextest/ci/junit.xml target/nextest/ci/junit-release.xml")
        run = s.index(CAP_RUN)
        last = s.index("mv target/nextest/ci/junit.xml target/nextest/ci/junit-command-cap.xml")
        self.assertLess(first, run)
        self.assertLess(run, last)
        self.assertIn("path: target/nextest/ci/junit*.xml", self.test)

    def test_the_test_it_names_exists_and_says_re_pin(self):
        src = read_root("tests/command_cap.rs")
        self.assertRegex(src, r"#\[ignore[^\n]*\]\nfn measure_the_longest_run")
        self.assertIn("re-pin MEASURED_MOST", src)
        self.assertIn("--run-ignored all", read("deep.yml"))

    def test_workflow_rules_hold(self):
        lines = self.text.splitlines()
        self.assertLess(lines.index("permissions: {}"), lines.index("jobs:"))
        self.assertIn("contents: read", self.test)
        for i, l in enumerate(lines):
            if "uses: actions/checkout@" in l:
                self.assertIn("persist-credentials: false", "\n".join(lines[i : i + 6]))


if __name__ == "__main__":
    unittest.main()
