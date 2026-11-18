"""Unit tests for scripts/device_check_issue.py (the issue Promote opens for each 0.N.0 release).

Linux-safe, no network: the whole-run tests put a fake `gh` first on PATH.
"""

import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(__file__))

import device_check_issue as dci

SCRIPT = Path(__file__).resolve().parent / "device_check_issue.py"
SHA = "d8a7ad14" + "0123456789abcdef0123456789ab"
TITLE = "Device check: v0.8.0"

# Fake gh: logs its arguments (one line each), answers `issue list` from FAKE_LIST, and fails
# `issue create` with the status in FAKE_CREATE_STATUS. A `--body-file` is copied to FAKE_BODY.
FAKE_GH = """#!@PYTHON@
import os, sys
args = sys.argv[1:]
with open(os.environ["FAKE_LOG"], "a") as f:
    f.write(" ".join(args) + "\\n")
if args[:2] == ["issue", "list"]:
    sys.stdout.write(os.environ.get("FAKE_LIST", "[]"))
    sys.exit(0)
if args[:2] == ["issue", "create"]:
    if "--body-file" in args:
        with open(args[args.index("--body-file") + 1], encoding="utf-8") as src:
            with open(os.environ["FAKE_BODY"], "w", encoding="utf-8") as dst:
                dst.write(src.read())
    sys.exit(int(os.environ.get("FAKE_CREATE_STATUS", "0")))
sys.exit(0)
"""


class Pure(unittest.TestCase):
    def test_which_tags_call_for_an_issue(self):
        for tag in ("v0.8.0", "v0.10.0", "v1.0.0", "v1.2.0"):
            self.assertTrue(dci.wants_issue(tag), tag)
        for tag in ("v0.7.1", "v0.8.0-rc.1", "v0.8", "0.8.0", "v0.8.0.0", "", "v0.8.10"):
            self.assertFalse(dci.wants_issue(tag), tag)

    def test_title(self):
        self.assertEqual(dci.issue_title("v0.8.0"), TITLE)

    def test_body_names_the_halves_and_the_decisions(self):
        body = dci.issue_body("v0.8.0", SHA)
        for needle in (
            "v0.8.0",
            SHA,
            "scripts/android_device_gate.py",
            "docs/device-gates.md",
            "aarch64-apple-ios",
            "ADR-REL-6",
            "ADR-PLAT-10",
        ):
            self.assertIn(needle, body)

    def test_body_has_one_box_per_role_in_order(self):
        boxes = [l for l in dci.issue_body("v0.8.0", SHA).splitlines() if l.startswith("- [ ]")]
        self.assertEqual(len(boxes), 3)
        for box, role in zip(boxes, ("Reference phone", "iOS reference device", "Profiling phones")):
            self.assertIn(role, box)

    def test_body_is_public_safe(self):
        body = dci.issue_body("v0.8.0", SHA)
        for bad in (
            "R5CR", "HA28", "SM-A", "Quantum", "Lenovo", "iPhone 13", "Galaxy", "/Users/",
            "tasks#", "DS-0", "stella-rain/tasks", "stella-rain/app",
        ):
            self.assertNotIn(bad, body)
        self.assertIsNone(re.search(r"^[A-Z0-9]{8,}$", body, re.M))
        for adr in re.findall(r"ADR-\S+", body):
            self.assertRegex(adr, r"^ADR-[A-Z]+-\d+")

    def test_existing_issue_is_an_exact_title_match(self):
        self.assertEqual(dci.existing_issue([{"number": 7, "title": TITLE}], TITLE), 7)
        self.assertIsNone(dci.existing_issue([{"number": 8, "title": "Device check: v0.8.1"}], TITLE))
        self.assertIsNone(dci.existing_issue([{"number": 9, "title": TITLE + " again"}], TITLE))


class WholeRun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        d = Path(self.tmp.name)
        gh = d / "gh"
        gh.write_text(FAKE_GH.replace("@PYTHON@", sys.executable))
        gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
        self.log = d / "log"
        self.body = d / "body"
        self.env = dict(os.environ, PATH=f"{d}{os.pathsep}{os.environ['PATH']}",
                        FAKE_LOG=str(self.log), FAKE_BODY=str(self.body))

    def run_script(self, *args, **env):
        return subprocess.run([sys.executable, str(SCRIPT), *args], env={**self.env, **env},
                              capture_output=True, text=True)

    def calls(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_opens_one_issue(self):
        r = self.run_script("--tag", "v0.8.0", "--commit", SHA)
        self.assertEqual(r.returncode, 0, r.stderr)
        creates = [c for c in self.calls() if c.startswith("issue create")]
        self.assertEqual(len(creates), 1)
        self.assertIn(f"--title {TITLE}", creates[0])
        self.assertIn("--label cmd:verify-needs-android", creates[0])
        self.assertIn("--label cmd:verify-needs-iphone", creates[0])
        self.assertEqual(self.body.read_text(encoding="utf-8"), dci.issue_body("v0.8.0", SHA))

    def test_a_second_run_opens_none(self):
        for state in ("OPEN", "CLOSED"):
            self.log.unlink(missing_ok=True)
            listing = json.dumps([{"number": 7, "title": TITLE, "state": state}])
            r = self.run_script("--tag", "v0.8.0", "--commit", SHA, FAKE_LIST=listing)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertFalse([c for c in self.calls() if c.startswith("issue create")])
            self.assertTrue(any("issue list" in c and "--state all" in c for c in self.calls()))

    def test_a_patch_or_rc_tag_calls_gh_not_at_all(self):
        for tag in ("v0.7.1", "v0.8.0-rc.1"):
            r = self.run_script("--tag", tag, "--commit", SHA)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(self.calls(), [])
            self.assertIn("device-issue:", r.stdout)
            self.assertIn("no issue", r.stdout)

    def test_a_failing_create_fails_the_run(self):
        r = self.run_script("--tag", "v0.8.0", "--commit", SHA, FAKE_CREATE_STATUS="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("device-issue:", r.stdout + r.stderr)

    def test_dry_run_prints_and_calls_nothing(self):
        r = self.run_script("--dry-run", "--tag", "v0.8.0", "--commit", SHA)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(TITLE, r.stdout)
        self.assertIn("- [ ] Reference phone", r.stdout)
        self.assertEqual(self.calls(), [])

    def test_bad_arguments_exit_2(self):
        self.assertEqual(self.run_script("--tag", "v0.8.0").returncode, 2)
        self.assertEqual(self.run_script("--commit", SHA).returncode, 2)


if __name__ == "__main__":
    unittest.main()
