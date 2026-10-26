#!/usr/bin/env python3
"""Decide which cases of a bench target the bench job compares with the base commit (ADR-SIM-10).

Reads tests/bench_pins.toml of the base and of the head. A case is comparable when it is in both
files with equal `before`, `stage` and `input` digests (`after` is pinned only) and the target's
bench file and benches/support/ are unchanged (--bench-changed true makes nothing comparable).
Prints one line per case, and writes `compare=<true|false>` and `cases=<comma list>` to
$GITHUB_OUTPUT. A missing or empty base file compares nothing. The head file must hold every
case of the target the base held (unless the bench file changed) and at least one.
"""

import argparse
import os
import sys
import tomllib

DIGESTS = ("before", "stage", "input")
NO_BASE = "::notice::the base has no tests/bench_pins.toml: nothing compared"


def load(path):
    """{case: table} of a pin file, or None when it is missing or empty; invalid TOML raises."""
    try:
        with open(path, "rb") as f:
            data = tomllib.load(f)
    except OSError:
        return None
    return data or None


def of_target(pins, bench):
    return {case: t for case, t in pins.items() if isinstance(t, dict) and t.get("bench") == bench}


def decide(base, head, bench, bench_changed):
    """(cases to compare, report lines); raises ValueError when the head lacks a case."""
    mine = of_target(head, bench)
    if not mine:
        raise ValueError(f"head tests/bench_pins.toml has no case of the target {bench}")
    if not bench_changed:
        for case in of_target(base, bench):
            if case not in mine:
                raise ValueError(f"head tests/bench_pins.toml lacks the case {case} of the target {bench}")
    compared, lines = [], []
    before = of_target(base, bench)
    for case, table in mine.items():
        if bench_changed:
            lines.append(f"{case}: bench changed: not compared")
        elif case not in before:
            lines.append(f"{case}: new case: not compared")
        else:
            moved = [k for k in DIGESTS if before[case].get(k) != table.get(k)]
            if moved:
                lines.append(f"{case}: case moved ({'|'.join(moved)} differ): not compared")
            else:
                compared.append(case)
                lines.append(f"{case}: compared")
    return compared, lines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--bench", required=True)
    parser.add_argument("--bench-changed", required=True, choices=("true", "false"))
    args = parser.parse_args()
    try:
        head = load(args.head)
        base = load(args.base)
    except tomllib.TOMLDecodeError as e:
        print(f"error: tests/bench_pins.toml is not valid TOML: {e}", file=sys.stderr)
        return 1
    if head is None:
        print(f"error: {args.head} is missing or empty", file=sys.stderr)
        return 1
    if base is None:
        print(NO_BASE)
        compared, lines = [], []
    else:
        try:
            compared, lines = decide(base, head, args.bench, args.bench_changed == "true")
        except ValueError as e:
            print(f"error: {e}", file=sys.stderr)
            return 1
    for line in lines:
        print(line)
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a", encoding="utf-8", newline="\n") as f:
            f.write(f"compare={'true' if compared else 'false'}\ncases={','.join(compared)}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
