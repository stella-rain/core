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
from unittest import mock

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

    def test_a_lone_saved_baseline_is_the_head(self):
        # A push to main runs with --save-baseline=main: iai writes no plain `.out`.
        with tempfile.TemporaryDirectory() as home:
            d = os.path.join(home, "stella-rain-core", "hit_tick", "hit_group", "hit_tick.hit_tick_spread")
            os.makedirs(d)
            name = "callgrind.hit_tick.hit_tick_spread.out.base@main"
            open(os.path.join(d, name), "w").close()
            found = c.find_cases(home)
            self.assertEqual(found, [("hit_tick.hit_tick_spread", os.path.join(d, name), None)])

    def test_a_baseline_beside_its_out_is_not_a_second_head(self):
        with tempfile.TemporaryDirectory() as home:
            both = os.path.join(home, "stella-rain-core", "t", "g", "t.both")
            lone = os.path.join(home, "stella-rain-core", "t", "g", "t.lone")
            os.makedirs(both)
            os.makedirs(lone)
            for d, name in ((both, "callgrind.t.both.out"), (both, "callgrind.t.both.out.base@main"),
                            (lone, "callgrind.t.lone.out.base@main")):
                open(os.path.join(d, name), "w").close()
            found = c.find_cases(home)
            self.assertEqual([(l, os.path.basename(h), b and os.path.basename(b)) for l, h, b in found], [
                ("t.both", "callgrind.t.both.out", "callgrind.t.both.out.base@main"),
                ("t.lone", "callgrind.t.lone.out.base@main", None),
            ])

    def test_an_empty_home_has_no_case(self):
        with tempfile.TemporaryDirectory() as home:
            self.assertEqual(c.find_cases(home), [])


FAKE = """#!{python}
import sys
path = sys.argv[-1]
print(open(path).read())
"""


class CliBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = os.path.join(self.tmp.name, "iai")
        self.out = os.path.join(self.tmp.name, "out")
        self.fake = os.path.join(self.tmp.name, "fake_annotate")
        with open(self.fake, "w") as f:
            f.write(FAKE.format(python=sys.executable))
        os.chmod(self.fake, os.stat(self.fake).st_mode | stat.S_IXUSR)

    def run_cli(self, *extra, annotate=None):
        env = dict(os.environ, GITHUB_STEP_SUMMARY=os.path.join(self.tmp.name, "sum.md"))
        return subprocess.run(
            [sys.executable, SCRIPT, "--home", self.home, "--out", self.out,
             "--annotate", annotate or self.fake, "--arch", "x86_64", *extra],
            capture_output=True, text=True, env=env, check=False,
        )


class Cli(CliBase):
    def case(self, base=True):
        d = os.path.join(self.home, "stella-rain-core", "worst_tick", "g", "worst_tick.worst_tick")
        os.makedirs(d)
        with open(os.path.join(d, "callgrind.worst_tick.worst_tick.out"), "w") as f:
            f.write(HEAD)
        if base:
            with open(os.path.join(d, "callgrind.worst_tick.worst_tick.out.base@main"), "w") as f:
                f.write(BASE)

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

P = "stella_rain_core::sim::schedule::steps::"

STEP_BASE = """900 PROGRAM TOTALS

  500  ???:""" + P + """run_attacks [/t/base/bench]
  200  ???:""" + P + """advance_tick::h0123abcd [/t/base/bench]
   50  ???:""" + P + """old_step [/t/base/bench]
  900  ???:other_fn [/t/base/bench]
"""

STEP_HEAD = """950 PROGRAM TOTALS

  550  ???:""" + P + """run_attacks [/t/head/bench]
  200  ???:""" + P + """advance_tick [/t/head/bench]
  900  ???:other_fn [/t/head/bench]
"""

SCHEDULE = """pub const TICK: &[Step] = steps![spawn_boss, advance_tick,
    run_attacks];
"""

