"""Tests for corpus_paths.py and the text that uses it. Run: python3 -m unittest discover -s .github/scripts

The CI readers of the replay corpus (fuzz-short, fuzz-long, Candidate) take either layout: today's
`tests/corpus/v0` + `tests/corpus/runs/v0`, or `tests/corpus/current/{stages,runs}`. Temp trees
cover both; the real tree is counted independently; the workflows and rules are read as text.
"""

import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SCRIPT = os.path.join(HERE, "corpus_paths.py")

sys.path.insert(0, HERE)
import corpus_paths  # noqa: E402,F401  (its absence is the first red)


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def make_tree(root, stages, runs):
    """Write stage cases (stage.json, run.replay, hashes.bin) and run cases (run.replay) with
    bytes that name their own source, so a copy from the wrong place is seen."""
    for case in ("x", "y"):
        for name in ("stage.json", "run.replay", "hashes.bin"):
            write(os.path.join(root, stages, case, name), f"{stages}/{case}/{name}".encode())
    for case in ("a", "b"):
        write(os.path.join(root, runs, case, "run.replay"), f"{runs}/{case}/run.replay".encode())


def old_tree(root):
    make_tree(root, "tests/corpus/v0", "tests/corpus/runs/v0")


def new_tree(root):
    make_tree(root, "tests/corpus/current/stages", "tests/corpus/current/runs")
    write(os.path.join(root, "tests/corpus/current/stages/x/case.json"), b"{}")
    write(os.path.join(root, "tests/corpus/current/packs/" + "ab" * 32 + ".json"), b"pack")
    write(os.path.join(root, "tests/corpus/v1/stages/z/stage.json"), b"v1 stage")
    write(os.path.join(root, "tests/corpus/v1/stages/z/run.replay"), b"v1 replay")
    write(os.path.join(root, "tests/corpus/v1/runs/q/run.replay"), b"v1 run")
    write(os.path.join(root, "tests/corpus/v1/manifest.sha256"), b"manifest")


def run(root, *args):
    return subprocess.run(
        [sys.executable, SCRIPT, "--root", root, *args], capture_output=True, text=True
    )


def listing(folder):
    return sorted(os.listdir(folder))


def read(path):
    with open(path, "rb") as f:
        return f.read()


class Seeds(unittest.TestCase):
    def seeds(self, build):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = os.path.join(tmp.name, "repo")
        build(root)
        out = os.path.join(tmp.name, "out")
        r = run(root, "seeds", os.path.join(out, "stage"), os.path.join(out, "replay"))
        return root, out, r

    def check_copied(self, root, out, stages, runs):
        self.assertEqual(listing(os.path.join(out, "replay")),
                         ["run-a.replay", "run-b.replay", "x.replay", "y.replay"])
        self.assertEqual(listing(os.path.join(out, "stage")), ["x.json", "y.json"])
        for case in ("x", "y"):
            self.assertEqual(read(os.path.join(out, "stage", case + ".json")),
                             read(os.path.join(root, stages, case, "stage.json")))
            self.assertEqual(read(os.path.join(out, "replay", case + ".replay")),
                             read(os.path.join(root, stages, case, "run.replay")))
        for case in ("a", "b"):
            self.assertEqual(read(os.path.join(out, "replay", f"run-{case}.replay")),
                             read(os.path.join(root, runs, case, "run.replay")))

    def test_old_layout(self):
        root, out, r = self.seeds(old_tree)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "2 stage cases, 2 run cases (layout: v0)")
        self.check_copied(root, out, "tests/corpus/v0", "tests/corpus/runs/v0")

    def test_new_layout_never_reads_packs_or_v1(self):
        root, out, r = self.seeds(new_tree)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "2 stage cases, 2 run cases (layout: current)")
        self.check_copied(root, out, "tests/corpus/current/stages", "tests/corpus/current/runs")

    def test_equal_sources_give_equal_folders(self):
        results = []
        for stages, runs in (("tests/corpus/v0", "tests/corpus/runs/v0"),
                             ("tests/corpus/current/stages", "tests/corpus/current/runs")):
            tmp = tempfile.TemporaryDirectory()
            self.addCleanup(tmp.cleanup)
            for case in ("x", "y"):
                for name in ("stage.json", "run.replay", "hashes.bin"):
                    write(os.path.join(tmp.name, stages, case, name), f"{case}{name}".encode())
            for case in ("a", "b"):
                write(os.path.join(tmp.name, runs, case, "run.replay"), f"{case}run".encode())
            out = os.path.join(tmp.name, "out")
            r = run(tmp.name, "seeds", out + "/stage", out + "/replay")
            self.assertEqual(r.returncode, 0, r.stderr)
            results.append({
                sub: {n: read(os.path.join(out, sub, n)) for n in listing(os.path.join(out, sub))}
                for sub in ("stage", "replay")
            })
        self.assertEqual(results[0], results[1])

    def test_both_layouts_fail(self):
        def both(root):
            old_tree(root)
            new_tree(root)
        _, _, r = self.seeds(both)
        self.assertEqual(r.returncode, 1)
        self.assertIn("both corpus layouts", r.stderr)

    def test_neither_layout_fails(self):
        _, _, r = self.seeds(lambda root: os.makedirs(os.path.join(root, "tests/corpus")))
        self.assertEqual(r.returncode, 1)

    def test_stage_case_without_replay_fails_naming_the_folder(self):
        def broken(root):
            old_tree(root)
            os.remove(os.path.join(root, "tests/corpus/v0/y/run.replay"))
        _, _, r = self.seeds(broken)
        self.assertEqual(r.returncode, 1)
        self.assertIn("tests/corpus/v0/y", r.stderr.replace(os.sep, "/"))

    def test_current_without_case_folders_fails(self):
        def empty(root):
            os.makedirs(os.path.join(root, "tests/corpus/current/stages"))
            os.makedirs(os.path.join(root, "tests/corpus/current/runs"))
        _, _, r = self.seeds(empty)
        self.assertEqual(r.returncode, 1)


