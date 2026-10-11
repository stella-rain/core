"""Release tags (ADR-054): the version a release tags, the checks it reads, and its notes.

A release is the annotated tag `vX.Y.Z`, where X.Y.Z is the crate version in Cargo.toml. It
must be new: not tagged yet and above the last release. fuzz/Cargo.lock must agree with
Cargo.toml, or `cargo build --locked` in fuzz/ fails on the tag. The release workflow tags an
`rc` commit only when every check below succeeded on the head of the PR merged as that commit.

Usage:
  python3 release_tag.py tag CARGO_TOML FUZZ_LOCK [TAG...]
      prints `vX.Y.Z` when the version is new and the lock agrees; else exits 1 with the
      problems. TAGS are the repository's existing tags.
  python3 release_tag.py previous [TAG...]
      prints the last release among TAGS, or nothing.
  python3 release_tag.py verify CHECK_RUNS_JSON
      CHECK_RUNS_JSON is the head commit's check runs (a list of {name, status, conclusion});
      exits 0 when every required check succeeded, else 1 with the problems.
  python3 release_tag.py notes TAG COMMIT PREVIOUS|- PR_URL CHECK_RUNS_JSON LOG_FILE
      prints the release notes; LOG_FILE holds one commit per line since PREVIOUS.

Follows enjay27/lakehouse's install/release_tag.py, with semver tags instead of dates.
"""

import json
import re
import sys
import tomllib

CRATE = "stella-rain-core"
TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
VERSION_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
# The checks the `rc` ruleset requires: release-checks.yml's jobs, and the rust.yml and
# eol-check.yml checks every pull request runs (the same 12 as auto-merge.yml's `require:`).
# A change to either list changes this one in the same pull request.
REQUIRED_CHECKS = (
    "the head is main",
    "version is new",
    "iOS Simulator / tests in the iOS Simulator (debug)",
    "iOS Simulator / tests in the iOS Simulator (release)",
    "long fuzz / fuzz stage_parse",
    "long fuzz / fuzz stage_run",
    "long fuzz / fuzz replay_load",
    "long fuzz / fuzz share_decode",
    "fmt and clippy",
    "tests on x86_64",
    "tests on arm64",
    "tests on windows-x86_64",
    "tests on macos-arm64",
    "cargo check for mobile targets",
    "fuzz crate fmt, clippy and tests",
    "fuzz targets build (nightly)",
    "parsing changed",
    "worst-case tick on x86_64",
    "worst-case tick on arm64",
    "eol / LF line endings",
)


def _key(tag):
    m = TAG_RE.match(tag)
    return tuple(int(x) for x in m.groups()) if m else None


def previous(tags):
    keyed = [(_key(t), t) for t in tags if _key(t)]
    return max(keyed)[1] if keyed else None


def new_version_problems(version, tags):
    m = VERSION_RE.match(version)
    if not m:
        return [f"version {version!r} is not X.Y.Z"]
    tag = f"v{version}"
    if tag in tags:
        return [f"{tag} is already tagged; bump the version in Cargo.toml"]
    last = previous(tags)
    if last and tuple(int(x) for x in m.groups()) <= _key(last):
        return [f"{version} is not above the last release {last}"]
    return []


def cargo_version(cargo_toml):
    return tomllib.loads(cargo_toml)["package"]["version"]


def lock_problems(version, fuzz_lock):
    locked = [p["version"] for p in tomllib.loads(fuzz_lock).get("package", [])
              if p.get("name") == CRATE]
    if not locked:
        return [f"fuzz/Cargo.lock has no {CRATE}"]
    if locked[0] != version:
        return [f"fuzz/Cargo.lock has {CRATE} {locked[0]}, Cargo.toml {version}; "
                f"run cargo update -p {CRATE} in fuzz/"]
    return []


def verify(check_runs):
    """Problems with the required checks; a name passes if any of its runs succeeded."""
    by_name = {}
    for run in check_runs:
        by_name.setdefault(run["name"], []).append(run)
    problems = []
    for name in REQUIRED_CHECKS:
        runs = by_name.get(name, [])
        if not runs:
            problems.append(f"{name}: not reported")
        elif not any(r.get("conclusion") == "success" for r in runs):
            last = runs[-1]
            problems.append(f"{name}: {last.get('conclusion') or last.get('status')}")
    return problems


def notes(tag, commit, prev, pr, checks, log):
    green = {}
    for run in checks:
        if run.get("conclusion") == "success":
            green.setdefault(run["name"], run)
    lines = [f"# {tag}", "", f"Commit `{commit}` on `rc`, from {pr}.", "",
             "## Release checks (ADR-054)", ""]
    lines += [f"- [{n}]({green[n]['html_url']})" for n in REQUIRED_CHECKS if n in green]
    lines += ["", "## Assets", "",
              "- `stage-verify` for Linux x86_64 and arm64, Windows x86_64 and macOS arm64.",
              "- The JSON Schema of the stage, event, input, replay and snapshot types.",
              "", "## Changes", ""]
    if prev:
        lines += [f"Commits since {prev}:", ""]
        lines += [f"- {line}" for line in log if line.strip()]
    else:
        lines.append("First release.")
    return "\n".join(lines) + "\n"


def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def main(argv):
    args = argv[1:]
    if len(args) >= 3 and args[0] == "tag":
        version = cargo_version(_read(args[1]))
        problems = new_version_problems(version, args[3:]) + lock_problems(version, _read(args[2]))
        for problem in problems:
            print(problem)
        if problems:
            return 1
        print(f"v{version}")
        return 0
    if args and args[0] == "previous":
        tag = previous(args[1:])
        if tag:
            print(tag)
        return 0
    if len(args) == 2 and args[0] == "verify":
        problems = verify(json.loads(_read(args[1])))
        for problem in problems:
            print(problem)
        return 1 if problems else 0
    if len(args) == 7 and args[0] == "notes":
        tag, commit, prev, pr, checks_path, log_path = args[1:7]
        print(notes(tag, commit, None if prev == "-" else prev, pr,
                    json.loads(_read(checks_path)), _read(log_path).splitlines()), end="")
        return 0
    print(__doc__.strip().splitlines()[0], file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
