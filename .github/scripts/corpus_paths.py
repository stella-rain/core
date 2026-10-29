#!/usr/bin/env python3
"""Where the replay corpus is, for the CI steps that read it (fuzz-short, fuzz-long, Candidate).

The corpus is tests/corpus/current/{stages,runs}. Folders named packs or v<N> are never read.

  corpus_paths.py [--root DIR] seeds STAGE_DIR REPLAY_DIR
      copy each stage case's stage.json and run.replay, and each run case's run.replay, as fuzz seeds
  corpus_paths.py [--root DIR] stage-dir NAME
      print the folder of the stage case NAME (relative to the root, with /)
"""

import argparse
import os
import shutil
import sys

DEFAULT_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))


class Fail(Exception):
    pass


STAGES = "tests/corpus/current/stages"
RUNS = "tests/corpus/current/runs"


def check(root):
    if not os.path.isdir(os.path.join(root, "tests", "corpus", "current")):
        raise Fail("no corpus under tests/corpus/current")


def cases(root, rel):
    folder = os.path.join(root, rel)
    if not os.path.isdir(folder):
        return []
    return sorted(d for d in os.listdir(folder) if os.path.isdir(os.path.join(folder, d)))


def copy(root, rel, case, src, dst):
    path = os.path.join(root, rel, case, src)
    if not os.path.isfile(path):
        raise Fail(f"{rel}/{case} has no {src}")
    shutil.copyfile(path, dst)


def seeds(root, stage_dir, replay_dir):
    check(root)
    stages, runs = STAGES, RUNS
    stage_cases, run_cases = cases(root, stages), cases(root, runs)
    if not stage_cases or not run_cases:
        raise Fail(f"no case folders under {stages} or {runs}")
    os.makedirs(stage_dir, exist_ok=True)
    os.makedirs(replay_dir, exist_ok=True)
    for case in stage_cases:
        copy(root, stages, case, "stage.json", os.path.join(stage_dir, case + ".json"))
        copy(root, stages, case, "run.replay", os.path.join(replay_dir, case + ".replay"))
    for case in run_cases:
        copy(root, runs, case, "run.replay", os.path.join(replay_dir, f"run-{case}.replay"))
    print(f"{len(stage_cases)} stage cases, {len(run_cases)} run cases")


def stage_dir(root, case):
    check(root)
    stages = STAGES
    if case not in cases(root, stages):
        raise Fail(f"no stage case {case} under {stages}")
    # Bytes, not print: on Windows print writes \r\n, and Git Bash's $(...) keeps the \r.
    sys.stdout.buffer.write(f"{stages}/{case}\n".encode())


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--root", default=DEFAULT_ROOT)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("seeds")
    s.add_argument("stage_dir")
    s.add_argument("replay_dir")
    d = sub.add_parser("stage-dir")
    d.add_argument("name")
    a = p.parse_args()
    try:
        if a.cmd == "seeds":
            seeds(a.root, a.stage_dir, a.replay_dir)
        else:
            stage_dir(a.root, a.name)
    except Fail as e:
        print(f"corpus_paths: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
