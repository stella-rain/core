"""Tests for fuzz_targets.py and the text that uses it. Run: python3 -m unittest discover -s .github/scripts

The fuzz runs (fuzz-short in rust.yml, fuzz-long.yml) take their targets from the [[bin]] table of
fuzz/Cargo.toml, which fuzz_targets.py checks against fuzz/fuzz_targets/. Temp trees cover the
script; the real tree is read independently; the workflows and rules are read as text and the
list line of the short step is run.
"""

import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SCRIPT = os.path.join(HERE, "fuzz_targets.py")

# Per-target overhead (builds, libFuzzer start: measured 15 to 22 s) and the rest of the job
# (toolchain, caches, seeds: measured about 95 s, plus slack).
OVERHEAD_S = 25
REST_S = 150


def read(*parts):
    with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
        return f.read()


def run(root, cmd):
    return subprocess.run(
        [sys.executable, SCRIPT, "--root", root, cmd], capture_output=True, text=True, check=False
    )


def tree(bins, files):
    """A temp root with fuzz/Cargo.toml holding bins [(name, path)] and the files fuzz_targets/<f>.rs."""
    d = tempfile.mkdtemp()
    os.makedirs(os.path.join(d, "fuzz", "fuzz_targets"))
    toml = '[package]\nname = "x"\n'
    for name, path in bins:
        toml += f'\n[[bin]]\nname = "{name}"\npath = "{path}"\n'
    with open(os.path.join(d, "fuzz", "Cargo.toml"), "w", encoding="utf-8") as f:
        f.write(toml)
    for stem in files:
        with open(os.path.join(d, "fuzz", "fuzz_targets", stem + ".rs"), "w", encoding="utf-8") as f:
            f.write("")
    return d


def good(name):
    return (name, f"fuzz_targets/{name}.rs")


def job_block(text, key):
    """The text of the job `key:` up to the next job key (two-space indent)."""
    m = re.search(rf"^  {re.escape(key)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text, re.S | re.M)
    assert m, f"no job {key}"
    return m.group(0)


ARM = "savepoint_load) seeds=seeds/savepoint ;;"
GEN = (
    'SAVEPOINT_SEEDS_DIR="$PWD/seeds/savepoint" cargo test --manifest-path fuzz/Cargo.toml --locked '
    "--test smoke write_savepoint_seeds -- --ignored"
)


def real_targets():
    with open(os.path.join(ROOT, "fuzz", "Cargo.toml"), "rb") as f:
        return [b["name"] for b in tomllib.load(f)["bin"]]


class RealTree(unittest.TestCase):
    def test_json_is_the_bin_table_and_the_directory(self):
        r = run(ROOT, "json")
        self.assertEqual(r.returncode, 0, r.stderr)
        names = json.loads(r.stdout)
        self.assertEqual(names, real_targets())
        files = {f[:-3] for f in os.listdir(os.path.join(ROOT, "fuzz", "fuzz_targets")) if f.endswith(".rs")}
        self.assertEqual(set(names), files)
        self.assertIn("packs_load", names)
        self.assertIn("minisign_verify", names)

    def test_words_is_the_same_names_on_one_line(self):
        r = run(ROOT, "words")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, " ".join(real_targets()) + "\n")


class TempTrees(unittest.TestCase):
    def fails(self, d, *mentions):
        for cmd in ("json", "words"):
            r = run(d, cmd)
            self.assertEqual(r.returncode, 1, cmd)
            self.assertEqual(r.stdout, "", cmd)
            for m in mentions:
                self.assertIn(m, r.stderr)

    def test_agreeing_tree_keeps_the_table_order(self):
        d = tree([good("b_two"), good("a_one")], ["a_one", "b_two"])
        r = run(d, "json")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), ["b_two", "a_one"])
        self.assertEqual(run(d, "words").stdout, "b_two a_one\n")

    def test_file_without_bin(self):
        self.fails(tree([good("a")], ["a", "extra"]), "extra")

    def test_bin_without_file(self):
        self.fails(tree([good("a"), good("b")], ["a"]), "b")

    def test_wrong_path(self):
        self.fails(tree([("a", "fuzz_targets/other.rs")], ["a"]), "a")

    def test_duplicate_name(self):
        self.fails(tree([good("a"), good("a")], ["a"]), "a")

    def test_bad_names(self):
        for name in ("Capital", "dash-ed", "1digit"):
            self.fails(tree([good(name)], [name]), name)

    def test_empty_table(self):
        self.fails(tree([], []))


def producers(case, block):
    """Every `seeds=seeds/<dir>` arm has `seeds/<dir>` in an earlier line of the job."""
    for m in re.finditer(r"seeds=(seeds/\w+)", block):
        case.assertIn(m.group(1), block[: m.start()], f"{m.group(1)} is not produced before its arm")


class Smoke(unittest.TestCase):
    def test_the_seed_writer_exists(self):
        text = read("fuzz", "tests", "smoke.rs")
        self.assertRegex(text, r"#\[ignore[^\]]*\][^{]*?fn write_savepoint_seeds")
        self.assertIn("SAVEPOINT_SEEDS_DIR", text)


