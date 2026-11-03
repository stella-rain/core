"""Tests for bench_drop_guard.py and its step in rust.yml. Run: python3 -m unittest discover -s .github/scripts

The annotate text has the shape of `callgrind_annotate --tree=caller --show=Ir --threshold=100`
(valgrind 3.22) on a bench built with iai-callgrind 0.16.1: the callers of a function sit above it,
each line starting `<`, and the function itself starts `*`. The names are the ones seen on a bench
that took the `Run` by value and returned a number.
"""

import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SCRIPT = os.path.join(HERE, "bench_drop_guard.py")
BIN = "[/work/target/release/deps/worst_minute_peak-0abd7656d581959d]"
WRAPPER = "???:worst_minute_peak::worst_minute_peak::__iai_callgrind_wrapper_mod::worst_minute_peak"
RUN_DROP = "???:core::ptr::drop_glue::<stella_rain_core::bench_fixtures::worst_minute_case::Run>"
ENGINE_DROP = "???:core::ptr::drop_glue::<stella_rain_core::engine::Engine>"
CASE_DROP = "???:core::ptr::drop_glue::<stella_rain_core::bench_fixtures::Case>"
IN_PLACE = "???:core::ptr::drop_in_place::<stella_rain_core::engine::Engine>"
VEC_DROP = "???:core::ptr::drop_glue::<alloc::vec::Vec<u8>>"
STEP_DROP = "???:core::ptr::drop_glue::<stella_rain_core::sim::scratch::Scratch>"
STEP = "???:<stella_rain_core::engine::Engine>::step"


def block(callers, callee, ir=477537):
    """One caller-tree block: the `<` lines, then the `*` line."""
    lines = [f"{ir:,} ( 5.00%)  < {name} (1x) {BIN}" for name in callers]
    lines.append(f"{ir:,} ( 5.00%)  *  {callee} {BIN}")
    return "\n".join(lines) + "\n\n"


HEADER = "Profile data file 'x.out' (creator: callgrind-3.22.0)\nEvents shown:     Ir\n\n"
CLEAN = HEADER + block([WRAPPER], STEP, 437878) + block([STEP], STEP_DROP, 79) + block(["???:main"], WRAPPER, 15)


def load():
    sys.path.insert(0, HERE)
    try:
        import bench_drop_guard
    finally:
        sys.path.remove(HERE)
    return bench_drop_guard


class Hits(unittest.TestCase):
    def hits(self, text):
        return load().find_hits(text)

    def test_a_drop_of_the_run_called_from_the_wrapper_is_a_hit(self):
        self.assertEqual(self.hits(HEADER + block([WRAPPER], RUN_DROP)), [(RUN_DROP, WRAPPER)])

    def test_the_engine_the_case_and_drop_in_place_are_hits(self):
        for callee in (ENGINE_DROP, CASE_DROP, IN_PLACE):
            self.assertEqual(len(self.hits(HEADER + block([WRAPPER], callee))), 1, callee)

    def test_any_caller_of_the_block_counts(self):
        self.assertEqual(len(self.hits(HEADER + block(["???:main", WRAPPER], RUN_DROP))), 1)

    def test_the_same_callee_called_from_elsewhere_is_not_a_hit(self):
        self.assertEqual(self.hits(HEADER + block(["???:main"], RUN_DROP)), [])
        self.assertEqual(self.hits(HEADER + block([STEP], ENGINE_DROP)), [])

    def test_a_small_drop_under_the_wrapper_is_not_a_hit(self):
        self.assertEqual(self.hits(HEADER + block([WRAPPER], VEC_DROP)), [])

    def test_a_callee_is_not_a_hit_for_a_caller_of_the_next_block(self):
        text = HEADER + block(["???:main"], STEP) + block([STEP], RUN_DROP)
        self.assertEqual(self.hits(text), [])

    def test_the_clean_profile_has_no_hit(self):
        self.assertEqual(self.hits(CLEAN), [])

    def test_columns_do_not_matter(self):
        text = f"  9 <  {WRAPPER} (1x)\n  9 *  {RUN_DROP}\n"
        self.assertEqual(len(self.hits(text)), 1)


