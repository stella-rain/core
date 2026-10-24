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


class FinalTag(unittest.TestCase):
    """The version in Cargo.toml is a floor (release-flow plan 9.2): V above the latest final tag
    L is the release; V equal to or below L is L's next patch; no L means V."""

    def test_a_version_above_the_latest_release_is_the_release(self):
        self.assertEqual(r.final_tag("0.2.0", ["v0.1.0"]), "v0.2.0")
        self.assertEqual(r.final_tag("0.0.4", ["v0.0.2", "v0.0.3"]), "v0.0.4")

    def test_a_version_equal_to_the_latest_release_is_its_next_patch(self):
        self.assertEqual(r.final_tag("0.1.0", ["v0.1.0"]), "v0.1.1")

    def test_a_version_below_the_latest_release_is_its_next_patch(self):
        self.assertEqual(r.final_tag("0.0.9", ["v0.0.9", "v0.1.0"]), "v0.1.1")
        self.assertEqual(r.final_tag("0.0.1", ["v0.1.0"]), "v0.1.1")

    def test_the_latest_release_is_the_highest_not_the_last_listed(self):
        self.assertEqual(r.final_tag("0.0.1", ["v0.0.10", "v0.0.9"]), "v0.0.11")

    def test_the_first_release_is_the_version(self):
        self.assertEqual(r.final_tag("0.0.1", []), "v0.0.1")

    def test_rc_tags_are_not_releases(self):
        self.assertEqual(r.final_tag("0.1.0", ["v0.1.0-rc.1", "v0.1.0-rc.4"]), "v0.1.0")
        self.assertEqual(r.final_tag("0.1.0", ["v0.1.0", "v0.1.1-rc.1"]), "v0.1.1")

    def test_a_second_candidate_of_the_same_final_is_rc_2(self):
        tags = ["v0.1.0", "v0.1.1-rc.1"]
        self.assertEqual(r.rc_tag(r.final_tag("0.1.0", tags), tags), "v0.1.1-rc.2")

    def test_a_version_that_is_not_x_y_z(self):
        with self.assertRaisesRegex(r.ReleaseError, "not X.Y.Z"):
            r.final_tag("0.0.4-beta", [])


class CheckMain(unittest.TestCase):
    def test_main_passes(self):
        r.check_main("refs/heads/main")

    def test_anything_else_is_refused(self):
        for ref in ("refs/heads/claude/x", "refs/heads/rc/v0.0.4-rc.1", "refs/heads/candidate/0123456789ab",
                    "refs/tags/v0.0.4", "refs/heads/main2", "", None):
            with self.assertRaisesRegex(r.ReleaseError, "refs/heads/main", msg=ref):
                r.check_main(ref)


class Stamp(unittest.TestCase):
    def test_cargo_toml_takes_the_package_version_only(self):
        text = CARGO.format("0.1.0") + '\n[dependencies]\nserde = { version = "1.0.0" }\n'
        out = r.stamp_cargo_toml(text, "0.1.1")
        self.assertEqual(r.cargo_version(out), "0.1.1")
        self.assertIn('serde = { version = "1.0.0" }', out)
        self.assertEqual(out.replace("0.1.1", "0.1.0"), text)

    def test_a_lock_takes_the_crates_entry_only(self):
        out = r.stamp_lock(LOCK.format("0.1.0"), "0.1.1")
        self.assertEqual(out, LOCK.format("0.1.1"))
        self.assertIn('name = "libfuzzer-sys"\nversion = "0.4.10"', out)

    def test_a_lock_without_the_crate_is_refused(self):
        with self.assertRaisesRegex(r.ReleaseError, "has no"):
            r.stamp_lock(LOCK.split("\n\n")[0], "0.1.1")


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


def run(name, status="completed", conclusion="success", run_id=7, job=1):
    return {"name": name, "status": status, "conclusion": conclusion,
            "details_url": f"https://github.com/o/r/actions/runs/{run_id}/job/{job}"}


