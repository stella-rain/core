"""Every generated set is marked generated in .gitattributes, so PR diffs collapse it and
diff_size.py leaves it out of the hand-written count. Run: python3 -m unittest discover -s .github/scripts"""

import os
import re
import subprocess
import unittest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

# Written by `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins`, outside `cargo xtask bless`.
OTHER_GENERATED = ["tests/bench_pins.toml"]

# Hand-written files beside the generated sets: they must stay counted.
HAND_WRITTEN = [
    "tests/corpus/v0/agents/stage.json",
    "tests/corpus/runs/v0/elites/mode.json",
    "src/lib.rs",
    "tests/corpus.rs",
]


def xtask_sets():
    """The (name, path) pairs of `SETS` in xtask/src/main.rs: the sets `cargo xtask bless` writes."""
    with open(os.path.join(ROOT, "xtask", "src", "main.rs"), encoding="utf-8") as f:
        src = f.read()
    block = re.search(r"const SETS: \[\(&str, &str\); \d+\] = \[(.*?)\];", src, re.S)
    return re.findall(r'\("(\w+)", "([^"]+)"\)', block.group(1))


def tracked(prefix):
    """The tracked files at or under `prefix`."""
    out = subprocess.run(
        ["git", "ls-files", "--", prefix], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout
    return out.split()


def generated(paths):
    """{path: True when .gitattributes sets linguist-generated on it}."""
    out = subprocess.run(
        ["git", "check-attr", "linguist-generated", "--", *paths],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    return {line.split(": ")[0]: line.endswith(": true") for line in out.splitlines()}


def attrs(paths, *names):
    """{path: {attribute: value}} from `git check-attr`, for paths tracked or not."""
    out = subprocess.run(
        ["git", "check-attr", *names, "--", *paths],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    result = {}
    for line in out.splitlines():
        path, name, value = line.split(": ", 2)
        result.setdefault(path, {})[name] = value
    return result


def corpus_hashes():
    """The corpus's per-tick hashes, which are the stage cases' only: raw little-endian u64s,
    never text (CH1)."""
    return [p for p in tracked("tests/corpus/") if p.endswith("/hashes.bin")]


class Attributes(unittest.TestCase):
    def test_xtask_still_names_its_sets(self):
        self.assertEqual(
            [name for name, _ in xtask_sets()], ["schema", "corpus", "golden", "pack"]
        )

    def test_every_generated_file_is_marked_generated(self):
        for name, prefix in xtask_sets():
            # The corpus stages and run modes are written by hand; the replays and hashes are not.
            files = [p for p in tracked(prefix) if not p.endswith(("/stage.json", "/mode.json"))]
            self.assertTrue(files, f"{name}: no tracked file under {prefix}")
            unmarked = [p for p, g in generated(files).items() if not g]
            self.assertEqual(unmarked, [], f"{name} ({prefix}) has unmarked files")
        for path in OTHER_GENERATED:
            self.assertTrue(generated([path])[path], path)

    def test_hand_written_files_stay_counted(self):
        for path, g in generated(HAND_WRITTEN).items():
            self.assertFalse(g, path)

    def test_corpus_hashes_are_binary(self):
        hashes = corpus_hashes()
        self.assertTrue(hashes)
        got = attrs(hashes, "binary", "diff", "text", "linguist-generated")
        for path in hashes:
            self.assertEqual(
                got[path],
                {"binary": "set", "diff": "unset", "text": "unset", "linguist-generated": "true"},
                path,
            )

    def test_no_text_hashes_remain(self):
        files = tracked("tests/corpus/")
        self.assertEqual([p for p in files if p.endswith("/hashes.txt")], [])

    def test_stage_cases_keep_per_tick_hashes_and_run_cases_keep_checkpoints(self):
        # A stage case has a stage.json and a hashes.bin (a hash per tick); a run case has
        # neither: its run.replay carries the expected hashes as checkpoints (CP1).
        files = tracked("tests/corpus/")
        folder = lambda suffix: sorted(p[: -len(suffix)] for p in files if p.endswith(suffix))
        stages, hashes, replays = folder("stage.json"), folder("hashes.bin"), folder("run.replay")
        self.assertTrue(replays)
        self.assertEqual(hashes, stages)
        for case in stages:
            self.assertIn(case, replays)
        self.assertTrue([case for case in replays if case not in stages], "no run case")


if __name__ == "__main__":
    unittest.main()
