#!/usr/bin/env python3
"""Per-function profile of every bench case, base against head (ADR-SIM-8).

Reads the callgrind files iai-callgrind leaves under its home, runs `callgrind_annotate` on the
head and, when there is one, the base (`.out.base@main`) of each case, and writes the annotate
texts and a diff per case into --out, the inclusive cost of each named tick step (`.steps.txt`).
It appends a short profile to $GITHUB_STEP_SUMMARY. A
profile never fails the job by its content: it exits 1 only when the home holds no callgrind file.
"""

import argparse
import os
import re
import subprocess
import sys

LINE = re.compile(r"^\s*([\d,]+)\s+(\S.*?)\s*$")
BINARY = re.compile(r"\s+\[[^\]]*\]$")
NAME_MAX = 200
TOTALS = "PROGRAM TOTALS"
# The live rules and the frozen ones (ADR-REPL-9) run the same schedule; a step is counted by its name.
STEP_PREFIXES = (
    "stella_rain_core::sim::schedule::steps::",
    "stella_rain_core::sim_v1::schedule::steps::",
)
STEP_NAME = re.compile(r"[A-Za-z0-9_]+")
STEP_SELF = re.compile(r"^(?:|::h[0-9a-f]+|<.*)$")
RECURSION = re.compile(r"'\d+$")  # callgrind's name for a recursive instance: free'2
FILE_SPLIT = re.compile(r"^[^:]*?:(?!:)")


def parse_annotate(text):
    """(total, {function: Ir}) of one `callgrind_annotate` text; the binary path is dropped."""
    total, rows = 0, {}
    for line in text.splitlines():
        m = LINE.match(line)
        if not m:
            continue
        count, name = int(m.group(1).replace(",", "")), BINARY.sub("", m.group(2))
        if name == TOTALS:
            total = count
        elif ":" in name:
            rows[name] = rows.get(name, 0) + count
    return total, rows


def step_prefix(name):
    """The schedule prefix `name` starts with, or None."""
    return next((p for p in STEP_PREFIXES if name.startswith(p)), None)


def parse_steps(rows):
    """{step: inclusive Ir} of the rows of a parsed inclusive annotate that are named tick steps."""
    steps = {}
    for name, count in rows.items():
        name = FILE_SPLIT.sub("", name, count=1)
        name = RECURSION.sub("", name.rstrip())
        prefix = step_prefix(name)
        if prefix:
            step = STEP_NAME.match(name[len(prefix):])
            # only the step's own function (a hash or generics may follow), not a closure or helper inside it
            if step and STEP_SELF.match(name[len(prefix) + step.end():]):
                steps[step.group()] = steps.get(step.group(), 0) + count
    return steps


def steps_unreliable(rows, total):
    """True when the step frames of a parsed inclusive annotate nest: a step carries a recursion suffix, or the
    steps sum to more than the case total plus 1% (tail calls leave frames open, so a later step runs inside an earlier)."""
    steps = 0
    for name, count in rows.items():
        name = FILE_SPLIT.sub("", name, count=1).rstrip()
        bare = RECURSION.sub("", name)
        prefix = step_prefix(bare)
        if prefix:
            step = STEP_NAME.match(bare[len(prefix):])
            if step and STEP_SELF.match(bare[len(prefix) + step.end():]):
                if bare != name:
                    return True
                steps += count
    return steps * 100 > total * 101


