"""Tests for release_tag.py. Run: python3 -m unittest discover -s .github/scripts

Follows enjay27/lakehouse's install/test_release_tag.py; core's tags are semver, `vX.Y.Z`.
"""

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import release_tag as rt  # noqa: E402

CARGO = '[package]\nname = "stella-rain-core"\nversion = "0.0.4"\nedition = "2021"\n'
LOCK = ('[[package]]\nname = "libfuzzer-sys"\nversion = "0.4.10"\n\n'
        '[[package]]\nname = "stella-rain-core"\nversion = "{}"\n')


class Previous(unittest.TestCase):
    def test_the_highest_semver_wins_not_the_last_listed(self):
        self.assertEqual(rt.previous(["v0.0.10", "v0.0.9", "v0.1.0", "v0.0.2"]), "v0.1.0")

    def test_numbers_compare_as_numbers(self):
        self.assertEqual(rt.previous(["v0.0.9", "v0.0.10"]), "v0.0.10")

    def test_other_tags_are_ignored(self):
        self.assertEqual(rt.previous(["v1.0", "x1.0.0", "v1.0.0-rc1", "v0.0.3"]), "v0.0.3")

    def test_no_release_yet(self):
        self.assertIsNone(rt.previous([]))


class NewVersion(unittest.TestCase):
    def test_a_version_above_the_last_release_is_new(self):
        self.assertEqual(rt.new_version_problems("0.0.4", ["v0.0.2", "v0.0.3"]), [])

    def test_the_first_release_is_new(self):
        self.assertEqual(rt.new_version_problems("0.0.1", []), [])

    def test_an_existing_tag_is_not_new(self):
        problems = rt.new_version_problems("0.0.3", ["v0.0.2", "v0.0.3"])
        self.assertEqual(problems, ["v0.0.3 is already tagged; bump the version in Cargo.toml"])

    def test_a_version_below_the_last_release_is_not_new(self):
        problems = rt.new_version_problems("0.0.2", ["v0.0.3"])
        self.assertEqual(problems, ["0.0.2 is not above the last release v0.0.3"])

    def test_a_version_that_is_not_x_y_z_is_refused(self):
        for version in ("0.0", "0.0.4-rc1", "v0.0.4", ""):
            with self.subTest(version=version):
                self.assertEqual(rt.new_version_problems(version, []),
                                 [f"version {version!r} is not X.Y.Z"])


class CrateVersion(unittest.TestCase):
    def test_reads_the_package_version(self):
        self.assertEqual(rt.cargo_version(CARGO), "0.0.4")

    def test_the_fuzz_lock_agreeing_is_fine(self):
        self.assertEqual(rt.lock_problems("0.0.4", LOCK.format("0.0.4")), [])

    def test_the_fuzz_lock_disagreeing_is_a_problem(self):
        self.assertEqual(rt.lock_problems("0.0.4", LOCK.format("0.0.3")),
                         ["fuzz/Cargo.lock has stella-rain-core 0.0.3, Cargo.toml 0.0.4; "
                          "run cargo update -p stella-rain-core in fuzz/"])

    def test_a_fuzz_lock_without_the_crate_is_a_problem(self):
        self.assertEqual(rt.lock_problems("0.0.4", '[[package]]\nname = "x"\nversion = "1"\n'),
                         ["fuzz/Cargo.lock has no stella-rain-core"])


def run(name, conclusion="success", status="completed"):
    return {"name": name, "status": status, "conclusion": conclusion,
            "html_url": f"https://example.invalid/{name}"}


ALL_GREEN = [run(n) for n in rt.REQUIRED_CHECKS]


