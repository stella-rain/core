"""release: the commands the release workflows run (the `release-flow` skill; ported from
kade-agent/agent-config's scripts/release.cjs).

A release is the annotated tag `vX.Y.Z`. The crate version in Cargo.toml is a floor (release-flow
plan 9.2): above the latest final tag it is the release, at or below it the release is that tag's
next patch, which Candidate stamps into a release commit on top of the main commit. Candidate
(.github/workflows/candidate.yml) runs on main, tags the tested commit `vX.Y.Z-rc.N` and makes the
branch `rc/vX.Y.Z-rc.N`; Promote (promote.yml), run on that branch, adds `vX.Y.Z` on the same
commit. Only a workflow makes a tag. Nothing writes to main.

Usage (run inside the repository; commands that list branches ask origin):
  release.py final                    the final tag of HEAD: Cargo.toml's version when it is above
                                      the latest final tag (or there is none), else that tag with
                                      the patch plus one; refuses locks that disagree with Cargo.toml
  release.py stamp FINAL              writes FINAL's version into Cargo.toml, Cargo.lock and
                                      fuzz/Cargo.lock (the files that repeat it)
  release.py check-main REF           refuses unless REF (GITHUB_REF) is refs/heads/main
  release.py check-release            refuses unless HEAD has a commit since the latest final tag's
                                      main commit ("nothing to release") and is not already a candidate
  release.py rc TAG                   the next candidate tag of TAG: TAG-rc.1, TAG-rc.2, ...
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
                                      the release notes; the title line is "core vX.Y.Z · rules vN",
                                      N being SIM_VERSION in src/version.rs at COMMIT (refuses when
                                      it cannot be read); LOG_FILE holds one commit per line
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
VERSION_RS = "src/version.rs"
SIM_VERSION_RE = re.compile(r"^pub const SIM_VERSION: u32 = (\d+);\s*$", re.M)
TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
VERSION_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
CANDIDATE_RE = re.compile(r"^(v\d+\.\d+\.\d+)-rc\.(\d+)$")
RC_BRANCH_RE = re.compile(r"^rc/(v\d+\.\d+\.\d+-rc\.\d+)$")
# The commit Candidate makes when the final tag is not Cargo.toml's version: its subject, and the
# only files it may change.
RELEASE_SUBJECT_RE = re.compile(r"^Release v\d+\.\d+\.\d+$")
STAMPED = ("Cargo.toml", "Cargo.lock", "fuzz/Cargo.lock")
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
    # auto-candidate.yml: a failed or unfinished auto run never makes Candidate refuse the commit.
    "decide on a candidate",
    "push the candidate branch",
})
CALLED_PREFIX = "release "
# auto-merge.yml's job ("<caller job> / <called job>"): it runs on main's commit after a merge,
# not as a gate, and its concurrency group cancels it when a newer run starts. Candidate run
# 39693572498 failed on four such cancelled runs. Left out whatever its conclusion.
AUTO_MERGE_JOBS = frozenset({"merge / merge when every check is green"})
FAILED = frozenset({"failure", "timed_out", "cancelled", "action_required", "stale", "startup_failure"})


class ReleaseError(Exception):
    pass


def _key(tag):
    m = TAG_RE.match(tag)
    return tuple(int(x) for x in m.groups()) if m else None


def previous(tags):
    keyed = [(_key(t), t) for t in tags if _key(t)]
    return max(keyed)[1] if keyed else None


def final_tag(version, tags):
    """The version is a floor: above the latest final tag it is the release, else that tag's
    next patch (plan 9.2)."""
    m = VERSION_RE.match(version)
    if not m:
        raise ReleaseError(f"version {version!r} is not X.Y.Z")
    last = previous(tags)
    if last is None or tuple(int(x) for x in m.groups()) > _key(last):
        return f"v{version}"
    x, y, z = _key(last)
    return f"v{x}.{y}.{z + 1}"


def check_main(ref):
    if ref != "refs/heads/main":
        raise ReleaseError(f"a candidate is made from refs/heads/main only; this run is on {ref or '(none)'}")


def cargo_version(cargo_toml):
    return tomllib.loads(cargo_toml)["package"]["version"]


def lock_problems(version, lock, path="fuzz/Cargo.lock"):
    locked = [p["version"] for p in tomllib.loads(lock).get("package", [])
              if p.get("name") == CRATE]
    if not locked:
        return [f"{path} has no {CRATE}"]
    if locked[0] != version:
        return [f"{path} has {CRATE} {locked[0]}, Cargo.toml {version}; "
                f"run cargo update -p {CRATE} (in {os.path.dirname(path) or '.'})"]
    return []


def stamp_cargo_toml(text, version):
    """Cargo.toml with [package] version set; the dependencies' versions stay."""
    out, n = re.subn(r'(\[package\].*?^version\s*=\s*")[^"]*(")', rf"\g<1>{version}\g<2>", text,
                     count=1, flags=re.S | re.M)
    if not n:
        raise ReleaseError("Cargo.toml has no [package] version")
    return out


