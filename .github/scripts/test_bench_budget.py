"""Tests for bench_budget.py. Run: python3 -m unittest discover -s .github/scripts

The fixture has the shape of iai-callgrind 0.16 output for the three bench targets: a header per
benchmark (`<target>::<group>::<function>`), then its metrics, with `|` between the count and the
baseline's (`N/A` when there is none).
"""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import bench_budget as b  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(__file__), "bench_budget.py")

BLOCK = """{name}
  Instructions:                {ir}|{base}                 ({change})
  L1 Hits:                     {hits}|N/A                  (*********)
  LL Hits:                        3012|N/A                  (*********)
  RAM Hits:                        401|N/A                  (*********)
  Total read+write:            {hits}|N/A                  (*********)
  Estimated Cycles:            {hits}|N/A                  (*********)

"""


def block(name, ir, base="N/A", change="*********"):
    return BLOCK.format(name=name, ir=ir, base=base, change=change, hits=ir + 1000)


def output(worst=12_232_020, base="N/A"):
    return (
        block("worst_tick::worst_case_group::worst_tick", worst, base)
        + block("collide_tick::collide_group::collide_tick", 3_100_500)
        + block("collide_tick::collide_group::collide_tick_spread", 4_800_100)
        + block("time_limit_tick::time_limit_group::time_limit_tick", 9_900_000)
    )


BUDGETS = "[worst_tick]\narm64 = 12_400_000\nx86_64 = 11_900_000\n"


class Counts(unittest.TestCase):
    def test_finds_each_blocks_instructions(self):
        counts = b.counts(output())
        self.assertEqual(counts["worst_tick"], 12_232_020)
        self.assertEqual(counts["time_limit_tick"], 9_900_000)

    def test_a_baseline_column_does_not_change_the_count(self):
        counts = b.counts(output(12_300_000, base="12_232_020".replace("_", "")))
        self.assertEqual(counts["worst_tick"], 12_300_000)

    def test_the_largest_block_of_a_target_counts(self):
        self.assertEqual(b.counts(output())["collide_tick"], 4_800_100)


class Coloured(unittest.TestCase):
    """CARGO_TERM_COLOR=always makes the CI output carry ANSI escapes (seen in the bench job log)."""

    LOG = (
        "\x1b[32mworst_tick::worst_case_group::worst_tick\x1b[0m \x1b[36mworst_tick\x1b[0m\x1b[36m:\x1b[0m"
        "\x1b[1;34mworst_case :: warmed_up()\x1b[0m\n"
        "\x1b[90m  \x1b[0mInstructions:                    \x1b[1m14106310\x1b[0m|N/A"
        "                  (\x1b[90m*********\x1b[0m)\n"
    )

    def test_counts_through_ansi_escapes(self):
        self.assertEqual(b.counts(self.LOG), {"worst_tick": 14_106_310})


class Check(unittest.TestCase):
    def rows(self, arch, worst):
        return b.check(arch, b.counts(output(worst)), b.tomllib.loads(BUDGETS))

    def test_under_budget_passes(self):
        rows, errors = self.rows("arm64", 12_232_020)
        self.assertEqual(errors, [])
        self.assertEqual(rows, [("worst_tick", 12_232_020, 12_400_000)])

    def test_exactly_the_budget_passes(self):
        self.assertEqual(self.rows("arm64", 12_400_000)[1], [])

    def test_over_budget_names_bench_count_budget_and_overshoot(self):
        _, errors = self.rows("x86_64", 12_000_000)
        self.assertEqual(len(errors), 1)
        self.assertIn("worst_tick", errors[0])
        for text in ("12000000", "11900000", "100000"):
            self.assertIn(text, errors[0].replace(",", ""))

    def test_a_budgeted_bench_missing_from_the_output_fails(self):
        text = block("collide_tick::collide_group::collide_tick", 1)
        _, errors = b.check("arm64", b.counts(text), b.tomllib.loads(BUDGETS))
        self.assertEqual(len(errors), 1)
        self.assertIn("worst_tick", errors[0])
        self.assertIn("missing", errors[0])

    def test_an_unknown_arch_fails(self):
        _, errors = self.rows("riscv", 1)
        self.assertEqual(len(errors), 1)
        self.assertIn("riscv", errors[0])


class Cli(unittest.TestCase):
    def run_cli(self, arch, worst, summary=True, extra=()):
        with tempfile.TemporaryDirectory() as d:
            out, budgets, md = (os.path.join(d, n) for n in ("bench.txt", "budgets.toml", "sum.md"))
            for path, text in ((out, output(worst)), (budgets, BUDGETS)):
                with open(path, "w", encoding="utf-8") as f:
                    f.write(text)
            env = dict(os.environ)
            env.pop("GITHUB_STEP_SUMMARY", None)
            if summary:
                env["GITHUB_STEP_SUMMARY"] = md
            p = subprocess.run(
                [sys.executable, SCRIPT, "--arch", arch, "--bench-output", out, "--budgets", budgets, *extra],
                capture_output=True, text=True, env=env, check=False,
            )
            text = open(md, encoding="utf-8").read() if summary and os.path.exists(md) else ""
            return p, text

    def test_under_budget_exits_zero_and_writes_the_table(self):
        p, md = self.run_cli("arm64", 12_232_020)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("| worst_tick | 12,232,020 | 12,400,000 | 98.6% |", md)
        self.assertIn("| Bench | Instructions | Budget | % of budget |", md)

    def test_over_budget_exits_one_and_still_writes_the_table(self):
        p, md = self.run_cli("arm64", 12_500_000)
        self.assertEqual(p.returncode, 1)
        self.assertIn("worst_tick", p.stderr)
        self.assertIn("100,000", p.stderr)
        self.assertIn("100.8%", md)

    def test_warn_only_over_budget_exits_zero_with_a_warning_and_the_table(self):
        p, md = self.run_cli("arm64", 14_106_310, extra=["--warn-only"])
        self.assertEqual(p.returncode, 0, p.stderr)
        warning = [l for l in p.stdout.splitlines() if l.startswith("::warning::")]
        self.assertEqual(len(warning), 1, p.stdout)
        for text in ("worst_tick", "14,106,310", "12,400,000", "1,706,310"):
            self.assertIn(text, warning[0])
        self.assertIn("| worst_tick | 14,106,310 | 12,400,000 | 113.8% |", md)

    def test_warn_only_under_budget_prints_no_warning(self):
        p, _ = self.run_cli("arm64", 12_232_020, extra=["--warn-only"])
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertNotIn("::warning::", p.stdout)

    def test_warn_only_still_fails_on_an_unknown_arch(self):
        p, _ = self.run_cli("riscv", 1, extra=["--warn-only"])
        self.assertEqual(p.returncode, 1)

    def test_without_warn_only_over_budget_still_exits_one_and_does_not_warn(self):
        p, _ = self.run_cli("arm64", 14_106_310)
        self.assertEqual(p.returncode, 1)
        self.assertNotIn("::warning::", p.stdout)

    def test_no_summary_file_is_fine(self):
        p, _ = self.run_cli("x86_64", 11_737_172, summary=False)
        self.assertEqual(p.returncode, 0, p.stderr)

    def test_unknown_arch_exits_one(self):
        p, _ = self.run_cli("riscv", 1)
        self.assertEqual(p.returncode, 1)


if __name__ == "__main__":
    unittest.main()
