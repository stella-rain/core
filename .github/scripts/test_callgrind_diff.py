"""Tests for callgrind_diff.py. Run: python3 -m unittest discover -s .github/scripts

The annotate fixtures have the shape of `callgrind_annotate --show=Ir --show-percs=no` output on a
bench profile: symbols without debuginfo, so names read `???:<fn>` or `<file>:<fn>`, then ` [<binary>]`.
"""

import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import callgrind_diff as c  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(__file__), "callgrind_diff.py")

BASE = """Profile data file 'callgrind.x.out.base@main'
--------------------------------------------------------------------------------
Ir
--------------------------------------------------------------------------------
1,500 PROGRAM TOTALS

--------------------------------------------------------------------------------
Ir  file:function
--------------------------------------------------------------------------------
1,000  ???:fn_a [/t/base/bench]
  500  ???:fn_b [/t/base/bench]
"""

HEAD = """Profile data file 'callgrind.x.out'
--------------------------------------------------------------------------------
Ir
--------------------------------------------------------------------------------
1,850 PROGRAM TOTALS

--------------------------------------------------------------------------------
Ir  file:function
--------------------------------------------------------------------------------
1,000  ???:fn_a [/t/head/bench]
  800  ???:fn_b [/t/head/bench]
   50  ???:fn_c [/t/head/bench]
"""


class Parse(unittest.TestCase):
    def test_total_and_functions_with_the_binary_suffix_stripped(self):
        total, rows = c.parse_annotate(BASE)
        self.assertEqual(total, 1_500)
        self.assertEqual(rows, {"???:fn_a": 1_000, "???:fn_b": 500})

    def test_a_duplicate_name_is_summed(self):
        text = "300 PROGRAM TOTALS\n 100  ???:f [/a/bench]\n 200  ???:f [/b/bench]\n"
        self.assertEqual(c.parse_annotate(text)[1], {"???:f": 300})

    def test_lines_without_a_colon_are_not_functions(self):
        text = "9 PROGRAM TOTALS\n 5  some words here\n 4  ???:f\n"
        self.assertEqual(c.parse_annotate(text)[1], {"???:f": 4})


class Diff(unittest.TestCase):
    def lines(self, base, head, top=40):
        return c.diff_lines(c.parse_annotate(base), c.parse_annotate(head), top)

    def test_changed_functions_only_with_total(self):
        lines = self.lines(BASE, HEAD)
        self.assertEqual(lines[0], "total: base 1,500 head 1,850 (+23.3%)")
        rows = [l for l in lines if "fn_" in l]
        self.assertEqual(len(rows), 2)
        self.assertTrue(rows[0].startswith("+300 500 800 ???:fn_b"), rows)
        self.assertTrue(rows[1].startswith("+50 0 50 ???:fn_c"), rows)
        self.assertFalse(any("fn_a" in l for l in lines))

    def test_ties_sort_by_name(self):
        base = "10 PROGRAM TOTALS\n"
        head = "30 PROGRAM TOTALS\n 10  ???:zz\n 10  ???:aa\n 10  ???:mm\n"
        rows = self.lines(base, head)[2:]
        self.assertEqual([r.split()[-1] for r in rows], ["???:aa", "???:mm", "???:zz"])

    def test_top_limits_the_rows(self):
        self.assertEqual(len([l for l in self.lines(BASE, HEAD, top=1) if "fn_" in l]), 1)

    def test_a_long_name_is_cut_to_200_characters(self):
        head = "5 PROGRAM TOTALS\n 5  ???:" + "x" * 300 + "\n"
        row = self.lines("0 PROGRAM TOTALS\n", head)[2]
        self.assertEqual(len(row.split(" ", 3)[3]), 200)

    def test_no_base_lists_the_head_only(self):
        lines = c.diff_lines(None, c.parse_annotate(HEAD), 40)
        self.assertIn("no base", lines[0])
        self.assertIn("1,850", lines[0])
        self.assertTrue(any("fn_a" in l for l in lines))


class Cases(unittest.TestCase):
    def test_old_is_ignored_and_a_missing_base_is_none(self):
        with tempfile.TemporaryDirectory() as home:
            d = os.path.join(home, "stella-rain-core", "hit_tick", "hit_group", "hit_tick.hit_tick_spread")
            os.makedirs(d)
            for name in ("callgrind.hit_tick.hit_tick_spread.out",
                         "callgrind.hit_tick.hit_tick_spread.out.old"):
                open(os.path.join(d, name), "w").close()
            found = c.find_cases(home)
            self.assertEqual(len(found), 1)
            label, head, base = found[0]
            self.assertEqual(label, "hit_tick.hit_tick_spread")
            self.assertTrue(head.endswith(".out"))
            self.assertIsNone(base)
            open(os.path.join(d, "callgrind.hit_tick.hit_tick_spread.out.base@main"), "w").close()
            self.assertTrue(c.find_cases(home)[0][2].endswith(".out.base@main"))


