"""Tests for fuzz_seconds.py. Run: python3 -m unittest discover -s .github/scripts

fuzz-long.yml hands the `minutes` input to the script as text: a dispatch of 1 arrives as `1.0`,
Candidate's 15 as `15`, a scheduled run's default as `7`.
"""

import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "fuzz_seconds.py")
sys.path.insert(0, HERE)

NOT_A_NUMBER = ["", "abc", "-1", "+5", "1,5", "1e3", "nan", "inf", "1.", ".5", "7 minutes"]
TOO_SMALL = ["0", "0.0", "0.0083"]


def cli(*args):
    return subprocess.run([sys.executable, SCRIPT, *args], capture_output=True, text=True, check=False)


class Seconds(unittest.TestCase):
    def test_values(self):
        from fuzz_seconds import seconds

        for text, want in [
            ("1.0", 60), ("1", 60), ("7", 420), ("15", 900), ("15.0", 900),
            ("0.5", 30), ("1.25", 75), (" 7 ", 420), ("0.0084", 1),
        ]:
            with self.subTest(text=text):
                got = seconds(text)
                self.assertIsInstance(got, int)
                self.assertEqual(got, want)

    def test_refused(self):
        from fuzz_seconds import seconds

        for text, why in [(t, "not a number") for t in NOT_A_NUMBER] + [
            (t, "must be at least 1 second") for t in TOO_SMALL
        ]:
            with self.subTest(text=text):
                with self.assertRaises(ValueError) as cm:
                    seconds(text)
                self.assertIn(why, str(cm.exception))
                self.assertIn(repr(text), str(cm.exception))


class Cli(unittest.TestCase):
    def test_prints_seconds(self):
        r = cli("1.0")
        self.assertEqual((r.returncode, r.stdout), (0, "60\n"))

    def test_triggers_agree(self):
        for text, want in [("7", "420\n"), ("15", "900\n"), ("1.0", "60\n"), ("7.0", "420\n")]:
            with self.subTest(text=text):
                self.assertEqual(cli(text).stdout, want)

    def test_bad_value(self):
        for text in NOT_A_NUMBER + TOO_SMALL:
            with self.subTest(text=text):
                r = cli(text)
                self.assertEqual((r.returncode, r.stdout), (1, ""))
                self.assertIn("fuzz_seconds.py:", r.stderr)
                self.assertIn(repr(text), r.stderr)

    def test_usage(self):
        for args in [(), ("1", "2")]:
            with self.subTest(args=args):
                r = cli(*args)
                self.assertEqual((r.returncode, r.stdout), (1, ""))
                self.assertIn("usage", r.stderr)


if __name__ == "__main__":
    unittest.main()
