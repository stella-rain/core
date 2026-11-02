#!/usr/bin/env python3
"""Fail the bench job when a measured profile counts the drop of a bench fixture (ADR-SIM-8).

iai-callgrind 0.16 collects only inside `__iai_callgrind_wrapper_mod::<bench fn>`; the teardown runs
outside it. So the drop glue of a fixture is counted when the bench function itself drops it: a
function that takes the `Run`, `Engine` or `Case` by value and returns something else. Those
instructions depend on the heap layout and are not the operation the bench names
(benches/support/mod.rs), and a profile that counts them looks like a rise or a fall of the tick.
`tests/bench_teardown.rs` checks the text of the benches; this reads the measured profile.

The check, per case of $IAI_CALLGRIND_HOME (the head profile only): `callgrind_annotate
--tree=caller --show=Ir --threshold=100`; a callee line starts `*` and the lines of its callers sit
above it, each starting `<`. A hit is a `drop_glue` or `drop_in_place` of a type that names Engine,
Run or Case whose direct caller is the wrapper (the shape seen on valgrind 3.22). Matching is by
marker and name, not by column. It fails closed: no callgrind file, or an annotate that cannot read
a head file, exits 1.

    python3 .github/scripts/bench_drop_guard.py --home "$IAI_CALLGRIND_HOME"
"""

import argparse
import os
import re
import subprocess
import sys

from callgrind_diff import find_cases

DROP = re.compile(r"drop_(?:in_place|glue)\b.*\b(?:Engine|Run|Case)\b")
WRAPPER = "__iai_callgrind_wrapper_mod"
LINE = re.compile(r"^\s*[\d,]+(?:\s+\(\s*[\d.]+%\))?\s+([<*])\s+(.*?)\s*$")
BINARY = re.compile(r"\s+\[/[^\]]*\]$")
CALLS = re.compile(r"\s+\(\d[\d,]*x\)$")


def find_hits(text):
    """[(callee, caller)] for every drop of a fixture called directly from the wrapper."""
    hits, callers = [], []
    for raw in text.splitlines():
        m = LINE.match(raw)
        if not m:
            continue
        name = CALLS.sub("", BINARY.sub("", m.group(2)))
        if m.group(1) == "<":
            callers.append(name)
            continue
        if DROP.search(name):
            hits += [(name, c) for c in callers if WRAPPER in c]
        callers = []
    return hits


def annotate(tool, head):
    cmd = [tool, "--tree=caller", "--show=Ir", "--threshold=100", head]
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", required=True, help="IAI_CALLGRIND_HOME")
    parser.add_argument("--annotate", default="callgrind_annotate")
    args = parser.parse_args()

    cases = find_cases(args.home)
    if not cases:
        print(f"::error title=bench teardown::no callgrind file under {args.home}: the guard cannot read a profile")
        return 1
    failed, summary = False, []
    for label, head, _ in cases:
        try:
            text = annotate(args.annotate, head)
        except (OSError, subprocess.CalledProcessError) as e:
            print(f"::error title=bench teardown::{label}: callgrind_annotate could not read {os.path.basename(head)}: {e}")
            summary.append(f"- {label}: profile unreadable")
            failed = True
            continue
        for callee, caller in find_hits(text):
            print(
                f"::error title=bench teardown::{label}: {callee} is called from {caller}; "
                "end the bench with teardown = support::forget_measured (benches/support/mod.rs)"
            )
            summary.append(f"- {label}: `{callee}` called from `{caller}`")
            failed = True
    if not failed:
        print(f"checked {len(cases)} cases: no drop glue under the wrapper")
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path and summary:
        with open(path, "a", encoding="utf-8") as f:
            f.write("\n".join(["### Bench teardown", "", *summary, ""]) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
