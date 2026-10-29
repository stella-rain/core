"""Tests for corpus_paths.py and the text that uses it. Run: python3 -m unittest discover -s .github/scripts

The CI readers of the replay corpus (fuzz-short, fuzz-long, Candidate) read one layout,
`tests/corpus/current/{stages,runs}`; the frozen `tests/corpus/v<N>/` folders are never read. Temp
trees cover the script; the real tree is counted independently; the workflows and rules are read as
text, and the `frozen corpus unchanged` step is run in a temp git repo.
"""

import os
import re
import shutil
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

    def test_new_layout_never_reads_packs_or_v1(self):
        root, out, r = self.seeds(new_tree)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "2 stage cases, 2 run cases")
        self.check_copied(root, out, "tests/corpus/current/stages", "tests/corpus/current/runs")

    def test_v0_is_gone(self):
        def only_v0(root):
            make_tree(root, "tests/corpus/v0", "tests/corpus/runs/v0")
        _, _, r = self.seeds(only_v0)
        self.assertEqual(r.returncode, 1)
        self.assertIn("no corpus under tests/corpus/current", r.stderr)

    def test_a_leftover_v0_is_never_read(self):
        def both(root):
            make_tree(root, "tests/corpus/v0", "tests/corpus/runs/v0")
            new_tree(root)
        root, out, r = self.seeds(both)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.check_copied(root, out, "tests/corpus/current/stages", "tests/corpus/current/runs")

    def test_no_current_folder_fails(self):
        _, _, r = self.seeds(lambda root: os.makedirs(os.path.join(root, "tests/corpus")))
        self.assertEqual(r.returncode, 1)
        self.assertIn("no corpus under tests/corpus/current", r.stderr)

    def test_stage_case_without_replay_fails_naming_the_folder(self):
        def broken(root):
            new_tree(root)
            os.remove(os.path.join(root, "tests/corpus/current/stages/y/run.replay"))
        _, _, r = self.seeds(broken)
        self.assertEqual(r.returncode, 1)
        self.assertIn("tests/corpus/current/stages/y", r.stderr.replace(os.sep, "/"))

    def test_current_without_case_folders_fails(self):
        def empty(root):
            os.makedirs(os.path.join(root, "tests/corpus/current/stages"))
            os.makedirs(os.path.join(root, "tests/corpus/current/runs"))
        _, _, r = self.seeds(empty)
        self.assertEqual(r.returncode, 1)

    def test_the_script_has_no_v0_and_no_layout_choice(self):
        with open(SCRIPT, encoding="utf-8") as f:
            text = f.read()
        self.assertNotIn("v0", text)
        self.assertNotIn("layout", text)


class StageDir(unittest.TestCase):
    def stage_dir(self, build, name):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        build(tmp.name)
        return run(tmp.name, "stage-dir", name)

    def test_new_layout(self):
        r = self.stage_dir(new_tree, "x")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "tests/corpus/current/stages/x"))

    def test_output_ends_in_a_bare_newline(self):
        # Candidate reads it with $(...) in Git Bash on Windows, which strips \n but not \r.
        r = subprocess.run([sys.executable, SCRIPT, "--root", ROOT, "stage-dir", "clear"],
                           capture_output=True)
        self.assertEqual(r.returncode, 0)
        self.assertNotIn(b"\r", r.stdout)

    def test_absent_name_fails_naming_it(self):
        r = self.stage_dir(new_tree, "nope")
        self.assertEqual(r.returncode, 1)
        self.assertIn("nope", r.stderr)


class RealTree(unittest.TestCase):
    def layout_dirs(self):
        current = os.path.join(ROOT, "tests", "corpus", "current")
        return os.path.join(current, "stages"), os.path.join(current, "runs")

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
            self.assertNotIn("either layout", text, wf)
        self.assertNotIn("version 0 corpus", self.text(".github/workflows/ios-sim.yml"))

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


