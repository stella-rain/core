"""Tests for release.py. Run: python3 -m unittest discover -s .github/scripts

Ported from kade-agent/agent-config's scripts/release.test.cjs; core's tags are semver, `vX.Y.Z`,
and its version is the crate version in Cargo.toml.
"""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import release as r  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(__file__), "release.py")
CARGO = '[package]\nname = "stella-rain-core"\nversion = "{}"\nedition = "2021"\n'
LOCK = ('[[package]]\nname = "libfuzzer-sys"\nversion = "0.4.10"\n\n'
        '[[package]]\nname = "stella-rain-core"\nversion = "{}"\n')
SHA = "0123456789abcdef0123456789abcdef01234567"


class Previous(unittest.TestCase):
    def test_the_highest_semver_wins_not_the_last_listed(self):
        self.assertEqual(r.previous(["v0.0.10", "v0.0.9", "v0.1.0", "v0.0.2"]), "v0.1.0")

    def test_numbers_compare_as_numbers(self):
        self.assertEqual(r.previous(["v0.0.9", "v0.0.10"]), "v0.0.10")

    def test_candidate_and_other_tags_are_not_releases(self):
        self.assertEqual(r.previous(["v1.0", "x1.0.0", "v0.0.4-rc.2", "v0.0.3"]), "v0.0.3")

    def test_no_release_yet(self):
        self.assertIsNone(r.previous(["v0.0.4-rc.1"]))


class NewVersion(unittest.TestCase):
    def test_a_version_above_the_last_release_is_new(self):
        self.assertEqual(r.new_version_problems("0.0.4", ["v0.0.2", "v0.0.3"]), [])

    def test_the_first_release_is_new(self):
        self.assertEqual(r.new_version_problems("0.0.1", []), [])

    def test_a_tagged_version_is_not_new(self):
        self.assertIn("already tagged", r.new_version_problems("0.0.3", ["v0.0.3"])[0])

    def test_a_version_below_the_last_release_is_not_new(self):
        self.assertIn("not above", r.new_version_problems("0.0.2", ["v0.0.3"])[0])

    def test_rc_tags_of_the_version_do_not_make_it_released(self):
        self.assertEqual(r.new_version_problems("0.0.4", ["v0.0.3", "v0.0.4-rc.1"]), [])

    def test_a_version_that_is_not_x_y_z(self):
        self.assertIn("not X.Y.Z", r.new_version_problems("0.0.4-beta", [])[0])


class Lock(unittest.TestCase):
    def test_the_lock_agrees(self):
        self.assertEqual(r.lock_problems("0.0.4", LOCK.format("0.0.4")), [])

    def test_the_lock_disagrees(self):
        self.assertIn("0.0.3", r.lock_problems("0.0.4", LOCK.format("0.0.3"))[0])

    def test_the_lock_has_no_crate(self):
        self.assertIn("has no", r.lock_problems("0.0.4", LOCK.split("\n\n")[0])[0])


class RcTag(unittest.TestCase):
    def test_the_first_candidate_is_rc_1(self):
        self.assertEqual(r.rc_tag("v0.0.4", ["v0.0.3"]), "v0.0.4-rc.1")

    def test_the_next_candidate_is_one_above_the_highest(self):
        tags = ["v0.0.4-rc.1", "v0.0.4-rc.9", "v0.0.4-rc.10", "v0.0.5-rc.30"]
        self.assertEqual(r.rc_tag("v0.0.4", tags), "v0.0.4-rc.11")

    def test_a_candidate_tag_is_no_version(self):
        with self.assertRaises(r.ReleaseError):
            r.rc_tag("v0.0.4-rc.1", [])


