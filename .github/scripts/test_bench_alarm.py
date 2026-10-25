"""Tests for bench_alarm.py. Run: python3 -m unittest discover -s .github/scripts"""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import bench_alarm as a  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(__file__), "bench_alarm.py")

BUDGETS = "[worst_tick]\narm64 = 1\nx86_64 = 1\n[time_limit_tick]\narm64 = 1\nx86_64 = 1\n"

BLOCK = (
    "\x1b[32mtime_limit_tick::time_limit_group::time_limit_tick\x1b[0m "
    "\x1b[36mtime_limit_tick\x1b[0m:\x1b[1;34mtime_limit :: run()\x1b[0m\n"
    "  Instructions:            \x1b[1m{head}\x1b[0m|{base}   (\x1b[31m+1.0%\x1b[0m)\n"
    "  L1 Hits:                 5|N/A   (*********)\n\n"
)


def output(head, base="1600046"):
    return BLOCK.format(head=head, base=base)


def entry(**kw):
    e = dict(bench="time_limit_tick", case="time_limit_tick", arch="arm64",
             base=1_600_046, count=1_693_000, reason="a new rule", pr=400)
    e.update(kw)
    return "[[accepted]]\n" + "".join(
        f'{k} = "{v}"\n' if isinstance(v, str) else f"{k} = {v}\n" for k, v in e.items()
    )


class Cases(unittest.TestCase):
    def test_parses_through_ansi_with_case_and_base(self):
        self.assertEqual(a.cases(output(1_632_046)), [("time_limit_tick", "time_limit_tick", 1_632_046, 1_600_046)])

    def test_na_base_is_none(self):
        self.assertIsNone(a.cases(output(5, base="N/A"))[0][3])


class Rise(unittest.TestCase):
    def test_boundary_at_two_percent_both_sides(self):
        self.assertFalse(a.is_rise(1_632_046, 1_600_046, "2"))
        self.assertTrue(a.is_rise(1_632_047, 1_600_046, "2"))

    def test_the_limit_moves_the_boundary(self):
        self.assertTrue(a.is_rise(1_608_047, 1_600_046, "0.5"))
        self.assertFalse(a.is_rise(1_608_046, 1_600_046, "0.5"))
        self.assertTrue(a.is_rise(1_616_047, 1_600_046, "1"))
        self.assertFalse(a.is_rise(1_616_046, 1_600_046, "1"))


class Ledger(unittest.TestCase):
    def run_check(self, head, base_ledger="", ledger=None, pr=400, base="1600046", arch="arm64"):
        return a.check(
            arch, a.cases(output(head, base)), "2", a.parse_ledger(ledger if ledger is not None else entry()),
            a.parse_ledger(base_ledger), pr, a.tomllib.loads(BUDGETS),
        )

    def test_no_rise_passes(self):
        rows, errors, warnings = self.run_check(1_632_046, ledger="")
        self.assertEqual((errors, warnings), ([], []))
        self.assertEqual(rows[0][-1], "ok")

    def test_uncovered_rise_names_everything(self):
        _, errors, _ = self.run_check(1_692_456, ledger="")
        self.assertEqual(len(errors), 1)
        for text in ("time_limit_tick", "arm64", "1,600,046", "1,692,456", "+5.78%", "bench-profile-arm64"):
            self.assertIn(text, errors[0])

    def test_a_new_entry_with_the_pr_covers_it(self):
        rows, errors, _ = self.run_check(1_692_456)
        self.assertEqual(errors, [])
        self.assertEqual(rows[0][-1], "accepted #400")

    def test_another_pr_does_not_cover(self):
        self.assertEqual(len(self.run_check(1_692_456, pr=401)[1]), 1)

    def test_head_over_count_fails_naming_count(self):
        _, errors, _ = self.run_check(1_693_001)
        self.assertIn("1,693,000", errors[0])

    def test_an_entry_already_in_the_base_does_not_cover(self):
        errors = self.run_check(1_692_456, base_ledger=entry())[1]
        self.assertEqual(len(errors), 1)

    def test_an_edited_base_entry_fails(self):
        errors = self.run_check(1_632_046, base_ledger=entry(), ledger=entry(count=1_700_000))[1]
        self.assertTrue(any("append-only" in e for e in errors), errors)

    def test_a_removed_base_entry_fails(self):
        errors = self.run_check(1_632_046, base_ledger=entry(), ledger="")[1]
        self.assertTrue(any("append-only" in e for e in errors), errors)

    def test_entry_for_a_case_that_did_not_rise_warns(self):
        _, errors, warnings = self.run_check(1_600_100)
        self.assertEqual(errors, [])
        self.assertEqual(len(warnings), 1)

    def test_other_arch_entry_does_not_cover(self):
        self.assertEqual(len(self.run_check(1_692_456, arch="x86_64")[1]), 1)

    def test_no_base_is_never_a_rise(self):
        rows, errors, _ = self.run_check(5, base="N/A", ledger="")
        self.assertEqual(errors, [])
        self.assertEqual(rows[0][-1], "no base")