class ChecksPassed(unittest.TestCase):
    def test_all_finished_and_green(self):
        r.checks_passed([run("tests", run_id=1), run("fmt", run_id=1)], "9")

    def test_skipped_and_neutral_pass(self):
        r.checks_passed([run("short fuzz run", conclusion="skipped", run_id=1),
                         run("x", conclusion="neutral", run_id=1)], "9")

    def test_no_check_run_at_all_passes(self):
        # auto-merge.yml merges with GITHUB_TOKEN, which starts no push run (app#176), so main's
        # commit may have none: the PR's head ran the gates.
        r.checks_passed([], "9")
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
        # A candidate's tag is the main commit itself, so an earlier run's jobs sit on it.
        names = [*r.CANDIDATE_JOBS, "release iOS Simulator / tests in the iOS Simulator (debug)",
                 "release long fuzz / fuzz stage_run"]
        for name in names:
            r.checks_passed([run("tests", run_id=1), run(name, conclusion="failure", run_id=3)], "9")

    def test_auto_candidates_jobs_are_not_a_gate_whatever_their_conclusion(self):
        # A failed auto run must never make Candidate refuse the commit; a later run's own jobs
        # on the same sha are left out of the decision too.
        for name in ("decide on a candidate", "push the candidate branch"):
            for status, conclusion in (("completed", "failure"), ("in_progress", None)):
                r.checks_passed([run("tests", run_id=1), run(name, status=status, conclusion=conclusion, run_id=7)], "9")

    def test_auto_merges_job_is_not_a_gate_whatever_its_conclusion(self):
        # Candidate run 39693572498: auto-merge's runs on main's commit were cancelled by its
        # concurrency group when a newer run started.
        name = "merge / merge when every check is green"
        for conclusion in ("cancelled", "failure", "success"):
            r.checks_passed([run("tests", run_id=1), run(name, conclusion=conclusion, run_id=5)], "9")
        r.checks_passed([run(name, status="in_progress", conclusion=None, run_id=5)], "9")
        r.checks_passed([run(name, conclusion="cancelled", run_id=5)], "9")

    def test_a_real_gate_still_fails_beside_auto_merges_cancelled_job(self):
        merge = run("merge / merge when every check is green", conclusion="cancelled", run_id=5)
        with self.assertRaises(r.ReleaseError) as caught:
            r.checks_passed([merge, run("rust checks", conclusion="cancelled", run_id=1)], "9")
        self.assertIn("rust checks: cancelled", str(caught.exception))
        self.assertNotIn("merge when every check", str(caught.exception))

    def test_a_manual_run_of_the_same_workflows_is_a_check(self):
        with self.assertRaises(r.ReleaseError):
            r.checks_passed([run("tests", run_id=1),
                             run("tests in the iOS Simulator (debug)", conclusion="failure", run_id=3)], "9")


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


class PackChanged(unittest.TestCase):
    def test_equal_bytes_are_unchanged(self):
        self.assertFalse(r.pack_changed(b"a", b"a"))

    def test_different_bytes_are_changed(self):
        self.assertTrue(r.pack_changed(b"a", b"b"))

    def test_a_pack_missing_at_the_previous_tag_is_changed(self):
        self.assertTrue(r.pack_changed(None, b"b"))