FAKE = """#!{python}
import sys
print(open(sys.argv[-1] + ".txt", encoding="utf-8").read())
"""


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = os.path.join(self.tmp.name, "iai")
        self.fake = os.path.join(self.tmp.name, "fake_annotate")
        with open(self.fake, "w") as f:
            f.write(FAKE.format(python=sys.executable))
        os.chmod(self.fake, os.stat(self.fake).st_mode | stat.S_IXUSR)

    def case(self, target, name, text):
        d = os.path.join(self.home, "stella-rain-core", target, f"{target}_group", f"{target}.{name}")
        os.makedirs(d)
        head = os.path.join(d, f"callgrind.{target}.{name}.out")
        with open(head, "w") as f:
            f.write("")
        if text is not None:
            with open(head + ".txt", "w", encoding="utf-8") as f:
                f.write(text)

    def run_cli(self, annotate=None):
        env = dict(os.environ, GITHUB_STEP_SUMMARY=os.path.join(self.tmp.name, "sum.md"))
        return subprocess.run(
            [sys.executable, SCRIPT, "--home", self.home, "--annotate", annotate or self.fake],
            capture_output=True, text=True, env=env, check=False,
        )

    def test_clean_cases_pass_and_are_counted(self):
        for name in ("a", "b", "c"):
            self.case("hit_tick", name, CLEAN)
        p = self.run_cli()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("checked 3 cases: no drop glue under the wrapper", p.stdout)

    def test_a_hit_in_one_case_of_three_fails_and_names_that_case(self):
        self.case("hit_tick", "a", CLEAN)
        self.case("worst_tick", "worst_tick", HEADER + block([WRAPPER], ENGINE_DROP))
        self.case("hit_tick", "c", CLEAN)
        p = self.run_cli()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        errors = [l for l in p.stdout.splitlines() if l.startswith("::error title=bench teardown::")]
        self.assertEqual(len(errors), 1, p.stdout)
        self.assertIn("worst_tick.worst_tick: ", errors[0])
        self.assertIn("is called from", errors[0])
        self.assertIn("forget_measured", errors[0])
        with open(os.path.join(self.tmp.name, "sum.md"), encoding="utf-8") as f:
            self.assertIn("worst_tick.worst_tick", f.read())

    def test_an_empty_home_fails_closed(self):
        os.makedirs(self.home)
        p = self.run_cli()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("no callgrind", p.stdout + p.stderr)

    def test_a_failing_annotate_fails_closed(self):
        self.case("hit_tick", "a", CLEAN)
        failing = os.path.join(self.tmp.name, "failing")
        with open(failing, "w") as f:
            f.write(f"#!{sys.executable}\nimport sys\nsys.exit(1)\n")
        os.chmod(failing, 0o755)
        p = self.run_cli(annotate=failing)
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)

    def test_a_text_that_never_names_the_wrapper_fails_closed(self):
        self.case("hit_tick", "a", HEADER + block([STEP], STEP_DROP, 79))
        p = self.run_cli()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("format this guard does not know", p.stdout)

    def test_the_annotate_error_is_reported(self):
        self.case("hit_tick", "a", CLEAN)
        failing = os.path.join(self.tmp.name, "failing")
        with open(failing, "w") as f:
            f.write(f"#!{sys.executable}\nimport sys\nsys.stderr.write('perl is missing')\nsys.exit(2)\n")
        os.chmod(failing, 0o755)
        p = self.run_cli(annotate=failing)
        self.assertIn("perl is missing", p.stdout)

    def test_a_missing_annotate_fails_closed(self):
        self.case("hit_tick", "a", CLEAN)
        p = self.run_cli(annotate=os.path.join(self.tmp.name, "nope"))
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)


def read(*parts):
    with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
        return f.read()


def bench_job():
    text = read(".github", "workflows", "rust.yml")
    return re.search(r"^  bench:\n((?:(?:   .*)?\n)*)", text, re.M).group(1)


class WorkflowStep(unittest.TestCase):
    NAME = "Check the teardown of the benches"

    def setUp(self):
        self.job = bench_job()
        m = re.search(rf"^      - name: {self.NAME}\n((?:(?!      - ).*\n)*)", self.job, re.M)
        self.assertTrue(m, "no step in job bench")
        self.step = m.group(0)

    def names(self):
        return re.findall(r"^      - name: (.*)$", self.job, re.M)

    def test_it_sits_between_the_budget_check_and_the_rise_check(self):
        names = self.names()
        self.assertEqual(
            names[names.index("Check the tick budgets") + 1 : names.index("Check for a rise")], [self.NAME]
        )

    def test_it_runs_after_the_count_even_when_an_earlier_check_failed(self):
        self.assertIn("        if: ${{ !cancelled() && steps.count.outcome == 'success' }}", self.step)

    def test_it_runs_the_script_on_the_home(self):
        self.assertIn(
            '        run: python3 .github/scripts/bench_drop_guard.py --home "$IAI_CALLGRIND_HOME"', self.step
        )

    def test_it_cannot_be_skipped_and_cites_its_rules(self):
        self.assertNotIn("continue-on-error", self.step)
        before = self.job[: self.job.index(self.step)]
        comment = before.rsplit("      - name:", 1)[-1]
        self.assertIn("ADR-SIM-6", comment)
        self.assertIn("tests/bench_teardown.rs", comment)


if __name__ == "__main__":
    unittest.main()