class RustYml(unittest.TestCase):
    def setUp(self):
        self.block = job_block(read(".github", "workflows", "rust.yml"), "fuzz-short")

    def test_list_comes_from_the_script(self):
        self.assertIn("fuzz_targets.py words", self.block)
        self.assertIn("for target in $targets", self.block)
        for line in self.block.splitlines():
            if re.search(r"\bfor\b.*\bin\b", line):
                for t in real_targets():
                    self.assertNotIn(t, line)

    def test_seed_arms_name_real_targets(self):
        targets = real_targets()
        for arm in re.findall(r'^\s+([a-z_*]+)\) seeds=', self.block, re.M):
            if arm == "*":
                continue
            self.assertTrue(
                any(re.fullmatch(arm.replace("*", ".*"), t) for t in targets), f"seed arm {arm} matches no target"
            )

    def test_savepoint_arm_and_step(self):
        self.assertIn(ARM, self.block)
        self.assertIn(GEN, self.block)
        self.assertLess(self.block.index(GEN), self.block.index("fuzz_targets.py words"))

    def test_every_arm_has_a_producer(self):
        producers(self, self.block)

    def test_time_fits_the_timeout(self):
        secs = re.findall(r"-max_total_time=(\d+)", self.block)
        self.assertEqual(len(secs), 1)
        timeout = int(re.search(r"timeout-minutes: (\d+)", self.block).group(1))
        n = len(real_targets())
        need = n * (int(secs[0]) + OVERHEAD_S) + REST_S
        self.assertLessEqual(need, 60 * timeout, f"{n} targets x {secs[0]} s need {need} s of {60 * timeout}")

    def test_list_line_runs(self):
        m = re.search(r"^\s+(targets=\$\(python3 \.github/scripts/fuzz_targets\.py words\))$", self.block, re.M)
        self.assertTrue(m, "no targets=$(...) line")
        r = subprocess.run(
            ["bash", "-c", f"set -eu\n{m.group(1)}\nfor t in $targets; do echo $t; done"],
            cwd=ROOT, capture_output=True, text=True, check=False,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(r.stdout.split()), len(real_targets()))


class FuzzLongYml(unittest.TestCase):
    def setUp(self):
        self.text = read(".github", "workflows", "fuzz-long.yml")

    def test_matrix_from_the_targets_job(self):
        targets = job_block(self.text, "targets")
        self.assertIn("fuzz_targets.py json", targets)
        self.assertRegex(targets, r"permissions:\n\s+contents: read\n")
        self.assertNotIn("write", targets)
        self.assertRegex(self.text, r"(?m)^permissions: \{\}")
        fuzz = job_block(self.text, "fuzz")
        self.assertRegex(fuzz, r"needs: targets\b")
        self.assertIn("fromJSON(needs.targets.outputs.names)", fuzz)
        self.assertNotIn("target: [", self.text)

    def test_report_waits_for_both(self):
        report = job_block(self.text, "report")
        needs = re.search(r"needs: (.*)", report).group(1)
        self.assertIn("targets", needs)
        self.assertIn("fuzz", needs)

    def test_header_and_seed_arms(self):
        self.assertNotRegex(self.text.splitlines()[0], r"\bsix\b")
        targets = real_targets()
        for arm in re.findall(r"^\s+([a-z_*]+)\) seeds=", self.text, re.M):
            if arm != "*":
                self.assertTrue(any(re.fullmatch(arm.replace("*", ".*"), t) for t in targets), arm)

    def test_savepoint_arm_and_step(self):
        fuzz = job_block(self.text, "fuzz")
        self.assertIn(ARM, fuzz)
        self.assertIn(GEN, fuzz)
        self.assertIn("if: matrix.target == 'savepoint_load'", fuzz)
        self.assertLess(fuzz.index(GEN), fuzz.index('cargo "+$FUZZ_NIGHTLY" fuzz run'))

    def test_every_arm_has_a_producer(self):
        producers(self, job_block(self.text, "fuzz"))

    def test_candidate_uses_the_long_workflow(self):
        text = read(".github", "workflows", "candidate.yml")
        self.assertIn("uses: ./.github/workflows/fuzz-long.yml", job_block(text, "long-fuzz"))
        self.assertNotIn("seeds=", text)

    def test_candidate_comment_has_no_count(self):
        self.assertNotIn("6 targets", read(".github", "workflows", "candidate.yml"))


class Rules(unittest.TestCase):
    def test_text(self):
        text = read(".claude", "rules", "benches-fuzz.md")
        flat = " ".join(text.split())
        self.assertIn("every file in `fuzz_targets/`", flat)
        self.assertIn("fuzz_targets.py", text)
        self.assertNotIn("the `fuzz-short` list of `rust.yml`", flat)
        self.assertLessEqual(len(text.splitlines()), 80)
        self.assertIn("savepoint_load", text)
        replay = read(".claude", "rules", "replay.md")
        self.assertIn("tests/savepoint_proof.rs", replay)
        self.assertLessEqual(len(replay.splitlines()), 80)


if __name__ == "__main__":
    unittest.main()