class Branches(unittest.TestCase):
    BRANCHES = ["main", "rc/v0.0.3-rc.1", "rc/v0.0.4-rc.1", "rc/v0.0.4-rc.2", "rc/v0.0.5-rc.1",
                "claude/x", "rc/junk"]

    def test_rc_of_branch(self):
        self.assertEqual(r.rc_of_branch("rc/v0.0.4-rc.2"), "v0.0.4-rc.2")

    def test_rc_of_branch_refuses_other_branches(self):
        for bad in ("main", "rc", "rc/v0.0.4", "candidate/0123456789ab", "", None):
            with self.assertRaises(r.ReleaseError, msg=bad):
                r.rc_of_branch(bad)

    def test_superseded_are_the_lower_rcs_of_the_same_version(self):
        self.assertEqual(r.superseded_branches(self.BRANCHES, "v0.0.4-rc.2"), ["rc/v0.0.4-rc.1"])

    def test_the_first_candidate_supersedes_nothing(self):
        self.assertEqual(r.superseded_branches(self.BRANCHES, "v0.0.4-rc.1"), [])

    def test_released_are_the_rcs_of_that_version_and_older(self):
        self.assertEqual(r.released_branches(self.BRANCHES, "v0.0.4"),
                         ["rc/v0.0.3-rc.1", "rc/v0.0.4-rc.1", "rc/v0.0.4-rc.2"])


class CheckCandidate(unittest.TestCase):
    NAME = f"candidate/{SHA[:12]}"

    def test_a_branch_naming_its_commit_on_main_alone_passes(self):
        r.check_candidate(self.NAME, SHA, True, [])

    def test_a_wrong_name(self):
        with self.assertRaisesRegex(r.ReleaseError, "not candidate/<12"):
            r.check_candidate("candidate/main", SHA, True, [])

    def test_a_name_of_another_commit(self):
        with self.assertRaisesRegex(r.ReleaseError, "names"):
            r.check_candidate(self.NAME, "f" * 40, True, [])

    def test_a_commit_off_main(self):
        with self.assertRaisesRegex(r.ReleaseError, "not on origin/main"):
            r.check_candidate(self.NAME, SHA, False, [])

    def test_another_candidate_branch(self):
        with self.assertRaisesRegex(r.ReleaseError, "left over"):
            r.check_candidate(self.NAME, SHA, True, ["candidate/aaaaaaaaaaaa"])


def run(name, status="completed", conclusion="success", run_id=7, job=1):
    return {"name": name, "status": status, "conclusion": conclusion,
            "details_url": f"https://github.com/o/r/actions/runs/{run_id}/job/{job}"}


class ChecksPassed(unittest.TestCase):
    def test_all_finished_and_green(self):
        r.checks_passed([run("tests", run_id=1), run("fmt", run_id=1)], "9")

    def test_skipped_and_neutral_pass(self):
        r.checks_passed([run("short fuzz run", conclusion="skipped", run_id=1),
                         run("x", conclusion="neutral", run_id=1)], "9")

    def test_no_check_run_besides_this_one(self):
        with self.assertRaisesRegex(r.ReleaseError, "no check run"):
            r.checks_passed([run("check and test", run_id=9)], "9")

    def test_a_failure_is_named(self):
        with self.assertRaisesRegex(r.ReleaseError, "tests on arm64: failure"):
            r.checks_passed([run("fmt", run_id=1), run("tests on arm64", conclusion="failure", run_id=1)], "9")

    def test_an_unfinished_check_is_named(self):
        with self.assertRaisesRegex(r.ReleaseError, "tests: in_progress"):
            r.checks_passed([run("tests", status="in_progress", conclusion=None, run_id=1)], "9")

    def test_cancelled_timed_out_and_action_required_fail(self):
        for conclusion in ("cancelled", "timed_out", "action_required", "stale", "startup_failure"):
            with self.assertRaises(r.ReleaseError, msg=conclusion):
                r.checks_passed([run("tests", conclusion=conclusion, run_id=1)], "9")

    def test_this_runs_own_jobs_are_left_out(self):
        r.checks_passed([run("tests", run_id=1), run("check", status="in_progress", conclusion=None, run_id=9)], "9")

    def test_the_candidate_workflows_jobs_of_any_run_are_left_out(self):
        for name in r.CANDIDATE_JOBS:
            r.checks_passed([run("tests", run_id=1), run(name, conclusion="failure", run_id=3)], "9")