class FrozenJob(unittest.TestCase):
    STEP = "Nothing in a frozen corpus folder changed"

    def lines(self):
        with open(os.path.join(ROOT, ".github/workflows/rust.yml"), encoding="utf-8") as f:
            return f.read().splitlines()

    def job(self):
        lines = self.lines()
        start = lines.index("  frozen:")
        end = next((i for i in range(start + 1, len(lines)) if re.match(r"  [A-Za-z0-9_-]+:", lines[i])),
                   len(lines))
        return lines[start:end]

    def step_script(self):
        job = self.job()
        i = next(n for n, l in enumerate(job) if l.strip() == f"- name: {self.STEP}")
        j = next(n for n in range(i, len(job)) if job[n].strip() == "run: |")
        indent = len(job[j]) - len(job[j].lstrip()) + 2
        body = []
        for l in job[j + 1:]:
            if l.strip() and len(l) - len(l.lstrip()) < indent:
                break
            body.append(l[indent:])
        return "\n".join(body) + "\n"

    def test_job_text(self):
        text = "\n".join(self.job())
        self.assertIn("name: frozen corpus unchanged", text)
        self.assertIn("uses: actions/checkout@v7", text)
        self.assertIn("fetch-depth: 0", text)
        self.assertIn("persist-credentials: false", text)
        self.assertIn("cargo test --locked --test integration corpus::the_frozen", text)
        self.assertIn("if: always()", text)
        for l in self.job():
            if "uses:" in l:
                self.assertRegex(l, r"@v\d+\s*$")
        i = self.job().index("    permissions:")
        self.assertEqual(self.job()[i + 1].strip(), "contents: read")
        self.assertNotRegex(self.job()[i + 2], r"^      \S")

    def test_workflow_permissions_stay_empty(self):
        self.assertIn("permissions: {}", self.lines())

    def test_rust_checks_waits_for_it_and_requires_it_to_run(self):
        text = "\n".join(self.lines())
        needs = next(l for l in self.lines() if l.strip().startswith("needs:") and "bench-profile" in l)
        self.assertRegex(needs, r"\bfrozen\b")
        self.assertRegex(text, r"frozen\) +runs=true")

    def run_step(self, edit):
        if not (shutil.which("bash") and shutil.which("git")):
            self.skipTest("bash or git is missing")
        with tempfile.TemporaryDirectory() as tmp:
            def git(*args):
                subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                               cwd=tmp, check=True, capture_output=True)
            git("init", "-q")
            for rel in ("v1/manifest.sha256", "v1/stages/a/f", "v0/f", "current/f", "v1a/f", "v01/f", "v12/f"):
                write(os.path.join(tmp, "tests/corpus", rel), b"base")
            git("add", "-A")
            git("commit", "-q", "-m", "base")
            base = subprocess.run(["git", "rev-parse", "HEAD"], cwd=tmp, capture_output=True,
                                  text=True, check=True).stdout.strip()
            edit(tmp)
            git("add", "-A")
            git("commit", "-q", "-m", "change", "--allow-empty")
            return subprocess.run(["bash", "-c", self.step_script()], cwd=tmp, capture_output=True,
                                  text=True, env={**os.environ, "BASE": base})

    def modify(self, rel):
        return lambda tmp: write(os.path.join(tmp, "tests/corpus", rel), b"changed")

    def test_a_changed_frozen_file_fails(self):
        def rename_within(tmp):
            os.rename(os.path.join(tmp, "tests/corpus/v1/stages/a/f"),
                      os.path.join(tmp, "tests/corpus/v1/stages/a/g"))

        def move_out(tmp):
            os.rename(os.path.join(tmp, "tests/corpus/v1/stages/a/f"),
                      os.path.join(tmp, "tests/corpus/current/f2"))

        def delete(tmp):
            os.remove(os.path.join(tmp, "tests/corpus/v1/manifest.sha256"))

        cases = {
            "modified": (self.modify("v1/stages/a/f"), "v1/stages/a/f"),
            "deleted": (delete, "v1/manifest.sha256"),
            "renamed within": (rename_within, "v1/stages/a/"),
            "moved out": (move_out, "v1/stages/a/f"),
            "v12 modified": (self.modify("v12/f"), "v12/f"),
        }
        for name, (edit, shown) in cases.items():
            with self.subTest(name):
                r = self.run_step(edit)
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertIn(shown, r.stderr)

    def test_an_allowed_change_passes(self):
        def new_version(tmp):
            write(os.path.join(tmp, "tests/corpus/v2/stages/a/f"), b"new")
            write(os.path.join(tmp, "tests/corpus/v2/manifest.sha256"), b"new")

        def new_file_in_v1(tmp):
            write(os.path.join(tmp, "tests/corpus/v1/stages/a/new"), b"new")

        cases = {
            "new v2": new_version,
            "new file in v1": new_file_in_v1,
            "current modified": self.modify("current/f"),
            "v0 modified": self.modify("v0/f"),
            "v1a modified": self.modify("v1a/f"),
            "v01 modified": self.modify("v01/f"),
        }
        for name, edit in cases.items():
            with self.subTest(name):
                r = self.run_step(edit)
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


class RulesText(unittest.TestCase):
    def text(self, rel):
        with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
            return f.read()

    def test_replay_rule_names_one_app_read_path(self):
        text = self.text(".claude/rules/replay.md")
        for stale in ("runs/v0", "From CO1", "today", "either layout"):
            self.assertNotIn(stale, text)
        self.assertIn("tests/corpus/current/runs/stage_skill/run.replay", text)
        self.assertIn("frozen", text)
        self.assertIn("v<N>/", text)
        self.assertNotIn("run's `run.replay` and `hashes.bin`", text)
        self.assertLessEqual(len(text.splitlines()), 80)

    def test_determinism_rule_names_the_two_trees(self):
        text = self.text(".claude/rules/determinism.md")
        self.assertIn("tests/corpus/current/", text)
        self.assertIn("never changes", text)
        self.assertNotIn("per-tick hashes", text)

    def test_ci_rule_has_no_layout_choice(self):
        self.assertNotIn("either layout", self.text(".claude/rules/ci.md"))

    def test_stale_per_tick_claims_are_gone(self):
        self.assertNotIn("every tick's hash is checked", self.text("AGENTS.md"))

    def test_layout_row_names_current_and_frozen(self):
        row = next(l for l in self.text("AGENTS.md").splitlines() if l.startswith("| `tests/corpus/`"))
        self.assertIn("current/", row)
        self.assertIn("v<N>/", row)
        self.assertLessEqual(len(self.text("AGENTS.md").splitlines()), 100)

    def test_the_contract_test_has_no_v0(self):
        self.assertNotIn("v0", self.text("tests/corpus_contract.rs"))


if __name__ == "__main__":
    unittest.main()
