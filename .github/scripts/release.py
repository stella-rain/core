"""release: the commands the release workflows run (the `release-flow` skill; ported from
kade-agent/agent-config's scripts/release.cjs).

A release is the annotated tag `vX.Y.Z`, where X.Y.Z is the crate version in Cargo.toml.
Candidate (.github/workflows/candidate.yml) tags a commit of main `vX.Y.Z-rc.N` and makes the
branch `rc/vX.Y.Z-rc.N`; Promote (promote.yml), run on that branch, adds `vX.Y.Z` on the same
commit. Only a workflow makes a tag.

Usage (run inside the repository; commands that list branches ask origin):
  release.py version                  the final tag for Cargo.toml's version; refuses a version
                                      already tagged or not above the latest release, and a
                                      fuzz/Cargo.lock that disagrees
  release.py rc TAG                   the next candidate tag of TAG: TAG-rc.1, TAG-rc.2, ...
  release.py check-candidate BRANCH   refuses unless BRANCH is candidate/<12-character sha of
                                      HEAD>, HEAD is on origin/main and origin has no other
                                      candidate/* branch
  release.py checks-passed RUN_ID     reads a commit's check runs (the API's JSON) on stdin;
                                      refuses unless one ran, all finished and none failed,
                                      leaving out run RUN_ID's own jobs and the candidate
                                      workflows' jobs from any run
  release.py check-promote RC_TAG     refuses unless RC_TAG is the highest rc of a version not
                                      released yet and above the latest release, and Cargo.toml
                                      at RC_TAG has that version; prints the final tag
  release.py rc-branch BRANCH         the rc tag of a branch rc/<rc tag>
  release.py superseded RC_TAG        origin's rc branches of RC_TAG's version below it
  release.py released TAG             origin's rc branches of version TAG and every older one
  release.py notes RC_TAG COMMIT PREVIOUS|- LOG_FILE
                                      the release notes; LOG_FILE holds one commit per line
                                      since PREVIOUS; starts with a warning when content/pack.json
                                      differs from PREVIOUS's (ADR-080: a content release must follow)
  release.py previous                 the latest release tag, or nothing

Exit status: 0 done; 1 refused (the reason on standard error); 2 usage.
"""

import json
import os
import re
import subprocess
import sys
import tomllib

CRATE = "stella-rain-core"
PACK = "content/pack.json"
TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
VERSION_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
CANDIDATE_RE = re.compile(r"^(v\d+\.\d+\.\d+)-rc\.(\d+)$")
RC_BRANCH_RE = re.compile(r"^rc/(v\d+\.\d+\.\d+-rc\.\d+)$")
CANDIDATE_BRANCH_RE = re.compile(r"^candidate/([0-9a-f]{12})$")
# The jobs of candidate.yml, promote.yml and candidate-guard.yml: their check runs on a main
# commit, from this run or an earlier candidate there (the rc tag is the commit itself), are not
# the commit's CI. The checks candidate.yml calls (iOS Simulator, long fuzz) are named
# "release <workflow> / <job>".
CANDIDATE_JOBS = frozenset({
    "check the candidate",
    "stage-verify on linux-x86_64",
    "stage-verify on linux-arm64",
    "stage-verify on windows-x86_64",
    "stage-verify on macos-arm64",
    "rc tag and prerelease",
    "delete the candidate branch",
    "tag and GitHub release",
    "candidate branch unchanged",
})
CALLED_PREFIX = "release "
FAILED = frozenset({"failure", "timed_out", "cancelled", "action_required", "stale", "startup_failure"})


class ReleaseError(Exception):
    pass


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


def _candidates(final, tags):
    found = [CANDIDATE_RE.match(t) for t in tags]
    return [int(m.group(2)) for m in found if m and m.group(1) == final]


def rc_tag(final, tags):
    if not TAG_RE.match(final or ""):
        raise ReleaseError(f"{final or '(none)'} is not a release tag")
    return f"{final}-rc.{max([0, *_candidates(final, tags)]) + 1}"