class CheckPromote(unittest.TestCase):
    def test_the_highest_rc_of_a_new_version(self):
        tags = ["v0.0.3", "v0.0.4-rc.1", "v0.0.4-rc.2"]
        self.assertEqual(r.check_promote("v0.0.4-rc.2", tags, "0.0.4"), "v0.0.4")

    def test_not_a_candidate_tag(self):
        with self.assertRaisesRegex(r.ReleaseError, "not a candidate tag"):
            r.check_promote("v0.0.4", ["v0.0.4"], "0.0.4")

    def test_a_newer_candidate_exists(self):
        with self.assertRaisesRegex(r.ReleaseError, "rc.2 is newer"):
            r.check_promote("v0.0.4-rc.1", ["v0.0.4-rc.1", "v0.0.4-rc.2"], "0.0.4")

    def test_already_released(self):
        with self.assertRaisesRegex(r.ReleaseError, "already released"):
            r.check_promote("v0.0.4-rc.1", ["v0.0.4", "v0.0.4-rc.1"], "0.0.4")

    def test_not_above_the_latest_release(self):
        with self.assertRaisesRegex(r.ReleaseError, "not above"):
            r.check_promote("v0.0.4-rc.1", ["v0.0.5", "v0.0.4-rc.1"], "0.0.4")

    def test_the_crate_version_at_the_tag_is_the_version(self):
        with self.assertRaisesRegex(r.ReleaseError, "Cargo.toml at v0.0.4-rc.1 has 0.0.3"):
            r.check_promote("v0.0.4-rc.1", ["v0.0.4-rc.1"], "0.0.3")


class Notes(unittest.TestCase):
    def test_the_notes_name_the_candidate_the_commit_and_the_changes(self):
        text = r.notes("v0.0.4-rc.1", SHA, "v0.0.3", ["abc1234 One", "", "def5678 Two"])
        self.assertIn("v0.0.4", text.splitlines()[0])
        self.assertIn(f"`{SHA}`", text)
        self.assertIn("Commits since v0.0.3:", text)
        self.assertIn("- abc1234 One\n- def5678 Two", text)
        self.assertIn("SHA256SUMS", text)

    def test_the_first_release_has_no_previous(self):
        self.assertIn("First release.", r.notes("v0.0.1-rc.1", SHA, None, []))


def git(cwd, *args):
    return subprocess.run(["git", "-C", cwd, *args], check=True, capture_output=True, text=True).stdout.strip()


def script(cwd, *args, stdin=None):
    return subprocess.run([sys.executable, SCRIPT, *args], cwd=cwd, input=stdin, capture_output=True, text=True)