class Steps(unittest.TestCase):
    def test_step_rows_are_cut_at_the_first_non_identifier_character(self):
        _, rows = c.parse_annotate(STEP_BASE)
        self.assertEqual(c.parse_steps(rows), {"run_attacks": 500, "advance_tick": 200, "old_step": 50})
        _, rows = c.parse_annotate("1 PROGRAM TOTALS\n 7  <f.rs>:" + P + "x::h0123abcd\n 4  ???:x\n")
        self.assertEqual(c.parse_steps(rows), {"x": 7})

    def test_a_step_of_the_frozen_rules_is_counted_as_the_same_step(self):
        frozen = P.replace("::sim::", "::sim_v1::")
        text = ("1 PROGRAM TOTALS\n 60  ???:" + frozen + "run_attacks\n 40  ???:" + P
                + "run_attacks\n 9  ???:" + frozen + "advance_tick::h0123abcd\n 5  ???:" + frozen + "a::inner\n")
        _, rows = c.parse_annotate(text)
        self.assertEqual(c.parse_steps(rows), {"run_attacks": 100, "advance_tick": 9})
        self.assertFalse(c.steps_unreliable(rows, 200))
        self.assertTrue(c.steps_unreliable(rows, 100))

    def test_closures_and_helpers_inside_a_step_are_not_added_to_it(self):
        text = ("1 PROGRAM TOTALS\n 60  ???:" + P + "a\n 50  ???:" + P + "a::inner\n 30  ???:" + P
                + "a::{{closure}}\n 5  ???:" + P + "a::h0123abcd\n")
        _, rows = c.parse_annotate(text)
        self.assertEqual(c.parse_steps(rows), {"a": 65})

    def test_order_comes_from_steps_macro(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "schedule.rs")
            with open(path, "w") as f:
                f.write(SCHEDULE)
            order = c.read_order(path)
            self.assertEqual(order, ["spawn_boss", "advance_tick", "run_attacks"])
            self.assertEqual(c.order_steps({"run_attacks", "old_step", "advance_tick"}, order),
                             ["advance_tick", "run_attacks", "old_step"])
            self.assertIsNone(c.read_order(os.path.join(d, "missing.rs")))
            with open(path, "w") as f:
                f.write("fn nothing() {}")
            self.assertIsNone(c.read_order(path))
        self.assertEqual(c.order_steps({"b", "a"}, None), ["a", "b"])

    def test_the_schedule_default_is_the_sim_file(self):
        class Stop(Exception):
            pass

        seen = []

        def order(path):
            seen.append(path)
            raise Stop

        with tempfile.TemporaryDirectory() as d, \
                mock.patch.object(sys, "argv", ["callgrind_diff.py", "--home", d, "--out", d]), \
                mock.patch.object(c, "find_cases", return_value=[("t", "h", "b")]), \
                mock.patch.object(c, "read_order", side_effect=order):
            with self.assertRaises(Stop):
                c.main()
        self.assertEqual(seen, ["src/sim/schedule.rs"])
        root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
        self.assertTrue(os.path.isfile(os.path.join(root, seen[0])))

    def test_rest_is_the_total_minus_the_steps_and_only_changed_rows_show(self):
        base = (900, {"run_attacks": 500, "advance_tick": 200, "old_step": 50})
        head = (950, {"run_attacks": 550, "advance_tick": 200})
        rows = c.step_table(base, head, ["advance_tick", "run_attacks"])
        self.assertEqual(rows, [("advance_tick", 200, 200), ("run_attacks", 500, 550),
                                ("old_step", 50, 0), ("rest", 150, 200)])
        self.assertEqual(c.step_lines(rows, True)[0], "step base head delta")
        self.assertIn("run_attacks 500 550 +50", c.step_lines(rows, True))
        self.assertEqual([r[0] for r in c.changed(rows)], ["run_attacks", "old_step", "rest"])
        self.assertEqual(c.step_lines(c.step_table(None, head, None), False)[0], "step head")


