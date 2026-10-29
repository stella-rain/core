#!/usr/bin/env python3
"""The fuzz targets the CI fuzz runs take (fuzz-short in rust.yml, fuzz-long.yml).

The list is the [[bin]] table of fuzz/Cargo.toml, checked against fuzz/fuzz_targets/*.rs: cargo-fuzz
builds only what the table names, and a file with no entry is never built. Order is the table's.

  fuzz_targets.py [--root DIR] json    the names as a JSON array (for fromJSON)
  fuzz_targets.py [--root DIR] words   the names on one line, space-separated (for a shell for)

Both check first; on a mismatch they print nothing to stdout and exit 1.
"""

import argparse
import json
import os
import re
import sys
import tomllib

DEFAULT_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))


class Fail(Exception):
    pass


def targets(root):
    with open(os.path.join(root, "fuzz", "Cargo.toml"), "rb") as f:
        bins = tomllib.load(f).get("bin", [])
    names = []
    for b in bins:
        name = b.get("name", "")
        if not re.fullmatch(r"[a-z][a-z0-9_]*", name):
            raise Fail(f"bin name {name!r} is not [a-z][a-z0-9_]*")
        if name in names:
            raise Fail(f"bin {name} is listed twice")
        if b.get("path") != f"fuzz_targets/{name}.rs":
            raise Fail(f"bin {name} has path {b.get('path')!r}, not fuzz_targets/{name}.rs")
        names.append(name)
    if not names:
        raise Fail("no [[bin]] in fuzz/Cargo.toml")
    folder = os.path.join(root, "fuzz", "fuzz_targets")
    files = {f[:-3] for f in os.listdir(folder) if f.endswith(".rs")}
    problems = [f"fuzz_targets/{f}.rs has no [[bin]]" for f in sorted(files - set(names))]
    problems += [f"[[bin]] {n} has no fuzz_targets/{n}.rs" for n in names if n not in files]
    if problems:
        raise Fail("; ".join(problems))
    return names


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--root", default=DEFAULT_ROOT)
    p.add_argument("cmd", choices=["json", "words"])
    a = p.parse_args()
    try:
        names = targets(a.root)
    except Fail as e:
        print(f"fuzz_targets: {e}", file=sys.stderr)
        return 1
    # Bytes, not print: on Windows print writes \r\n.
    out = json.dumps(names) if a.cmd == "json" else " ".join(names)
    sys.stdout.buffer.write((out + "\n").encode())
    return 0


if __name__ == "__main__":
    sys.exit(main())