def rc_of_branch(branch):
    m = RC_BRANCH_RE.match(branch or "")
    if not m:
        raise ReleaseError(f"{branch or '(none)'} is not rc/<rc tag>")
    return m.group(1)


def _rc_branches(branches):
    return [(b, CANDIDATE_RE.match(m.group(1))) for b in branches if (m := RC_BRANCH_RE.match(b))]


def superseded_branches(branches, rc):
    final, n = CANDIDATE_RE.match(rc).groups()
    return [b for b, m in _rc_branches(branches) if m.group(1) == final and int(m.group(2)) < int(n)]


def released_branches(branches, final):
    return [b for b, m in _rc_branches(branches) if _key(m.group(1)) <= _key(final)]


def check_candidate(branch, head, on_main, others):
    m = CANDIDATE_BRANCH_RE.match(branch)
    if not m:
        raise ReleaseError(f"{branch} is not candidate/<12-character sha>")
    if not head.startswith(m.group(1)):
        raise ReleaseError(f"{branch} names {m.group(1)}, not HEAD {head}")
    if not on_main:
        raise ReleaseError(f"{head} is not on origin/main")
    if others:
        raise ReleaseError(f"{', '.join(others)} is left over; Kade deletes it first")


def checks_passed(runs, run_id):
    """auto-merge.yml merges with GITHUB_TOKEN, which starts no push run (app#176), so main's
    commit may have no check run at all: the PR's head ran the gates. Runs that exist must pass."""
    own = f"/actions/runs/{run_id}/"
    others = [c for c in runs
              if own not in (c.get("details_url") or "") and c["name"] not in CANDIDATE_JOBS
              and not c["name"].startswith(CALLED_PREFIX)]
    bad = [f"{c['name']}: {c['conclusion'] if c['status'] == 'completed' else c['status']}"
           for c in others if c["status"] != "completed" or c.get("conclusion") in FAILED]
    if bad:
        raise ReleaseError(f"checks not passed: {'; '.join(bad)}")


def check_promote(rc, tags, version_at_rc):
    m = CANDIDATE_RE.match(rc or "")
    if not m:
        raise ReleaseError(f"{rc or '(none)'} is not a candidate tag vX.Y.Z-rc.N")
    final, n = m.group(1), int(m.group(2))
    highest = max([n, *_candidates(final, tags)])
    if highest > n:
        raise ReleaseError(f"{final}-rc.{highest} is newer than {rc}")
    if final in tags:
        raise ReleaseError(f"{final} is already released")
    last = previous(tags)
    if last and _key(final) <= _key(last):
        raise ReleaseError(f"{final} is not above the latest release {last}")
    if f"v{version_at_rc}" != final:
        raise ReleaseError(f"Cargo.toml at {rc} has {version_at_rc}, not {final[1:]}")
    return final


def pack_changed(prev_bytes, head_bytes):
    """ADR-080 decision 1: a core release that changes content/pack.json gets a content release
    before an app build ships. A file is None when missing, so a pack the previous tag lacks counts
    as changed."""
    return prev_bytes != head_bytes


def notes(rc, commit, prev, log, pack_changed=False):
    final = CANDIDATE_RE.match(rc).group(1)
    lines = []
    if pack_changed and prev:
        lines += [f"content/pack.json changed since {prev}: publish a stella-rain/content release "
                  "from this tag before an app build pins it (ADR-080)", ""]
    lines += [f"# {final}", "", f"Commit `{commit}` on `main`, tested as `{rc}`.", "",
             "## Assets", "",
             "- `stage-verify` for Linux x86_64 and arm64, Windows x86_64 and macOS arm64.",
             "- The JSON Schema of the stage, event, input, replay and snapshot types.",
             "- `SHA256SUMS` over all of them. Promote releases these files, never rebuilt.",
             "", "## Changes", ""]
    if prev:
        lines += [f"Commits since {prev}:", ""]
        lines += [f"- {line}" for line in log if line.strip()]
    else:
        lines.append("First release.")
    return "\n".join(lines) + "\n"