class Cli(unittest.TestCase):
    """The commands against a real clone of a real origin."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = self.tmp.name
        self.origin = os.path.join(root, "origin.git")
        self.dev = os.path.join(root, "dev")
        git(root, "init", "-q", "--bare", "-b", "main", self.origin)
        git(root, "clone", "-q", self.origin, self.dev)
        git(self.dev, "symbolic-ref", "HEAD", "refs/heads/main")
        git(self.dev, "config", "user.email", "t@t")
        git(self.dev, "config", "user.name", "t")
        os.makedirs(os.path.join(self.dev, "fuzz"))
        self.commit("0.0.3", "first")
        git(self.dev, "tag", "v0.0.3")
        git(self.dev, "push", "-q", "origin", "main", "v0.0.3")
        self.commit("0.0.4", "version 0.0.4")
        git(self.dev, "push", "-q", "origin", "main")

    def commit(self, version, message, lock=None):
        with open(os.path.join(self.dev, "Cargo.toml"), "w") as f:
            f.write(CARGO.format(version))
        with open(os.path.join(self.dev, "fuzz", "Cargo.lock"), "w") as f:
            f.write(LOCK.format(lock or version))
        git(self.dev, "add", "-A")
        git(self.dev, "commit", "-q", "-m", message)

    def candidate(self):
        return "candidate/" + git(self.dev, "rev-parse", "HEAD")[:12]

    def test_version_prints_the_new_tag(self):
        out = script(self.dev, "version")
        self.assertEqual((out.returncode, out.stdout.strip()), (0, "v0.0.4"))

    def test_version_refuses_a_lock_that_disagrees(self):
        self.commit("0.0.5", "bump", lock="0.0.4")
        out = script(self.dev, "version")
        self.assertEqual(out.returncode, 1)
        self.assertIn("fuzz/Cargo.lock", out.stderr)

    def test_version_refuses_a_released_version(self):
        self.commit("0.0.3", "back")
        self.assertEqual(script(self.dev, "version").returncode, 1)

    def test_rc_counts_up(self):
        self.assertEqual(script(self.dev, "rc", "v0.0.4").stdout.strip(), "v0.0.4-rc.1")
        git(self.dev, "tag", "v0.0.4-rc.1")
        self.assertEqual(script(self.dev, "rc", "v0.0.4").stdout.strip(), "v0.0.4-rc.2")

    def test_check_candidate_passes_for_a_branch_at_main(self):
        name = self.candidate()
        git(self.dev, "push", "-q", "origin", f"HEAD:refs/heads/{name}")
        self.assertEqual(script(self.dev, "check-candidate", name).returncode, 0)

    def test_check_candidate_refuses_a_commit_off_main(self):
        self.commit("0.0.5", "off main")
        name = self.candidate()
        git(self.dev, "push", "-q", "origin", f"HEAD:refs/heads/{name}")
        out = script(self.dev, "check-candidate", name)
        self.assertEqual(out.returncode, 1)
        self.assertIn("not on origin/main", out.stderr)

    def test_check_candidate_refuses_a_left_over_branch(self):
        name = self.candidate()
        git(self.dev, "push", "-q", "origin", f"HEAD:refs/heads/{name}",
            "HEAD~1:refs/heads/candidate/aaaaaaaaaaaa")
        out = script(self.dev, "check-candidate", name)
        self.assertEqual(out.returncode, 1)
        self.assertIn("candidate/aaaaaaaaaaaa", out.stderr)

    def test_superseded_and_released_read_origins_rc_branches(self):
        for b in ("rc/v0.0.3-rc.1", "rc/v0.0.4-rc.1", "rc/v0.0.4-rc.2"):
            git(self.dev, "push", "-q", "origin", f"HEAD:refs/heads/{b}")
        self.assertEqual(script(self.dev, "superseded", "v0.0.4-rc.2").stdout.split(),
                         ["rc/v0.0.4-rc.1"])
        self.assertEqual(script(self.dev, "released", "v0.0.4").stdout.split(),
                         ["rc/v0.0.3-rc.1", "rc/v0.0.4-rc.1", "rc/v0.0.4-rc.2"])

    def test_check_promote_reads_the_crate_version_at_the_tag(self):
        git(self.dev, "tag", "v0.0.4-rc.1")
        out = script(self.dev, "check-promote", "v0.0.4-rc.1")
        self.assertEqual((out.returncode, out.stdout.strip()), (0, "v0.0.4"))
        git(self.dev, "tag", "v0.0.5-rc.1", "v0.0.3")
        self.assertEqual(script(self.dev, "check-promote", "v0.0.5-rc.1").returncode, 1)

    def test_checks_passed_reads_the_api_json_on_stdin(self):
        ok = '{"check_runs": [{"name": "t", "status": "completed", "conclusion": "success", "details_url": "x/runs/1/job/2"}]}'
        self.assertEqual(script(self.dev, "checks-passed", "9", stdin=ok).returncode, 0)
        self.assertEqual(script(self.dev, "checks-passed", "9", stdin='{"check_runs": []}').returncode, 1)

    def test_an_unknown_command_is_a_usage_error(self):
        self.assertEqual(script(self.dev, "nope").returncode, 2)


if __name__ == "__main__":
    unittest.main()
