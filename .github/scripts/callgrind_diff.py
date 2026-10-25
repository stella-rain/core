#!/usr/bin/env python3
"""Per-function profile of every bench case, base against head (ADR-SIM-8).

Reads the callgrind files iai-callgrind leaves under its home, runs `callgrind_annotate` on the
head and, when there is one, the base (`.out.base@main`) of each case, and writes the annotate
texts and a diff per case into --out. It appends a short profile to $GITHUB_STEP_SUMMARY. A
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
    """[(label, head file, base file or None)], label `<target>.<case>`, in sorted order."""
    found = []
    for dirpath, _, files in os.walk(home):
        heads = [f for f in files if f.startswith("callgrind.") and f.endswith(".out")]
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
    args = parser.parse_args()
    cases = find_cases(args.home)
    if not cases:
        print(f"error: no callgrind file under {args.home}", file=sys.stderr)
        return 1
    os.makedirs(args.out, exist_ok=True)
    summary = ["### Profile on " + args.arch, ""]
    for label, head_path, base_path in cases:
        diffs = {}
        for inclusive, kind in ((False, "exclusive"), (True, "inclusive")):
            parsed = {}
            for side, path in (("head", head_path), ("base", base_path)):
                if path is None:
                    parsed[side] = None
                    continue
                text = annotate(args.annotate, path, inclusive)
                write(args.out, f"{label}.{side}.{kind}.txt", text)
                parsed[side] = parse_annotate(text)
            lines = diff_lines(parsed["base"], parsed["head"], args.top)
            suffix = "diff-inclusive" if inclusive else "diff"
            write(args.out, f"{label}.{suffix}.txt", "\n".join(lines) + "\n")
            diffs[kind] = lines
        lines = diffs["exclusive"]
        moved = lines[0].endswith("(no base)") or not lines[0].endswith("(+0.0%)")
        if moved:
            summary += [f"**{label}**", "", "```", *lines[: 2 + args.summary_top], "```", ""]
    summary.append(f"Per-function diffs and the callgrind files: artifact `bench-profile-{args.arch}`.")
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a", encoding="utf-8") as f:
            f.write("\n".join(summary) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
