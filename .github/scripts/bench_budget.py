#!/usr/bin/env python3
"""Fail when a bench counts more instructions than its budget for the job's architecture.

Reads the iai-callgrind output the bench job tees to a file and benches/budgets.toml (D-BUDGET:
an absolute ceiling beside the relative +2% limit). Only benches named in the budgets file are
checked; a budgeted bench missing from the output, or an unknown arch, fails.
"""

import argparse
import os
import re
import sys
import tomllib

HEADER = re.compile(r"^([a-z_][a-z0-9_]*)::\S+")
INSTRUCTIONS = re.compile(r"^\s*Instructions:\s*(\d+)")


def counts(text):
    """The instruction count per bench target: the largest of its blocks."""
    found, current = {}, None
    for line in text.splitlines():
        header = HEADER.match(line)
        if header:
            current = header.group(1)
            continue
        count = INSTRUCTIONS.match(line)
        if count and current:
            found[current] = max(found.get(current, 0), int(count.group(1)))
    return found


def check(arch, found, budgets):
    """Rows (bench, count, budget) and the error lines."""
    rows, errors = [], []
    for bench, per_arch in budgets.items():
        if arch not in per_arch:
            known = ", ".join(sorted(per_arch))
            errors.append(f"{bench}: no budget for arch {arch!r} (budgets exist for: {known})")
            continue
        budget = per_arch[arch]
        if bench not in found:
            errors.append(f"{bench}: missing from the bench output, so its budget of {budget:,} was not checked")
            continue
        rows.append((bench, found[bench], budget))
        if found[bench] > budget:
            over = found[bench] - budget
            errors.append(
                f"{bench}: {found[bench]:,} instructions on {arch}, over the budget of {budget:,} by {over:,}"
            )
    return rows, errors


def table(arch, rows):
    lines = [
        f"### Tick budgets on {arch}",
        "",
        "| Bench | Instructions | Budget | % of budget |",
        "|---|---:|---:|---:|",
    ]
    lines += [f"| {b} | {n:,} | {m:,} | {100 * n / m:.1f}% |" for b, n, m in rows]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--bench-output", required=True)
    parser.add_argument("--budgets", default="benches/budgets.toml")
    args = parser.parse_args()
    with open(args.bench_output, encoding="utf-8") as f:
        found = counts(f.read())
    with open(args.budgets, "rb") as f:
        budgets = tomllib.load(f)
    rows, errors = check(args.arch, found, budgets)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary and rows:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(table(args.arch, rows))
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