def _git(*args):
    done = subprocess.run(["git", *args], capture_output=True, text=True)
    if done.returncode:
        raise ReleaseError(f"git {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout.strip()


def _tags():
    return [t for t in _git("tag", "-l", "v*").split("\n") if t]


def _origin_branches(pattern):
    out = _git("ls-remote", "--heads", "origin", pattern)
    return [line.split("refs/heads/", 1)[1] for line in out.split("\n") if line]


def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def _version():
    version = cargo_version(_read("Cargo.toml"))
    problems = new_version_problems(version, _tags()) + lock_problems(version, _read("fuzz/Cargo.lock"))
    if problems:
        raise ReleaseError("; ".join(problems))
    return f"v{version}"


def _check_candidate_here(branch):
    head = _git("rev-parse", "HEAD")
    on_main = subprocess.run(["git", "merge-base", "--is-ancestor", head, "origin/main"],
                             capture_output=True).returncode == 0
    others = [b for b in _origin_branches("candidate/*") if b != branch]
    check_candidate(branch, head, on_main, others)


def _check_promote_here(rc):
    version = cargo_version(_git("show", f"{rc}^{{commit}}:Cargo.toml"))
    return check_promote(rc, _tags(), version)


def _released(tag):
    if not TAG_RE.match(tag or ""):
        raise ReleaseError(f"{tag or '(none)'} is not a release tag")
    return " ".join(released_branches(_origin_branches("rc/*"), tag))


def _pack_at(tag):
    """content/pack.json at TAG as bytes, or None when TAG has no such file."""
    done = subprocess.run(["git", "show", f"{tag}^{{commit}}:{PACK}"], capture_output=True)
    return None if done.returncode else done.stdout


def _notes(rc, commit, prev, log_file):
    prev = None if prev == "-" else prev
    changed = False
    if prev:
        head = open(PACK, "rb").read() if os.path.exists(PACK) else None
        changed = pack_changed(_pack_at(prev), head)
    return notes(rc, commit, prev, _read(log_file).splitlines(), changed)


USAGE = ("usage: release.py version | rc TAG | check-candidate BRANCH | checks-passed RUN_ID | "
         "check-promote RC_TAG | rc-branch BRANCH | superseded RC_TAG | released TAG | "
         "notes RC_TAG COMMIT PREVIOUS|- LOG_FILE | previous")


def main(argv):
    cmd, args = (argv[1], argv[2:]) if len(argv) > 1 else ("", [])
    arity = {"version": 0, "rc": 1, "check-candidate": 1, "checks-passed": 1, "check-promote": 1,
             "rc-branch": 1, "superseded": 1, "released": 1, "notes": 4, "previous": 0}
    if arity.get(cmd) != len(args):
        print(USAGE, file=sys.stderr)
        return 2
    try:
        if cmd == "version":
            print(_version())
        elif cmd == "rc":
            print(rc_tag(args[0], _tags()))
        elif cmd == "check-candidate":
            _check_candidate_here(args[0])
        elif cmd == "checks-passed":
            checks_passed(json.load(sys.stdin).get("check_runs", []), args[0])
        elif cmd == "check-promote":
            print(_check_promote_here(args[0]))
        elif cmd == "rc-branch":
            print(rc_of_branch(args[0]))
        elif cmd == "superseded":
            print(" ".join(superseded_branches(_origin_branches("rc/*"), rc_of_branch(f"rc/{args[0]}"))))
        elif cmd == "released":
            print(_released(args[0]))
        elif cmd == "notes":
            print(_notes(*args), end="")
        else:
            print(previous(_tags()) or "")
    except ReleaseError as e:
        print(f"release: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