class Schema(unittest.TestCase):
    def bad(self, text):
        with self.assertRaises(a.LedgerError):
            a.parse_ledger(text, benches={"time_limit_tick"})

    def test_valid_entry_loads(self):
        self.assertEqual(len(a.parse_ledger(entry(), benches={"time_limit_tick"})), 1)

    def test_unknown_key_fails(self):
        self.bad(entry() + "extra = 1\n")

    def test_missing_reason_fails(self):
        self.bad(entry().replace('reason = "a new rule"\n', ""))

    def test_empty_reason_fails(self):
        self.bad(entry(reason=""))

    def test_count_not_above_base_fails(self):
        self.bad(entry(count=1_600_046))

    def test_unknown_bench_fails(self):
        self.bad(entry(bench="nope"))

    def test_unknown_arch_fails(self):
        self.bad(entry(arch="riscv"))


class Cli(unittest.TestCase):
    def run_cli(self, head, ledger, base_ledger, pr="400"):
        with tempfile.TemporaryDirectory() as d:
            paths = {n: os.path.join(d, n) for n in ("bench.txt", "accepted.toml", "base.toml", "budgets.toml", "sum.md")}
            for n, text in (("bench.txt", output(head)), ("accepted.toml", ledger),
                            ("base.toml", base_ledger), ("budgets.toml", BUDGETS)):
                with open(paths[n], "w") as f:
                    f.write(text)
            env = dict(os.environ, GITHUB_STEP_SUMMARY=paths["sum.md"])
            p = subprocess.run(
                [sys.executable, SCRIPT, "--arch", "arm64", "--bench-output", paths["bench.txt"],
                 "--limit-pct", "2", "--accepted", paths["accepted.toml"], "--accepted-base", paths["base.toml"],
                 "--pr", pr, "--budgets", paths["budgets.toml"]],
                capture_output=True, text=True, env=env, check=False,
            )
            md = open(paths["sum.md"]).read() if os.path.exists(paths["sum.md"]) else ""
            return p, md

    def test_rise_exits_one_with_error_line_and_table(self):
        p, md = self.run_cli(1_692_456, "", "")
        self.assertEqual(p.returncode, 1)
        self.assertIn("error:", p.stderr)
        self.assertIn("### Benches on arm64", md)
        self.assertIn("| Bench | Case | Base | Head | Change | Alarm |", md)
        self.assertIn("rise", md)

    def test_covered_rise_exits_zero(self):
        p, md = self.run_cli(1_692_456, entry(), "")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("accepted #400", md)

    def test_warning_is_printed_for_an_unneeded_entry(self):
        p, _ = self.run_cli(1_600_100, entry(), "")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("::warning::", p.stdout)

    def test_empty_accepted_base_means_no_entries(self):
        p, _ = self.run_cli(1_600_100, "", "")
        self.assertEqual(p.returncode, 0, p.stderr)

    def test_no_pr_number_covers_nothing(self):
        p, _ = self.run_cli(1_692_456, entry(), "", pr="")
        self.assertEqual(p.returncode, 1)


if __name__ == "__main__":
    unittest.main()
