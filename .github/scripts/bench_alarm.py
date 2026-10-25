#!/usr/bin/env python3
"""Fail when a bench case rose by more than the limit, unless a new ledger entry covers it (ADR-SIM-8).

Reads the iai-callgrind output the bench job tees to a file (`Instructions: <head>|<base>`) and
benches/accepted.toml. A case rose when (head - base) * 100 > base * limit, exactly (the rule of
iai's `--callgrind-limits=ir=2%`). The ledger is append-only: the base commit's entries must be a
prefix of the head's. An entry added by this PR covers a rise of its bench, case and arch when its
`pr` is this PR's number and the head count is at most its `count`. An unexplained rise is fixed,
not accepted; the profile (bench-profile-<arch>) says where it comes from.
"""

import argparse
import os
import re
import sys
import tomllib
from fractions import Fraction

ANSI = re.compile(r"\x1b\[[0-9;]*m")
HEADER = re.compile(r"^([a-z_][a-z0-9_]*)::\S+(?:\s+([A-Za-z0-9_]+):)?")
INSTRUCTIONS = re.compile(r"^\s*Instructions:\s*(\d+)\|(\d+|N/A)")
ARCHES = ("x86_64", "arm64")
KEYS = {"bench": str, "case": str, "arch": str, "base": int, "count": int, "reason": str, "pr": int}


class LedgerError(Exception):
    pass


def cases(text):
    """[(target, case, head, base or None)] in output order."""
    found, current = [], None
    for line in ANSI.sub("", text).splitlines():
        header = HEADER.match(line)
        if header:
            current = (header.group(1), header.group(2) or header.group(1))
            continue
        count = INSTRUCTIONS.match(line)
        if count and current:
            base = None if count.group(2) == "N/A" else int(count.group(2))
            found.append((*current, int(count.group(1)), base))
    return found


def is_rise(head, base, limit_pct):
    return (head - base) * 100 > base * Fraction(str(limit_pct))


def parse_ledger(text, benches=None):
    """The `[[accepted]]` entries, validated; `benches` limits `bench` when given."""
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as e:
        raise LedgerError(f"benches/accepted.toml is not valid TOML: {e}") from e
    if set(data) - {"accepted"}:
        raise LedgerError(f"benches/accepted.toml: unknown top-level key(s) {sorted(set(data) - {'accepted'})}")
    entries = data.get("accepted", [])
    for i, e in enumerate(entries, 1):
        where = f"benches/accepted.toml entry {i}"
        if set(e) != set(KEYS):
            raise LedgerError(f"{where}: keys must be exactly {sorted(KEYS)}, got {sorted(e)}")
        for key, kind in KEYS.items():
            if not isinstance(e[key], kind) or isinstance(e[key], bool):
                raise LedgerError(f"{where}: {key} must be {kind.__name__}")
        if not e["reason"].strip() or "\n" in e["reason"]:
            raise LedgerError(f"{where}: reason must be a non-empty single line")
        if e["arch"] not in ARCHES:
            raise LedgerError(f"{where}: arch {e['arch']!r} is not one of {ARCHES}")
        if benches is not None and e["bench"] not in benches:
            raise LedgerError(f"{where}: bench {e['bench']!r} is not a key of benches/budgets.toml")
        if e["count"] <= e["base"]:
            raise LedgerError(f"{where}: count {e['count']} must be above base {e['base']}")
    return entries


def pct(head, base):
    return f"{100 * (head - base) / base:+.2f}%"


def check(arch, found, limit_pct, ledger, base_ledger, pr, budgets=None):
    """(rows, errors, warnings); a row is (bench, case, base, head, change, alarm)."""
    rows, errors, warnings = [], [], []
    if ledger[: len(base_ledger)] != base_ledger:
        errors.append("benches/accepted.toml is append-only: an entry of the base commit was edited or removed")
    new = ledger[len(base_ledger):]
    risen = set()
    for bench, case, head, base in found:
        if base is None:
            rows.append((bench, case, None, head, "", "no base"))
            continue
        change = pct(head, base)
        if not is_rise(head, base, limit_pct):
            rows.append((bench, case, base, head, change, "ok"))
            continue
        risen.add((bench, case))
        match = [e for e in new if (e["bench"], e["case"], e["arch"]) == (bench, case, arch) and e["pr"] == pr]
        if match and head <= match[-1]["count"]:
            rows.append((bench, case, base, head, change, f"accepted #{pr}"))
            continue
        rows.append((bench, case, base, head, change, "rise"))
        why = f"; the entry's count is {match[-1]['count']:,}" if match else ""
        errors.append(
            f"{bench} {case} on {arch}: {base:,} -> {head:,} instructions ({change}), over the limit of "
            f"+{limit_pct}%{why}. profile: artifact bench-profile-{arch}; "
            "an unexplained rise is fixed, not accepted (ADR-SIM-8)"
        )
    for e in new:
        if e["arch"] == arch and (e["bench"], e["case"]) not in risen:
            warnings.append(
                f"benches/accepted.toml entry for {e['bench']} {e['case']} on {arch} is not needed: "
                "the case did not rise"
            )
    return rows, errors, warnings


def table(arch, rows):
    lines = [f"### Benches on {arch}", "", "| Bench | Case | Base | Head | Change | Alarm |", "|---|---|---:|---:|---:|---|"]
    for bench, case, base, head, change, alarm in rows:
        lines.append(f"| {bench} | {case} | {'N/A' if base is None else f'{base:,}'} | {head:,} | {change} | {alarm} |")
    return "\n".join(lines) + "\n"


def read(path):
    if not path:
        return ""
    with open(path, encoding="utf-8") as f:
        return f.read()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--bench-output", required=True)
    parser.add_argument("--limit-pct", required=True)
    parser.add_argument("--accepted", default="benches/accepted.toml")
    parser.add_argument("--accepted-base", default="")
    parser.add_argument("--pr", default="")
    parser.add_argument("--budgets", default="benches/budgets.toml")
    args = parser.parse_args()
    try:
        with open(args.budgets, "rb") as f:
            benches = set(tomllib.load(f))
        ledger = parse_ledger(read(args.accepted), benches)
        base_ledger = parse_ledger(read(args.accepted_base))
        pr = int(args.pr) if args.pr else None
    except (LedgerError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    rows, errors, warnings = check(args.arch, cases(read(args.bench_output)), args.limit_pct, ledger, base_ledger, pr)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary and rows:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(table(args.arch, rows))
    for warning in warnings:
        print(f"::warning::{warning}")
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