def read_order(path):
    """The step names inside `steps![...]` of the schedule file, or None when there are none."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return None
    start = text.find("steps![")
    if start < 0:
        return None
    body = text[start + len("steps!["):].split("]", 1)[0]
    return re.findall(r"[A-Za-z_][A-Za-z0-9_]*", body) or None


def order_steps(names, order):
    """Names in schedule order, those the schedule does not list last in name order."""
    listed = [n for n in (order or []) if n in names]
    return listed + sorted(set(names) - set(listed))


def step_table(base, head, order):
    """[(step, base Ir or None, head Ir)] in order, then `rest` (total less the steps); each side is (total, steps)."""
    head_total, head_steps = head
    names = set(head_steps) | set(base[1] if base else ())
    rows = [(n, base[1].get(n, 0) if base else None, head_steps.get(n, 0)) for n in order_steps(names, order)]
    rest_base = base[0] - sum(base[1].values()) if base else None
    return rows + [("rest", rest_base, head_total - sum(head_steps.values()))]


def changed(rows):
    return [r for r in rows if r[1] is not None and r[1] != r[2]]


def step_lines(rows, with_base):
    if not with_base:
        return ["step head"] + [f"{n} {h:,}" for n, _, h in rows]
    return ["step base head delta"] + [f"{n} {b:,} {h:,} {h - b:+,}" for n, b, h in rows]


def cut(name):
    return name[:NAME_MAX]


def diff_lines(base, head, top):
    """The diff text of one case as lines; `base` is None when there is nothing to compare with."""
    head_total, head_rows = head
    if base is None:
        lines = [f"total: head {head_total:,} (no base)", "head function"]
        ranked = sorted(head_rows.items(), key=lambda kv: (-kv[1], kv[0]))
        return lines + [f"{n:,} {cut(name)}" for name, n in ranked[:top]]
    base_total, base_rows = base
    change = f"{100 * (head_total - base_total) / base_total:+.1f}%" if base_total else "n/a"
    lines = [f"total: base {base_total:,} head {head_total:,} ({change})", "delta base head function"]
    deltas = []
    for name in base_rows.keys() | head_rows.keys():
        b, h = base_rows.get(name, 0), head_rows.get(name, 0)
        if h != b:
            deltas.append((h - b, b, h, name))
    deltas.sort(key=lambda r: (-abs(r[0]), r[3]))
    return lines + [f"{d:+,} {b:,} {h:,} {cut(n)}" for d, b, h, n in deltas[:top]]


def find_cases(home):
    """[(label, head file, base file or None)], label `<target>.<case>`, in sorted order.

    A push to main saves its run as the baseline, so a case may have only `.out.base@main`:
    that file is then the head, with nothing to diff against."""
    found = []
    for dirpath, _, files in os.walk(home):
        heads = [f for f in files if f.startswith("callgrind.") and f.endswith(".out")]
        heads += [f for f in files
                  if f.startswith("callgrind.") and f.endswith(".out.base@main")
                  and f[:-len(".base@main")] not in files]
        for head in heads:
            parts = os.path.relpath(dirpath, home).split(os.sep)
            if len(parts) < 3:
                continue
            target, case = parts[-3], parts[-1].split(".", 1)[-1]
            base = head + ".base@main"
            found.append((
                f"{target}.{case}",
                os.path.join(dirpath, head),
                os.path.join(dirpath, base) if base in files else None,
            ))
    return sorted(found)


def annotate(tool, path, inclusive):
    cmd = [tool, "--show=Ir", "--show-percs=no", "--threshold=100",
           f"--inclusive={'yes' if inclusive else 'no'}", path]
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def profile(tool, path, inclusive):
    """The annotate text, or None (after a warning) when the tool cannot read the file."""
    try:
        return annotate(tool, path, inclusive)
    except (OSError, subprocess.CalledProcessError) as e:
        print(f"::warning::{tool} failed on {path}: {e}")
        return None


def write(out, name, text):
    with open(os.path.join(out, name), "w", encoding="utf-8") as f:
        f.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", required=True, help="IAI_CALLGRIND_HOME")
    parser.add_argument("--out", required=True)
    parser.add_argument("--arch", default=os.environ.get("RUNNER_ARCH", "").lower() or "unknown")
    parser.add_argument("--top", type=int, default=40)
    parser.add_argument("--summary-top", type=int, default=10)
    parser.add_argument("--annotate", default="callgrind_annotate")
    parser.add_argument("--schedule", default="src/sim/schedule.rs",
                        help="the file whose steps![...] gives the order of the step table")
    args = parser.parse_args()
    cases = find_cases(args.home)
    if not cases:
        print(f"error: no callgrind file under {args.home}", file=sys.stderr)
        return 1
    os.makedirs(args.out, exist_ok=True)
    summary = ["### Profile on " + args.arch, ""]
    order, any_steps = read_order(args.schedule), False
    unreliable = set()
    for label, head_path, base_path in cases:
        lines, moved, step_rows = None, False, None
        for inclusive, kind in ((False, "exclusive"), (True, "inclusive")):
            parsed = {}
            for side, path in (("head", head_path), ("base", base_path)):
                text = profile(args.annotate, path, inclusive) if path else None
                if text is not None:
                    write(args.out, f"{label}.{side}.{kind}.txt", text)
                    parsed[side] = parse_annotate(text)
            if "head" not in parsed:
                break
            if base_path and "base" not in parsed:
                break
            base = parsed.get("base")
            diff = diff_lines(base, parsed["head"], args.top)
            suffix = "diff-inclusive" if inclusive else "diff"
            write(args.out, f"{label}.{suffix}.txt", "\n".join(diff) + "\n")
            if not inclusive:
                lines, moved = diff, base is None or base[0] != parsed["head"][0]
            else:
                head_steps = parse_steps(parsed["head"][1])
                base_steps = parse_steps(base[1]) if base else {}
                if steps_unreliable(parsed["head"][1], parsed["head"][0]) or (
                        base and steps_unreliable(base[1], base[0])):
                    note = f"step table unreliable on {args.arch} (nested frames)"
                    write(args.out, f"{label}.steps.txt", note + "\n")
                    unreliable.add(note)
                    any_steps = True
                elif head_steps or base_steps:
                    sides = [(base[0], base_steps) if base else None, (parsed["head"][0], head_steps)]
                    step_rows = step_table(sides[0], sides[1], order)
                    write(args.out, f"{label}.steps.txt", "\n".join(step_lines(step_rows, base is not None)) + "\n")
                    any_steps = True
        if lines is not None and moved:
            summary += [f"**{label}**", "", "```", *lines[: 2 + args.summary_top], "```", ""]
            if step_rows:
                shown = step_rows if base_path is None else changed(step_rows)
                if shown:
                    summary += ["```", *step_lines(shown, base_path is not None), "```", ""]
    if unreliable:
        summary += [*sorted(unreliable), ""]
    if not any_steps:
        summary += ["no named tick steps in the profile (BS2a)", ""]
    summary.append(f"Per-function diffs and the callgrind files: artifact `bench-profile-{args.arch}`.")
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a", encoding="utf-8") as f:
            f.write("\n".join(summary) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
