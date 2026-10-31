"""Tests for android-emu.yml and its calls. Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflows are read as lines. The workflow runs core's tests in an
Android emulator for Candidate; it is never a PR check, and it is advisory first (CORE-ANDROID-EMU).
"""

import os
import re
import unittest

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")


def read(*parts):
    with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
        return f.read()


def job_block(text, job_id):
    m = re.search(rf"^  {re.escape(job_id)}:\n((?:(?:   .*)?\n)*)", text, re.M)
    assert m, f"no job {job_id}"
    return m.group(1)


class AndroidEmuWorkflow(unittest.TestCase):
    def setUp(self):
        self.text = read(".github", "workflows", "android-emu.yml")
        self.job = job_block(self.text, "android-emu")

    def test_it_has_no_permissions_at_the_top(self):
        self.assertRegex(self.text, r"(?m)^permissions: \{\}$")

    def test_the_job_reads_the_contents_and_nothing_else(self):
        m = re.search(r"(?m)^    permissions:\n((?:      .*\n)+)", self.job)
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1).strip(), "contents: read")

    def test_it_is_not_a_pr_check(self):
        on = re.search(r"(?m)^on:\n((?:  .*\n|\n)+)", self.text).group(1)
        triggers = re.findall(r"(?m)^  (\w+):", on)
        self.assertEqual(sorted(triggers), ["workflow_call", "workflow_dispatch"])
        self.assertNotIn("pull_request", self.text)
        self.assertNotIn("push:", self.text)

    def test_every_checkout_drops_the_credentials(self):
        checkouts = self.text.count("uses: actions/checkout@")
        self.assertGreaterEqual(checkouts, 1)
        self.assertEqual(self.text.count("persist-credentials: false"), checkouts)

    def test_every_action_is_pinned_to_a_major_version(self):
        uses = re.findall(r"(?m)^\s+(?:- )?uses: (\S+)", self.text)
        self.assertTrue(uses)
        for u in uses:
            self.assertRegex(u, r"^[\w.-]+/[\w./-]+@v\d+$", u)

    def test_it_uses_the_emulator_runner_action(self):
        self.assertIn("reactivecircus/android-emulator-runner@v2", self.text)
        self.assertIn("arch: x86_64", self.text)

    def test_cargo_links_with_the_ndk_and_runs_the_emulator_runner(self):
        self.assertIn("x86_64-linux-android24-clang", self.text)
        self.assertIn("CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER", self.text)
        m = re.search(r"CARGO_TARGET_X86_64_LINUX_ANDROID_RUNNER: (\S+)", self.text)
        self.assertIsNotNone(m)
        self.assertTrue(m.group(1).endswith(".github/scripts/android-emu-runner.sh"), m.group(1))
        self.assertIn("--selfcheck", self.text)

    def test_the_emulator_gets_kvm(self):
        self.assertIn("/dev/kvm", self.text)
        self.assertIn("udevadm trigger", self.text)

    def test_the_job_and_both_long_steps_have_timeouts(self):
        self.assertRegex(self.job, r"(?m)^    timeout-minutes: \d+$")
        self.assertGreaterEqual(len(re.findall(r"(?m)^        timeout-minutes: \d+$", self.job)), 2)

    def test_it_tests_debug_and_release(self):
        self.assertRegex(self.job, r"profile: \[debug, release\]")


class CandidateCallsIt(unittest.TestCase):
    def setUp(self):
        self.text = read(".github", "workflows", "candidate.yml")

    def test_the_job_calls_the_workflow(self):
        job = job_block(self.text, "android-emulator")
        self.assertIn("name: release Android emulator", job)
        self.assertIn("uses: ./.github/workflows/android-emu.yml", job)
        self.assertIn("contents: read", job)

    def test_it_is_advisory_first_not_a_need_of_the_tag_job(self):
        tag = job_block(self.text, "tag")
        needs = re.search(r"(?m)^    needs: \[(.*)\]$", tag).group(1)
        self.assertNotIn("android-emulator", needs)
        self.assertIn("ios-simulator", needs)


class OtherFiles(unittest.TestCase):
    def test_it_is_not_a_pr_check(self):
        self.assertNotIn("Android", read(".github", "workflows", "auto-merge.yml"))

    def test_the_rule_file_names_the_runner(self):
        self.assertIn("android-emu-runner.sh", read(".claude", "rules", "ci.md"))


if __name__ == "__main__":
    unittest.main()