class PackWarning(unittest.TestCase):
    LINE = ("content/pack.json changed since v0.0.3: publish a stella-rain/content release "
            "from this tag before an app build pins it (ADR-080)")

    def test_a_changed_pack_puts_the_warning_first(self):
        text = r.notes("v0.0.4-rc.1", SHA, "v0.0.3", [], pack_changed=True)
        self.assertEqual(text.splitlines()[0], self.LINE)

    def test_an_unchanged_pack_has_no_warning(self):
        self.assertNotIn("content/pack.json", r.notes("v0.0.4-rc.1", SHA, "v0.0.3", []))

    def test_the_first_release_has_no_warning(self):
        self.assertNotIn("content/pack.json", r.notes("v0.0.1-rc.1", SHA, None, [], pack_changed=True))


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
        with open(os.path.join(self.dev, "Cargo.lock"), "w") as f:
            f.write(LOCK.format(version))
        git(self.dev, "add", "-A")
        git(self.dev, "commit", "-q", "-m", message)

    def stamp_commit(self, final, message=None):
        """What Candidate makes: a commit on HEAD that changes the version files only."""
        self.commit(final[1:], message or f"Release {final}")
        return git(self.dev, "rev-parse", "HEAD")

    def notes_from_dev(self):
        log = os.path.join(self.dev, "log.txt")
        with open(log, "w") as f:
            f.write("")
        rc = script(self.dev, "notes", "v0.0.4-rc.1", SHA, git(self.dev, "describe", "--tags", "--abbrev=0"), log)
        self.assertEqual(rc.returncode, 0, rc.stderr)
        return rc.stdout

    def write_pack(self, content):
        os.makedirs(os.path.join(self.dev, "content"), exist_ok=True)
        with open(os.path.join(self.dev, "content", "pack.json"), "wb") as f:
            f.write(content)

    def test_notes_warn_when_the_pack_is_missing_at_the_previous_tag(self):
        self.write_pack(b"{}")
        self.assertTrue(self.notes_from_dev().startswith("content/pack.json changed since v0.0.3"))

    def release_with_pack(self, content):
        self.write_pack(content)
        git(self.dev, "add", "-A")
        git(self.dev, "commit", "-q", "-m", "pack")
        git(self.dev, "tag", "v0.0.4")

    def test_notes_warn_when_the_pack_differs_from_the_previous_tag(self):
        self.release_with_pack(b"1")
        self.write_pack(b"2")
        self.assertTrue(self.notes_from_dev().startswith("content/pack.json changed since v0.0.4"))

    def test_notes_do_not_warn_when_the_pack_equals_the_previous_tags(self):
        self.release_with_pack(b"1")
        self.assertNotIn("content/pack.json", self.notes_from_dev())

    def test_notes_do_not_warn_when_neither_has_a_pack(self):
        self.assertNotIn("content/pack.json", self.notes_from_dev())

    def test_final_prints_the_version_above_the_latest_release(self):
        out = script(self.dev, "final")
        self.assertEqual((out.returncode, out.stdout.strip()), (0, "v0.0.4"))

    def test_final_is_the_next_patch_of_a_released_version(self):
        git(self.dev, "tag", "v0.0.4")
        out = script(self.dev, "final")
        self.assertEqual((out.returncode, out.stdout.strip()), (0, "v0.0.5"))

    def test_final_is_the_next_patch_of_the_latest_release_when_the_file_is_below_it(self):
        self.commit("0.0.2", "back")
        git(self.dev, "tag", "v0.1.0")
        self.assertEqual(script(self.dev, "final").stdout.strip(), "v0.1.1")

    def test_final_refuses_a_fuzz_lock_that_disagrees(self):
        self.commit("0.0.5", "bump", lock="0.0.4")
        out = script(self.dev, "final")
        self.assertEqual(out.returncode, 1)
        self.assertIn("fuzz/Cargo.lock", out.stderr)

    def test_final_refuses_a_root_lock_that_disagrees(self):
        with open(os.path.join(self.dev, "Cargo.lock"), "w") as f:
            f.write(LOCK.format("0.0.3"))
        git(self.dev, "commit", "-q", "-am", "lock")
        out = script(self.dev, "final")
        self.assertEqual(out.returncode, 1)
        self.assertIn("Cargo.lock has", out.stderr)

    def test_stamp_rewrites_the_version_in_every_file_that_repeats_it(self):
        self.assertEqual(script(self.dev, "stamp", "v0.0.9").returncode, 0)
        read = lambda *p: open(os.path.join(self.dev, *p), encoding="utf-8").read()  # noqa: E731
        self.assertEqual(read("Cargo.toml"), CARGO.format("0.0.9"))
        self.assertEqual(read("Cargo.lock"), LOCK.format("0.0.9"))
        self.assertEqual(read("fuzz", "Cargo.lock"), LOCK.format("0.0.9"))
        self.assertEqual(git(self.dev, "status", "--porcelain").count("\n") + 1, 3)

    def test_stamp_refuses_a_name_that_is_not_a_final_tag(self):
        self.assertEqual(script(self.dev, "stamp", "v0.0.9-rc.1").returncode, 1)

    def test_check_main_reads_the_ref(self):
        self.assertEqual(script(self.dev, "check-main", "refs/heads/main").returncode, 0)
        out = script(self.dev, "check-main", "refs/heads/claude/x")
        self.assertEqual(out.returncode, 1)
        self.assertIn("refs/heads/main", out.stderr)

    def test_check_release_passes_with_a_commit_since_the_latest_release(self):
        self.assertEqual(script(self.dev, "check-release").returncode, 0)

    def test_check_release_passes_before_the_first_release(self):
        git(self.dev, "tag", "-d", "v0.0.3")
        self.assertEqual(script(self.dev, "check-release").returncode, 0)

    def test_check_release_refuses_when_main_is_the_latest_releases_commit(self):
        git(self.dev, "tag", "v0.0.4")
        out = script(self.dev, "check-release")
        self.assertEqual(out.returncode, 1)
        self.assertIn("nothing to release", out.stderr)

    def test_check_release_refuses_when_main_is_the_parent_of_the_stamped_release_commit(self):
        main = git(self.dev, "rev-parse", "HEAD")
        self.stamp_commit("v0.0.5")
        git(self.dev, "tag", "v0.0.5")
        git(self.dev, "reset", "-q", "--hard", main)
        out = script(self.dev, "check-release")
        self.assertEqual(out.returncode, 1)
        self.assertIn("nothing to release", out.stderr)

    def test_check_release_passes_on_a_commit_after_a_stamped_release(self):
        main = git(self.dev, "rev-parse", "HEAD")
        self.stamp_commit("v0.0.5")
        git(self.dev, "tag", "v0.0.5")
        git(self.dev, "reset", "-q", "--hard", main)
        with open(os.path.join(self.dev, "later.txt"), "w") as f:
            f.write("later")
        self.commit("0.0.4", "later")
        self.assertEqual(script(self.dev, "check-release").returncode, 0)

    def test_check_release_refuses_a_commit_that_is_already_a_candidate(self):
        git(self.dev, "tag", "v0.0.4-rc.1")
        out = script(self.dev, "check-release")
        self.assertEqual(out.returncode, 1)
        self.assertIn("already a candidate", out.stderr)

    def test_check_release_refuses_a_commit_whose_stamped_child_is_a_candidate(self):
        main = git(self.dev, "rev-parse", "HEAD")
        self.stamp_commit("v0.0.5")
        git(self.dev, "tag", "v0.0.5-rc.1")
        git(self.dev, "reset", "-q", "--hard", main)
        out = script(self.dev, "check-release")
        self.assertEqual(out.returncode, 1)
        self.assertIn("already a candidate", out.stderr)

    def test_a_commit_that_only_looks_like_a_release_commit_is_a_main_commit(self):
        # Same subject, but it changes more than the version files: it is main's own commit.
        with open(os.path.join(self.dev, "other.txt"), "w") as f:
            f.write("x")
        self.commit("0.0.4", "Release v0.0.4")
        git(self.dev, "add", "-A")
        git(self.dev, "commit", "-q", "--amend", "--no-edit")
        git(self.dev, "tag", "v0.0.4")
        # Taken for a release commit, its parent would be main's commit and HEAD one commit past it.
        out = script(self.dev, "check-release")
        self.assertEqual(out.returncode, 1)
        self.assertIn("nothing to release", out.stderr)

    def test_rc_counts_up(self):
        self.assertEqual(script(self.dev, "rc", "v0.0.4").stdout.strip(), "v0.0.4-rc.1")
        git(self.dev, "tag", "v0.0.4-rc.1")
        self.assertEqual(script(self.dev, "rc", "v0.0.4").stdout.strip(), "v0.0.4-rc.2")

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
        bad = '{"check_runs": [{"name": "t", "status": "completed", "conclusion": "failure", "details_url": "x/runs/1/job/2"}]}'
        self.assertEqual(script(self.dev, "checks-passed", "9", stdin=bad).returncode, 1)
        self.assertEqual(script(self.dev, "checks-passed", "9", stdin='{"check_runs": []}').returncode, 0)

    def test_an_unknown_command_is_a_usage_error(self):
        self.assertEqual(script(self.dev, "nope").returncode, 2)


if __name__ == "__main__":
    unittest.main()
