#!/usr/bin/env python3
"""Fail when a bench counts more instructions than its budget for the job's architecture.

Reads the iai-callgrind output the bench job tees to a file and benches/budgets.toml (ADR-SIM-7:
an absolute ceiling beside the relative limit of bench_alarm.py, ADR-SIM-8). Only benches named in the budgets file are
checked; a budgeted bench missing from the output, or an unknown arch, fails. With `--bench <name>`
(repeatable; a shard of the bench job) only those budgets are checked, and "missing" applies to them only.
"""

import argparse
import os
import re
import sys
import tomllib

ANSI = re.compile(r"\x1b\[[0-9;]*m")
HEADER = re.compile(r"^([a-z_][a-z0-9_]*)::\S+")
INSTRUCTIONS = re.compile(r"^\s*Instructions:\s*(\d+)")


def counts(text):
    """The instruction count per bench target: the largest of its blocks."""
    found, current = {}, None
    for line in ANSI.sub("", text).splitlines():
        header = HEADER.match(line)
        if header:
            current = header.group(1)
            continue
        count = INSTRUCTIONS.match(line)
        if count and current:
            found[current] = max(found.get(current, 0), int(count.group(1)))
    return found


def check(arch, found, budgets, only=None):
    """Rows (bench, count, budget) and the error lines; an over-budget count is an error.

    `only` limits the check to those benches; a name with no budget is an error."""
    rows, errors = [], []
    for bench in only or ():
        if bench not in budgets:
            errors.append(f"{bench}: not a key of the budgets file")
    for bench, per_arch in budgets.items():
        if only is not None and bench not in only:
            continue
        if arch not in per_arch:
            known = ", ".join(sorted(per_arch))
            errors.append(f"{bench}: no budget for arch {arch!r} (budgets exist for: {known})")
            continue
        budget = per_arch[arch]
        if bench not in found:
            errors.append(f"{bench}: missing from the bench output, so its budget of {budget:,} was not checked")
            continue
        rows.append((bench, found[bench], budget))
    errors += overshoots(arch, rows)
    return rows, errors


def overshoots(arch, rows):
    return [
        f"{bench}: {count:,} instructions on {arch}, over the budget of {budget:,} by {count - budget:,}"
        for bench, count, budget in rows
        if count > budget
    ]


def table(arch, rows):
    lines = [
        f"### Tick budgets on {arch}",
        "",
        "| Bench | Instructions | Budget | Headroom | % left |",
        "|---|---:|---:|---:|---:|",
    ]
    lines += [f"| {b} | {n:,} | {m:,} | {m - n:,} | {100 * (m - n) / m:.1f}% |" for b, n, m in rows]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--bench-output", required=True)
    parser.add_argument("--budgets", default="benches/budgets.toml")
    parser.add_argument("--bench", action="append", help="check only this bench's budget (repeatable)")
    args = parser.parse_args()
    with open(args.bench_output, encoding="utf-8") as f:
        found = counts(f.read())
    with open(args.budgets, "rb") as f:
        budgets = tomllib.load(f)
    rows, errors = check(args.arch, found, budgets, args.bench)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary and rows:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(table(args.arch, rows))
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