def stamp_lock(text, version):
    """A Cargo.lock with the crate's own entry set to VERSION."""
    out, n = re.subn(rf'(name = "{re.escape(CRATE)}"\nversion = ")[^"]*(")', rf"\g<1>{version}\g<2>", text,
                     count=1)
    if not n:
        raise ReleaseError(f"lock file has no {CRATE}")
    return out


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


def checks_passed(runs, run_id):
    """auto-merge.yml merges with GITHUB_TOKEN, which starts no push run (app#176), so main's
    commit may have no check run at all: the PR's head ran the gates. Runs that exist must pass."""
    own = f"/actions/runs/{run_id}/"
    others = [c for c in runs
              if own not in (c.get("details_url") or "") and c["name"] not in CANDIDATE_JOBS
              and c["name"] not in AUTO_MERGE_JOBS
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


def sim_version(source):
    """SIM_VERSION as declared in src/version.rs; exactly one declaration, a plain number."""
    found = SIM_VERSION_RE.findall(source)
    if len(found) != 1:
        raise ReleaseError(f'{VERSION_RS} has no single line "pub const SIM_VERSION: u32 = <number>;"')
    return int(found[0])


def notes(rc, commit, prev, log, pack_changed=False, *, rules):
    final = CANDIDATE_RE.match(rc).group(1)
    lines = []
    if pack_changed and prev:
        lines += [f"content/pack.json changed since {prev}: publish a stella-rain/content release "
                  "from this tag before an app build pins it (ADR-080)", ""]
    lines += [f"# core {final} \u00b7 rules v{rules}", "", f"Commit `{commit}` on `main`, tested as `{rc}`.", "",
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


def _final():
    version = cargo_version(_read("Cargo.toml"))
    problems = (lock_problems(version, _read("fuzz/Cargo.lock"))
                + lock_problems(version, _read("Cargo.lock"), "Cargo.lock"))
    if problems:
        raise ReleaseError("; ".join(problems))
    return final_tag(version, _tags())


def _stamp(final):
    if not TAG_RE.match(final or ""):
        raise ReleaseError(f"{final or '(none)'} is not a release tag")
    version = final[1:]
    for path, fn in (("Cargo.toml", stamp_cargo_toml), ("Cargo.lock", stamp_lock),
                     ("fuzz/Cargo.lock", stamp_lock)):
        text = fn(_read(path), version)
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(text)


def _main_commit(tag):
    """The commit of main a tag was made from: the tag's own commit, or its parent when that is
    the release commit Candidate stamped (subject `Release vX.Y.Z`, only the stamped files)."""
    commit = _git("rev-parse", f"{tag}^{{commit}}")
    parents = _git("rev-list", "--parents", "-n", "1", commit).split()[1:]
    if len(parents) == 1 and RELEASE_SUBJECT_RE.match(_git("log", "-1", "--format=%s", commit)):
        files = _git("diff-tree", "--no-commit-id", "--name-only", "-r", commit).split("\n")
        if files and set(files) <= set(STAMPED):
            return parents[0]
    return commit


def _check_release_here():
    head = _git("rev-parse", "HEAD")
    tags = _tags()
    for t in tags:
        if CANDIDATE_RE.match(t) and _main_commit(t) == head:
            raise ReleaseError(f"{head[:12]} is already a candidate ({t})")
    last = previous(tags)
    if last and _git("rev-list", "--count", f"{_main_commit(last)}..{head}") == "0":
        raise ReleaseError(f"nothing to release: no commit since {last}")


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


def _sim_version_at(commit):
    """SIM_VERSION in src/version.rs at COMMIT, never the working tree."""
    try:
        return sim_version(_git("show", f"{commit}^{{commit}}:{VERSION_RS}"))
    except ReleaseError as e:
        raise ReleaseError(f"{e} (at {commit})") from None


def _notes(rc, commit, prev, log_file):
    prev = None if prev == "-" else prev
    rules = _sim_version_at(commit)
    changed = False
    if prev:
        head = open(PACK, "rb").read() if os.path.exists(PACK) else None
        changed = pack_changed(_pack_at(prev), head)
    return notes(rc, commit, prev, _read(log_file).splitlines(), changed, rules=rules)


USAGE = ("usage: release.py final | stamp FINAL | check-main REF | check-release | rc TAG | checks-passed RUN_ID | "
         "check-promote RC_TAG | rc-branch BRANCH | superseded RC_TAG | released TAG | "
         "notes RC_TAG COMMIT PREVIOUS|- LOG_FILE | previous")


def main(argv):
    cmd, args = (argv[1], argv[2:]) if len(argv) > 1 else ("", [])
    arity = {"final": 0, "stamp": 1, "check-main": 1, "check-release": 0, "rc": 1, "checks-passed": 1, "check-promote": 1,
             "rc-branch": 1, "superseded": 1, "released": 1, "notes": 4, "previous": 0}
    if arity.get(cmd) != len(args):
        print(USAGE, file=sys.stderr)
        return 2
    try:
        if cmd == "final":
            print(_final())
        elif cmd == "stamp":
            _stamp(args[0])
        elif cmd == "check-main":
            check_main(args[0])
        elif cmd == "check-release":
            _check_release_here()
        elif cmd == "rc":
            print(rc_tag(args[0], _tags()))
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
