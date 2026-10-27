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


if __name__ == "__main__":
    unittest.main()