class Verify(unittest.TestCase):
    def test_every_required_check_green_passes(self):
        self.assertEqual(rt.verify(ALL_GREEN), [])

    def test_the_required_checks_are_the_rc_ruleset_and_version_is_new(self):
        self.assertEqual(len(rt.REQUIRED_CHECKS), 20)
        self.assertIn("the head is main", rt.REQUIRED_CHECKS)
        self.assertIn("version is new", rt.REQUIRED_CHECKS)
        self.assertIn("long fuzz / fuzz share_decode", rt.REQUIRED_CHECKS)
        self.assertNotIn("short fuzz run", rt.REQUIRED_CHECKS)

    def test_a_missing_check_is_a_problem(self):
        runs = [r for r in ALL_GREEN if r["name"] != "tests on arm64"]
        self.assertEqual(rt.verify(runs), ["tests on arm64: not reported"])

    def test_a_failed_or_skipped_check_is_a_problem(self):
        for conclusion in ("failure", "skipped", "cancelled"):
            with self.subTest(conclusion=conclusion):
                runs = [r for r in ALL_GREEN if r["name"] != "version is new"]
                runs.append(run("version is new", conclusion=conclusion))
                self.assertEqual(rt.verify(runs), [f"version is new: {conclusion}"])

    def test_a_rerun_that_succeeded_counts(self):
        runs = ALL_GREEN + [run("tests on arm64", conclusion="failure")]
        self.assertEqual(rt.verify(runs), [])


class Notes(unittest.TestCase):
    def test_notes_name_the_commit_the_checks_and_the_changes(self):
        text = rt.notes("v0.0.4", "abc123", "v0.0.3", "https://example.invalid/pr/80",
                        ALL_GREEN, ["d1 One change", "", "d2 Another"])
        self.assertTrue(text.startswith("# v0.0.4\n"))
        self.assertIn("Commit `abc123` on `rc`, from https://example.invalid/pr/80.", text)
        self.assertIn("- [the head is main](https://example.invalid/the head is main)", text)
        self.assertIn("Commits since v0.0.3:\n\n- d1 One change\n- d2 Another\n", text)

    def test_the_first_release_says_so(self):
        text = rt.notes("v0.0.1", "abc", None, "u", ALL_GREEN, [])
        self.assertIn("First release.", text)


class CommandLine(unittest.TestCase):
    def main(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            code = rt.main(["release_tag.py", *args])
        return code, out.getvalue()

    def files(self, d, lock_version="0.0.4"):
        cargo, lock = os.path.join(d, "Cargo.toml"), os.path.join(d, "Cargo.lock")
        with open(cargo, "w", encoding="utf-8") as f:
            f.write(CARGO)
        with open(lock, "w", encoding="utf-8") as f:
            f.write(LOCK.format(lock_version))
        return cargo, lock

    def test_tag_prints_the_new_tag(self):
        with tempfile.TemporaryDirectory() as d:
            cargo, lock = self.files(d)
            self.assertEqual(self.main("tag", cargo, lock, "v0.0.2", "v0.0.3"), (0, "v0.0.4\n"))

    def test_tag_fails_on_an_old_version_or_a_stale_lock(self):
        with tempfile.TemporaryDirectory() as d:
            cargo, lock = self.files(d)
            code, out = self.main("tag", cargo, lock, "v0.0.4")
            self.assertEqual((code, out), (1, "v0.0.4 is already tagged; bump the version in "
                                              "Cargo.toml\n"))
            cargo, lock = self.files(d, lock_version="0.0.3")
            self.assertEqual(self.main("tag", cargo, lock)[0], 1)

    def test_previous_prints_nothing_without_releases(self):
        self.assertEqual(self.main("previous"), (0, ""))
        self.assertEqual(self.main("previous", "v0.0.1", "v0.0.2"), (0, "v0.0.2\n"))

    def test_verify_exits_1_with_the_problems(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "checks.json")
            with open(path, "w", encoding="utf-8") as f:
                json.dump(ALL_GREEN[1:], f)
            code, out = self.main("verify", path)
        self.assertEqual((code, out), (1, f"{ALL_GREEN[0]['name']}: not reported\n"))

    def test_wrong_arguments_exit_2(self):
        self.assertEqual(self.main()[0], 2)
        self.assertEqual(self.main("tag", "only-one")[0], 2)


if __name__ == "__main__":
    unittest.main()