class StageDir(unittest.TestCase):
    def stage_dir(self, build, name):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        build(tmp.name)
        return run(tmp.name, "stage-dir", name)

    def test_old_layout(self):
        r = self.stage_dir(old_tree, "x")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "tests/corpus/v0/x"))

    def test_new_layout(self):
        r = self.stage_dir(new_tree, "x")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "tests/corpus/current/stages/x"))

    def test_absent_name_fails_naming_it(self):
        r = self.stage_dir(old_tree, "nope")
        self.assertEqual(r.returncode, 1)
        self.assertIn("nope", r.stderr)


class RealTree(unittest.TestCase):
    def layout_dirs(self):
        corpus = os.path.join(ROOT, "tests", "corpus")
        if os.path.isdir(os.path.join(corpus, "current")):
            return (os.path.join(corpus, "current", "stages"),
                    os.path.join(corpus, "current", "runs"))
        return os.path.join(corpus, "v0"), os.path.join(corpus, "runs", "v0")

    def count(self, folder, name):
        return sum(os.path.isfile(os.path.join(folder, d, name)) for d in os.listdir(folder))

    def test_seeds_count_matches_the_folders(self):
        stages, runs = self.layout_dirs()
        with tempfile.TemporaryDirectory() as tmp:
            r = run(ROOT, "seeds", os.path.join(tmp, "stage"), os.path.join(tmp, "replay"))
            self.assertEqual(r.returncode, 0, r.stderr)
            n_stage, n_run = self.count(stages, "stage.json"), self.count(runs, "run.replay")
            self.assertGreater(n_stage, 0)
            self.assertGreater(n_run, 0)
            self.assertEqual(len(listing(os.path.join(tmp, "stage"))), n_stage)
            self.assertEqual(len(listing(os.path.join(tmp, "replay"))), n_stage + n_run)

    def test_clear_is_a_complete_stage_case(self):
        r = run(ROOT, "stage-dir", "clear")
        self.assertEqual(r.returncode, 0, r.stderr)
        folder = os.path.join(ROOT, r.stdout.strip())
        for name in ("stage.json", "run.replay", "hashes.bin"):
            self.assertTrue(os.path.isfile(os.path.join(folder, name)), name)


class WorkflowText(unittest.TestCase):
    def text(self, rel):
        with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
            return f.read()

    def test_readers_go_through_the_script(self):
        for wf in ("rust.yml", "fuzz-long.yml", "candidate.yml"):
            text = self.text(f".github/workflows/{wf}")
            self.assertIn("corpus_paths.py", text, wf)
            self.assertNotIn("tests/corpus/v0", text, wf)
            self.assertNotIn("runs/v0", text, wf)

    def test_candidate_keeps_hashes_beside_the_stage(self):
        text = self.text(".github/workflows/candidate.yml")
        self.assertIn("corpus_paths.py stage-dir clear", text)
        self.assertIn('--hashes "$case/hashes.bin"', text)

    def test_seed_steps_keep_their_other_seeds(self):
        for wf in ("rust.yml", "fuzz-long.yml"):
            text = self.text(f".github/workflows/{wf}")
            self.assertIn("corpus_paths.py seeds seeds/stage seeds/replay", text, wf)
            self.assertIn("cp content/*.json seeds/stage/", text, wf)
            self.assertIn("cp tests/fixtures/run_setup/*.json seeds/setup/", text, wf)
            self.assertIn("content-pack -- export", text, wf)


class RulesText(unittest.TestCase):
    def text(self, rel):
        with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
            return f.read()

    def test_replay_rule_names_both_app_read_paths(self):
        text = self.text(".claude/rules/replay.md")
        self.assertIn("tests/corpus/runs/v0/stage_skill/run.replay", text)
        self.assertIn("tests/corpus/current/runs/stage_skill/run.replay", text)
        self.assertNotIn("run's `run.replay` and `hashes.bin`", text)
        self.assertLessEqual(len(text.splitlines()), 80)

    def test_stale_per_tick_claims_are_gone(self):
        self.assertNotIn("per-tick hashes", self.text(".claude/rules/determinism.md"))
        self.assertNotIn("every tick's hash is checked", self.text("AGENTS.md"))

    def test_layout_row_names_no_folder(self):
        row = next(l for l in self.text("AGENTS.md").splitlines() if l.startswith("| `tests/corpus/`"))
        for word in ("v0", "current/", "v1"):
            self.assertNotIn(word, row)
        self.assertLessEqual(len(self.text("AGENTS.md").splitlines()), 100)


if __name__ == "__main__":
    unittest.main()