class StepRecursion(unittest.TestCase):
    def test_a_recursion_suffix_does_not_hide_a_step_frame(self):
        name = "???:" + P + "run_attacks'2"
        self.assertEqual(c.parse_steps({name: 5}), {"run_attacks": 5})


class StepsCli(CliBase):
    def case_with(self, head, base, label="worst_tick"):
        d = os.path.join(self.home, "stella-rain-core", label, "g", f"{label}.{label}")
        os.makedirs(d)
        for suffix, text in ((".out", head), (".out.base@main", base)):
            with open(os.path.join(d, f"callgrind.{label}.{label}{suffix}"), "w") as f:
                f.write(text)

    def schedule(self):
        path = os.path.join(self.tmp.name, "schedule.rs")
        with open(path, "w") as f:
            f.write(SCHEDULE)
        return path

    def summary(self):
        return open(os.path.join(self.tmp.name, "sum.md")).read()

    def test_steps_are_written_and_summarised_and_no_allocation_line_is(self):
        self.case_with(STEP_HEAD, STEP_BASE)
        p = self.run_cli("--schedule", self.schedule())
        self.assertEqual(p.returncode, 0, p.stderr)
        steps = open(os.path.join(self.out, "worst_tick.worst_tick.steps.txt")).read()
        self.assertIn("advance_tick 200 200 +0", steps)
        self.assertIn("run_attacks 500 550 +50", steps)
        self.assertIn("rest 150 200 +50", steps)
        self.assertFalse(any(n.endswith(".alloc.txt") for n in os.listdir(self.out)))
        summary = self.summary()
        self.assertIn("run_attacks 500 550 +50", summary)
        self.assertNotIn("advance_tick 200 200", summary)
        self.assertNotIn("Allocator calls", summary)
        self.assertEqual(p.stdout.count("::warning::"), 0)

    def test_no_step_frames_writes_no_steps_file_and_notes_it(self):
        self.case_with(HEAD, BASE)
        p = self.run_cli("--schedule", self.schedule())
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertFalse(any(n.endswith(".steps.txt") for n in os.listdir(self.out)))
        self.assertEqual(self.summary().count("no named tick steps in the profile (BS2a)"), 1)

    def test_nested_step_frames_print_the_unreliable_note_instead_of_rows(self):
        nested = STEP_HEAD.replace("950 PROGRAM", "1,000 PROGRAM").replace(
            "run_attacks [", "run_attacks'2 [").replace("  200  ???:" + P + "advance_tick", "  900  ???:" + P + "advance_tick")
        self.case_with(nested, STEP_BASE)
        p = self.run_cli("--schedule", self.schedule(), "--arch", "arm64")
        self.assertEqual(p.returncode, 0, p.stderr)
        steps = open(os.path.join(self.out, "worst_tick.worst_tick.steps.txt")).read()
        self.assertEqual(steps, "step table unreliable on arm64 (nested frames)\n")
        summary = self.summary()
        self.assertIn("step table unreliable on arm64 (nested frames)", summary)
        self.assertNotIn("run_attacks 500", summary)
        self.assertEqual(summary.count("no named tick steps"), 0)


class Unreliable(unittest.TestCase):
    def test_a_suffixed_step_or_a_sum_over_the_total_is_unreliable(self):
        ok = {"???:" + P + "a": 400, "???:" + P + "b": 500}
        self.assertFalse(c.steps_unreliable(ok, 1000))
        self.assertTrue(c.steps_unreliable({"???:" + P + "a'2": 10}, 1000))
        self.assertTrue(c.steps_unreliable({"???:" + P + "a": 600, "???:" + P + "b": 500}, 1000))
        self.assertFalse(c.steps_unreliable({"???:" + P + "a": 600, "???:" + P + "b": 405}, 1000))  # within 1%
        self.assertFalse(c.steps_unreliable({"???:other'2": 10}, 1000))


if __name__ == "__main__":
    unittest.main()
