"""Tests for bench_pins.py. Run: python3 -m unittest discover -s .github/scripts"""

import os
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(__file__), "bench_pins.py")

NOTICE = "::notice::the base has no tests/bench_pins.toml: nothing compared"


def pins(**cases):
    """A pin file; each case is (before, after, stage, input) under the bench `t`."""
    out = ""
    for name, (before, after, stage, inp) in cases.items():
        out += (
            f'[{name}]\nbench = "t"\nbefore = "{before}"\nafter = "{after}"\n'
            f'stage = "{stage}"\ninput = "{inp}"\n\n'
        )
    return out


A = ("b" * 16, "a" * 16, "5" * 16, "1" * 16)
B = ("c" * 16, "d" * 16, "6" * 16, "2" * 16)


class Cli(unittest.TestCase):
    def run_script(self, base, head, bench="t", changed="false"):
        """(exit code, stdout, stderr, GITHUB_OUTPUT lines); base None = no file."""
        with tempfile.TemporaryDirectory() as d:
            head_path = os.path.join(d, "head.toml")
            with open(head_path, "w", encoding="utf-8") as f:
                f.write(head)
            base_path = os.path.join(d, "base.toml")
            if base is not None:
                with open(base_path, "w", encoding="utf-8") as f:
                    f.write(base)
            out_path = os.path.join(d, "out")
            env = dict(os.environ, GITHUB_OUTPUT=out_path)
            r = subprocess.run(
                [sys.executable, SCRIPT, "--base", base_path, "--head", head_path,
                 "--bench", bench, "--bench-changed", changed],
                capture_output=True, text=True, env=env,
            )
            written = []
            if os.path.exists(out_path):
                with open(out_path, encoding="utf-8") as f:
                    written = f.read().splitlines()
            return r.returncode, r.stdout.splitlines(), r.stderr, written

    def test_equal_digests_are_compared(self):
        code, out, _, written = self.run_script(pins(x=A), pins(x=A))
        self.assertEqual((code, out, written), (0, ["x: compared"], ["compare=true", "cases=x"]))

    def test_stage_differs_is_moved(self):
        head = (A[0], A[1], B[2], A[3])
        code, out, _, written = self.run_script(pins(x=A), pins(x=head))
        self.assertEqual(out, ["x: case moved (stage differ): not compared"])
        self.assertEqual((code, written), (0, ["compare=false", "cases="]))

    def test_before_differs_is_moved(self):
        head = (B[0], A[1], A[2], A[3])
        _, out, _, _ = self.run_script(pins(x=A), pins(x=head))
        self.assertEqual(out, ["x: case moved (before differ): not compared"])

    def test_input_differs_is_moved(self):
        head = (A[0], A[1], A[2], B[3])
        _, out, _, _ = self.run_script(pins(x=A), pins(x=head))
        self.assertEqual(out, ["x: case moved (input differ): not compared"])

    def test_several_digests_are_all_named(self):
        _, out, _, _ = self.run_script(pins(x=A), pins(x=B))
        self.assertEqual(out, ["x: case moved (before|stage|input differ): not compared"])

    def test_after_differs_only_is_compared(self):
        head = (A[0], B[1], A[2], A[3])
        code, out, _, written = self.run_script(pins(x=A), pins(x=head))
        self.assertEqual((code, out, written), (0, ["x: compared"], ["compare=true", "cases=x"]))

    def test_a_case_only_in_head_is_new(self):
        code, out, _, written = self.run_script(pins(x=A), pins(x=A, y=B))
        self.assertEqual(out, ["x: compared", "y: new case: not compared"])
        self.assertEqual((code, written), (0, ["compare=true", "cases=x"]))

    def test_bench_file_changed_compares_nothing(self):
        code, out, _, written = self.run_script(pins(x=A), pins(x=A), changed="true")
        self.assertEqual(out, ["x: bench changed: not compared"])
        self.assertEqual((code, written), (0, ["compare=false", "cases="]))

    def test_no_base_file_is_compare_false_with_the_notice(self):
        code, out, _, written = self.run_script(None, pins(x=A))
        self.assertEqual((code, out, written), (0, [NOTICE], ["compare=false", "cases="]))

    def test_an_empty_base_file_is_the_same(self):
        code, out, _, written = self.run_script("", pins(x=A))
        self.assertEqual((code, out, written), (0, [NOTICE], ["compare=false", "cases="]))

    def test_head_missing_a_case_of_the_target_fails(self):
        code, _, err, _ = self.run_script(pins(x=A, y=B), pins(x=A))
        self.assertEqual(code, 1)
        self.assertIn("head tests/bench_pins.toml lacks the case y of the target t", err)

    def test_a_malformed_base_file_fails_not_silently_skips(self):
        code, _, err, _ = self.run_script("[x\nbench =", pins(x=A))
        self.assertEqual(code, 1)
        self.assertIn("not valid TOML", err)

    def test_a_removed_case_with_a_changed_bench_file_is_allowed(self):
        code, _, _, written = self.run_script(pins(x=A, y=B), pins(x=A), changed="true")
        self.assertEqual((code, written), (0, ["compare=false", "cases="]))

    def test_head_without_any_case_of_the_target_fails(self):
        code, _, err, _ = self.run_script(pins(x=A), pins(x=A), bench="other")
        self.assertEqual(code, 1)
        self.assertIn("other", err)

    def test_other_targets_cases_are_ignored(self):
        other = '[z]\nbench = "o"\nbefore = "0"\nafter = "0"\nstage = "0"\ninput = "0"\n\n'
        code, out, _, written = self.run_script(pins(x=A) + other, pins(x=A))
        self.assertEqual((code, out, written), (0, ["x: compared"], ["compare=true", "cases=x"]))

    def test_cases_are_listed_in_head_order(self):
        _, _, _, written = self.run_script(pins(x=A, y=A), pins(x=A, y=A))
        self.assertEqual(written, ["compare=true", "cases=x,y"])


if __name__ == "__main__":
    unittest.main()