FAKE = """#!{python}
import sys
path = sys.argv[-1]
print(open(path).read())
"""


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = os.path.join(self.tmp.name, "iai")
        self.out = os.path.join(self.tmp.name, "out")
        self.fake = os.path.join(self.tmp.name, "fake_annotate")
        with open(self.fake, "w") as f:
            f.write(FAKE.format(python=sys.executable))
        os.chmod(self.fake, os.stat(self.fake).st_mode | stat.S_IXUSR)

    def case(self, base=True):
        d = os.path.join(self.home, "stella-rain-core", "worst_tick", "g", "worst_tick.worst_tick")
        os.makedirs(d)
        with open(os.path.join(d, "callgrind.worst_tick.worst_tick.out"), "w") as f:
            f.write(HEAD)
        if base:
            with open(os.path.join(d, "callgrind.worst_tick.worst_tick.out.base@main"), "w") as f:
                f.write(BASE)

    def run_cli(self, *extra, annotate=None):
        env = dict(os.environ, GITHUB_STEP_SUMMARY=os.path.join(self.tmp.name, "sum.md"))
        return subprocess.run(
            [sys.executable, SCRIPT, "--home", self.home, "--out", self.out,
             "--annotate", annotate or self.fake, "--arch", "x86_64", *extra],
            capture_output=True, text=True, env=env, check=False,
        )

    def test_writes_the_files_and_the_summary(self):
        self.case()
        p = self.run_cli()
        self.assertEqual(p.returncode, 0, p.stderr)
        names = set(os.listdir(self.out))
        for side in ("base", "head"):
            for kind in ("exclusive", "inclusive"):
                self.assertIn(f"worst_tick.worst_tick.{side}.{kind}.txt", names)
        self.assertIn("worst_tick.worst_tick.diff.txt", names)
        self.assertIn("worst_tick.worst_tick.diff-inclusive.txt", names)
        diff = open(os.path.join(self.out, "worst_tick.worst_tick.diff.txt")).read()
        self.assertIn("+300 500 800 ???:fn_b", diff)
        summary = open(os.path.join(self.tmp.name, "sum.md")).read()
        self.assertIn("### Profile on x86_64", summary)
        self.assertIn("fn_b", summary)
        self.assertIn("bench-profile-x86_64", summary)

    def test_no_base_says_so_and_exits_zero(self):
        self.case(base=False)
        p = self.run_cli()
        self.assertEqual(p.returncode, 0, p.stderr)
        diff = open(os.path.join(self.out, "worst_tick.worst_tick.diff.txt")).read()
        self.assertIn("no base", diff)

    def test_a_failing_annotate_skips_the_case_with_a_warning(self):
        self.case()
        failing = os.path.join(self.tmp.name, "failing_annotate")
        with open(failing, "w") as f:
            f.write(f"#!{sys.executable}\nimport sys\nsys.exit(3)\n")
        os.chmod(failing, 0o755)
        p = self.run_cli(annotate=failing)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("::warning::", p.stdout)
        self.assertIn("worst_tick.worst_tick", p.stdout)

    def test_a_missing_annotate_skips_with_a_warning(self):
        self.case()
        p = self.run_cli(annotate=os.path.join(self.tmp.name, "nope"))
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("::warning::", p.stdout)

    def test_a_tiny_change_still_counts_as_moved(self):
        self.case()
        bigger = BASE.replace("1,500 PROGRAM", "1,000,000 PROGRAM")
        head = HEAD.replace("1,850 PROGRAM", "1,000,001 PROGRAM")
        d = os.path.join(self.home, "stella-rain-core", "worst_tick", "g", "worst_tick.worst_tick")
        open(os.path.join(d, "callgrind.worst_tick.worst_tick.out.base@main"), "w").write(bigger)
        open(os.path.join(d, "callgrind.worst_tick.worst_tick.out"), "w").write(head)
        self.assertEqual(self.run_cli().returncode, 0)
        summary = open(os.path.join(self.tmp.name, "sum.md")).read()
        self.assertIn("**worst_tick.worst_tick**", summary)

    def test_an_empty_home_exits_one_naming_it(self):
        os.makedirs(self.home)
        p = self.run_cli()
        self.assertEqual(p.returncode, 1)
        self.assertIn(self.home, p.stderr)

    @unittest.skipUnless(shutil.which("callgrind_annotate"), "callgrind_annotate not installed")
    def test_the_real_callgrind_annotate_reads_a_minimal_out_file(self):
        out = (
            "version: 1\ncreator: test\npositions: line\nevents: Ir\nsummary: 30\n\n"
            "fl=(1) a.rs\nfn=(1) fn_a\n1 10\n2 20\n\ntotals: 30\n"
        )
        d = os.path.join(self.home, "stella-rain-core", "t", "g", "f.c")
        os.makedirs(d)
        with open(os.path.join(d, "callgrind.f.c.out"), "w") as f:
            f.write(out)
        p = self.run_cli(annotate="callgrind_annotate")
        self.assertEqual(p.returncode, 0, p.stderr)
        diff = open(os.path.join(self.out, "t.c.diff.txt")).read()
        self.assertIn("fn_a", diff)


if __name__ == "__main__":
    unittest.main()
