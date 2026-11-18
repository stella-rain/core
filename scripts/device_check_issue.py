#!/usr/bin/env python3
"""Open the device-check issue for a 0.N.0 release (ADR-REL-6 point 4; docs/device-gates.md).

Promote runs this after it has tagged a release. A minor or major release (`vX.Y.0`) gets one
issue, labelled for the two device halves; a patch or an rc tag gets none, and a re-run finds
the issue by its exact title and opens no second one. Standard library only; needs `gh`.

    python3 scripts/device_check_issue.py --tag vX.Y.Z --commit <sha> [--repo OWNER/REPO] [--dry-run]
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile

LABELS = ("cmd:verify-needs-android", "cmd:verify-needs-iphone")


def wants_issue(tag: str) -> bool:
    """A minor or major release (`v<major>.<minor>.0`) calls for a device run; a patch does not."""
    return re.fullmatch(r"v(\d+)\.(\d+)\.0", tag) is not None


def issue_title(tag: str) -> str:
    return f"Device check: {tag}"


def issue_body(tag: str, commit: str) -> str:
    return f"""Release `{tag}` at commit `{commit}`.

This is the device corpus run of ADR-REL-6 on the released tag. It runs after Promote and is not a
Candidate gate. Roles are those of ADR-PLAT-10; the issue names roles, never models or serials.

## Android

From a checkout of `{tag}`, with the phone connected (`--serial` when several are):

```
python3 scripts/android_device_gate.py
```

Set-up is in `docs/device-gates.md`.

## iPhone

From a Mac with the iPhone connected, from a checkout of `{tag}` (set-up in `docs/device-gates.md`):

```
cargo test --locked --all-features --test integration --release --target aarch64-apple-ios -- corpus:: the_golden_hash_is_the_same_on_every_platform
```

## Results

- [ ] Reference phone (Android): corpus passed
- [ ] iOS reference device (iPhone): corpus passed
- [ ] Profiling phones attached: corpus passed on each (none attached: write "none")

Record each device run as one comment: the role, the OS version, the command, and the last
`test result:` line with the exit status. No serials, no personal data.

A full pass removes both labels and closes this issue. A failure stays open and starts the next
patch (ADR-REL-6).
"""


def existing_issue(issues: list, title: str):
    """The number of the issue with exactly this title, in any state, or None."""
    for issue in issues:
        if issue.get("title") == title:
            return issue.get("number")
    return None


def run_gh(args: list) -> subprocess.CompletedProcess:
    return subprocess.run(["gh", *args], capture_output=True, text=True)


def fail(message: str, code: int) -> int:
    print(f"device-issue: {message}")
    return code


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--repo")
    parser.add_argument("--dry-run", action="store_true")
    try:
        args = parser.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0

    if not re.fullmatch(r"v\d+\.\d+\.\d+(-rc\.\d+)?", args.tag):
        return fail(f"{args.tag!r} is not a release tag", 2)
    if not re.fullmatch(r"[0-9a-f]{7,40}", args.commit):
        return fail(f"{args.commit!r} is not a commit sha", 2)
    if not wants_issue(args.tag):
        print(f"device-issue: {args.tag} is not a minor or major release; no issue is called for")
        return 0

    title, body = issue_title(args.tag), issue_body(args.tag, args.commit)
    if args.dry_run:
        print(f"{title}\n\n{body}")
        return 0

    repo = ["--repo", args.repo] if args.repo else []
    listed = run_gh(["issue", "list", "--state", "all", "--limit", "1000",
                     "--json", "number,title", *repo])
    if listed.returncode != 0:
        return fail(f"gh issue list failed ({listed.returncode}): {listed.stderr.strip()}",
                    listed.returncode)
    try:
        issues = json.loads(listed.stdout or "[]")
    except json.JSONDecodeError as e:
        return fail(f"gh issue list gave unreadable output: {e}", 1)
    found = existing_issue(issues, title)
    if found is not None:
        print(f"device-issue: issue #{found} already exists for {args.tag}; none opened")
        return 0

    with tempfile.NamedTemporaryFile("w", suffix=".md", encoding="utf-8") as f:
        f.write(body)
        f.flush()
        created = run_gh(["issue", "create", "--title", title, "--body-file", f.name,
                          "--label", LABELS[0], "--label", LABELS[1], *repo])
    if created.returncode != 0:
        return fail(f"gh issue create failed ({created.returncode}): {created.stderr.strip()}",
                    created.returncode)
    print(f"device-issue: opened {created.stdout.strip() or title}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
