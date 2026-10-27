"""Tests for diff_size.py. Run: python3 -m unittest discover -s .github/scripts"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(__file__), "diff_size.py")


def git(repo, *args):
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
        cwd=repo, capture_output=True, text=True, check=True,
    ).stdout.strip()


def write(repo, path, text):
    full = os.path.join(repo, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as f:
        f.write(text)


def lines(n, word="x"):
    return "".join(f"{word}{i}\n" for i in range(n))


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = self.tmp.name
        git(self.repo, "init", "-q", "-b", "main")
        write(self.repo, ".gitattributes", "gen/** linguist-generated=true\n"
              "*.replay linguist-generated=true -diff\n")
        write(self.repo, "src/a.rs", lines(10))
        write(self.repo, "gen/s.json", lines(10))
        write(self.repo, "run.replay", lines(5))
        write(self.repo, "later.json", lines(4))
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "base")
        self.base = git(self.repo, "rev-parse", "HEAD")

    def tearDown(self):
        self.tmp.cleanup()

    def head(self):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "head")
        return git(self.repo, "rev-parse", "HEAD")

    def run_script(self, *args):
        return subprocess.run(
            [sys.executable, SCRIPT, *args], cwd=self.repo, capture_output=True, text=True
        )

    def counts(self, head):
        r = self.run_script(self.base, head, "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads(r.stdout)

    def test_generated_files_are_left_out_of_the_hand_written_count(self):
        write(self.repo, "src/a.rs", lines(9) + "new1\nnew2\nnew3\n")  # +3 -1
        write(self.repo, "gen/s.json", lines(110, "y"))  # +110 -10, generated
        write(self.repo, "run.replay", lines(50, "z"))  # -diff: no line count
        c = self.counts(self.head())
        self.assertEqual(c["hand_written"], {"additions": 3, "deletions": 1, "files": ["src/a.rs"]})
        self.assertEqual(c["generated"], {"files": ["gen/s.json", "run.replay"]})

    def test_the_text_line_says_both_counts(self):
        write(self.repo, "src/a.rs", lines(10) + "more\n")
        write(self.repo, "gen/s.json", lines(3))
        head = self.head()
        r = self.run_script(self.base, head)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(
            r.stdout.splitlines(),
            ["hand-written: +1 -0 in 1 file", "generated: 1 file (gen/s.json)"],
        )

    def test_the_text_line_names_five_generated_files_at_most(self):
        for i in range(7):
            write(self.repo, f"gen/f{i}.json", lines(2))
        r = self.run_script(self.base, self.head())
        self.assertEqual(
            r.stdout.splitlines()[1],
            "generated: 7 files (gen/f0.json, gen/f1.json, gen/f2.json, gen/f3.json, gen/f4.json,"
            " and 2 more)",
        )

    def test_the_head_attributes_decide(self):
        # A file the head newly marks generated is generated in that PR.
        write(self.repo, ".gitattributes", "gen/** linguist-generated=true\n"
              "*.replay linguist-generated=true -diff\nlater.json linguist-generated=true\n")
        write(self.repo, "later.json", lines(40, "w"))
        c = self.counts(self.head())
        self.assertEqual(c["generated"]["files"], ["later.json"])
        self.assertEqual(c["hand_written"]["files"], [".gitattributes"])

    def test_added_deleted_and_binary_files(self):
        write(self.repo, "src/new.rs", lines(7))  # +7
        os.remove(os.path.join(self.repo, "later.json"))  # -4
        with open(os.path.join(self.repo, "img.png"), "wb") as f:
            f.write(b"\x89PNG\x00\x01\x02")  # binary, hand-written: a file, no lines
        c = self.counts(self.head())
        self.assertEqual(
            c["hand_written"],
            {"additions": 7, "deletions": 4, "files": ["img.png", "later.json", "src/new.rs"]},
        )
        self.assertEqual(c["generated"], {"files": []})

    def test_a_bad_ref_fails(self):
        r = self.run_script(self.base, "no-such-ref")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("no-such-ref", r.stderr)


if __name__ == "__main__":
    unittest.main()
