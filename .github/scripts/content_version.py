#!/usr/bin/env python3
"""The VERSION the content publish pull request carries.

content-publish.yml opens a pull request into the content repository. Content's VERSION is the
release it will make next: it stays when it is above content's latest final `v*` tag (a release
is pending), otherwise it becomes that tag's next patch. Only final tags `vX.Y.Z` count; release
candidates (`-rc.N`) and anything else are skipped.

  content_version.py <path of VERSION>    the value on one line; tag names, one per line, on stdin
                                          (a `refs/tags/` prefix is stripped)

A bad VERSION file prints nothing to stdout, `content_version.py: <reason>` to stderr, and exits 1.
"""

import re
import sys

NUM = r"(0|[1-9][0-9]*)"
VERSION = re.compile(rf"{NUM}\.{NUM}\.{NUM}")
TAG = re.compile(rf"v{NUM}\.{NUM}\.{NUM}")


def parse(pattern, text):
    m = pattern.fullmatch(text)
    return tuple(int(g) for g in m.groups()) if m else None


def next(version, tags):
    current = parse(VERSION, version.strip())
    if current is None:
        raise ValueError(f"not a version x.y.z: {version!r}")
    latest = None
    for tag in tags:
        tag = tag.strip().removeprefix("refs/tags/")
        found = parse(TAG, tag)
        if found is not None and (latest is None or found > latest):
            latest = found
    if latest is None or current > latest:
        return ".".join(str(n) for n in current)
    return f"{latest[0]}.{latest[1]}.{latest[2] + 1}"


def main(argv, stdin):
    if len(argv) != 2:
        print("usage: content_version.py <path of VERSION>  (tag names on stdin)", file=sys.stderr)
        return 1
    try:
        with open(argv[1], encoding="utf-8") as f:
            version = f.read()
        print(next(version, stdin.read().splitlines()))
    except (OSError, ValueError) as e:
        print(f"content_version.py: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv, sys.stdin))
