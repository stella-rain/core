#!/usr/bin/env python3
"""Count a change's hand-written lines, leaving generated files out.

GitHub's +/- totals on a PR count every file, so a change that regenerates the corpus reads as
a hundred thousand lines. This counts `git diff --numstat BASE...HEAD` without every path that
HEAD's .gitattributes marks `linguist-generated` (the corpus, schema, goldens, content pack and
bench pins; test_gitattributes.py keeps that list whole). A binary or -diff file counts as a
file with no lines. Prints two lines, or with --json one object:
{"hand_written": {"additions", "deletions", "files"}, "generated": {"files"}}.

Use it wherever a line count decides something (a review threshold, a PR's size), not the
API's additions and deletions. Run in a clone that has both commits.
"""

import argparse
import json
import subprocess
import sys

# The text line names at most this many generated files.
SHOWN = 5


def git(*args, stdin=None):
    r = subprocess.run(["git", *args], input=stdin, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


def numstat(base, head):
    """[(additions, deletions, path)]; None and None for a binary or -diff file."""
    out = git("diff", "--numstat", "-z", "--no-renames", f"{base}...{head}")
    rows = []
    for record in filter(None, out.split("\0")):
        add, delete, path = record.split("\t", 2)
        rows.append((None if add == "-" else int(add), None if delete == "-" else int(delete), path))
    return rows


def generated(head, paths):
    """The paths HEAD's .gitattributes marks linguist-generated."""
    if not paths:
        return set()
    out = git("check-attr", "-z", "--stdin", "--source", head, "linguist-generated",
              stdin="\0".join(paths) + "\0")
    fields = out.split("\0")
    # -z output: path, attribute, value, repeated.
    return {fields[i] for i in range(0, len(fields) - 2, 3) if fields[i + 2] == "true"}


def count(base, head):
    rows = numstat(base, head)
    gen = generated(head, [p for _, _, p in rows])
    hand = [r for r in rows if r[2] not in gen]
    return {
        "hand_written": {
            "additions": sum(a or 0 for a, _, _ in hand),
            "deletions": sum(d or 0 for _, d, _ in hand),
            "files": sorted(p for _, _, p in hand),
        },
        "generated": {"files": sorted(gen)},
    }


def plural(n):
    return f"{n} file" if n == 1 else f"{n} files"


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("base")
    p.add_argument("head")
    p.add_argument("--json", action="store_true", help="print one JSON object")
    a = p.parse_args(argv)
    git("rev-parse", "--verify", "--quiet", f"{a.base}^{{commit}}")
    git("rev-parse", "--verify", "--quiet", f"{a.head}^{{commit}}")
    c = count(a.base, a.head)
    if a.json:
        print(json.dumps(c))
        return 0
    h, g = c["hand_written"], c["generated"]["files"]
    print(f"hand-written: +{h['additions']} -{h['deletions']} in {plural(len(h['files']))}")
    names = ", ".join(g[:SHOWN]) + (f", and {len(g) - SHOWN} more" if len(g) > SHOWN else "")
    print(f"generated: {plural(len(g))}" + (f" ({names})" if g else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
