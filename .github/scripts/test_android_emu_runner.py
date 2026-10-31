"""Tests for android-emu-runner.sh, the Cargo runner of android-emu.yml. Run:
python3 -m unittest discover -s .github/scripts

The script runs against a fake `adb` first on PATH: it logs every call as one JSON line, answers
`test -e` from a marker file that the runner's own `touch` creates, and exits with a settable status.
"""

import json
import os
import shlex
import stat
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "android-emu-runner.sh")

FAKE_ADB = r"""#!/usr/bin/env python3
import json, os, shlex, sys
args = sys.argv[1:]
with open(os.environ["FAKE_ADB_LOG"], "a") as log:
    log.write(json.dumps(args) + "\n")
marker = os.environ["FAKE_ADB_MARKER"]
if args[:1] == ["shell"]:
    cmd = args[1]
    if cmd.startswith("test -e "):
        sys.exit(0 if os.path.exists(marker) else 1)
    if cmd.startswith("touch "):
        open(marker, "w").close()
        sys.exit(0)
    if cmd == "exit 7":
        sys.exit(int(os.environ.get("FAKE_ADB_SELF", "7")))
    if cmd.startswith("cd "):
        sys.exit(int(os.environ.get("FAKE_ADB_EXIT", "0")))
sys.exit(0)
"""

SKIPS = ["stage_verify::", "content_pack::", "content_pack_compare::", "content_pack_unpack::", "tuning::"]


class Runner(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        base = self.tmp.name
        self.root = os.path.join(base, "tree")
        for d in (".git", "target", "src", ".github", ".config"):
            os.makedirs(os.path.join(self.root, d))
        for f in ("Cargo.toml", "rust-toolchain.toml"):
            open(os.path.join(self.root, f), "w").close()
        self.bin = os.path.join(base, "bin")
        os.makedirs(self.bin)
        adb = os.path.join(self.bin, "adb")
        with open(adb, "w") as f:
            f.write(FAKE_ADB)
        os.chmod(adb, os.stat(adb).st_mode | stat.S_IXUSR)
        self.log = os.path.join(base, "adb.log")
        self.marker = os.path.join(base, "marker")
        self.deps = os.path.join(self.root, "target", "x86_64-linux-android", "debug", "deps")

    def run_runner(self, *args, env=None, unset=()):
        e = {k: v for k, v in os.environ.items()
             if k not in ("PROPTEST_CASES", "RUST_BACKTRACE", "RUST_TEST_THREADS", "CARGO_MANIFEST_DIR")}
        e.update(
            PATH=self.bin + os.pathsep + e["PATH"],
            FAKE_ADB_LOG=self.log,
            FAKE_ADB_MARKER=self.marker,
            CARGO_MANIFEST_DIR=self.root,
        )
        e.update(env or {})
        for name in unset:
            e.pop(name, None)
        return subprocess.run(["bash", SCRIPT, *args], env=e, capture_output=True, text=True)

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as f:
            return [json.loads(line) for line in f]

    def shells(self):
        return [c[1] for c in self.calls() if c[0] == "shell"]

    def pushes(self):
        return [c[1:] for c in self.calls() if c[0] == "push"]

    def final_words(self):
        """The words the device shell makes of the last `adb shell` command, after `env`."""
        cmd = self.shells()[-1]
        probe = subprocess.run(
            ["bash", "-c", 'cd() { :; }; env() { printf "%s\\n" "$@"; }; ' + cmd],
            capture_output=True, text=True, check=True)
        return probe.stdout.split("\n")[:-1]

    def test_it_needs_the_manifest_dir(self):
        r = self.run_runner(os.path.join(self.deps, "integration-abc"), unset=("CARGO_MANIFEST_DIR",))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("CARGO_MANIFEST_DIR", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_the_first_call_pushes_the_tree_and_the_binary(self):
        binary = os.path.join(self.deps, "stella_rain_core-abc")
        r = self.run_runner(binary)
        self.assertEqual(r.returncode, 0, r.stderr)
        pushed = sorted(p[0] for p in self.pushes() if p[1] == self.root + "/")
        expected = sorted(os.path.join(self.root, n) for n in
                          ("src", ".github", ".config", "Cargo.toml", "rust-toolchain.toml"))
        self.assertEqual(pushed, expected)
        self.assertIn([binary, binary], self.pushes())
        shells = self.shells()
        self.assertTrue(any(s.startswith("touch ") and ".android-emu-tree" in s for s in shells))
        mk = [s for s in shells if s.startswith("mkdir -p ")]
        for want in (self.root + "/target/tmp", self.root + "/target/corpus-diff", self.deps):
            self.assertTrue(any(want in s for s in mk), want)
        self.assertTrue(any(s.startswith("chmod 755 ") and binary in s for s in shells))
        self.assertTrue(os.path.exists(self.marker))

    def test_a_second_call_does_not_push_the_tree_again(self):
        self.run_runner(os.path.join(self.deps, "stella_rain_core-abc"))
        self.run_runner(os.path.join(self.deps, "integration-abc"))
        trees = [p for p in self.pushes() if p[1] == self.root + "/"]
        self.assertEqual(len(trees), 5)  # once, for the first call only

    def test_the_integration_binary_skips_the_modules_that_start_a_host_process(self):
        self.run_runner(os.path.join(self.deps, "integration-abc"), "--test-threads=2")
        words = self.final_words()
        tail = words[words.index("--test-threads=2"):]
        expected = ["--test-threads=2"]
        for s in SKIPS:
            expected += ["--skip", s]
        self.assertEqual(tail, expected)

    def test_other_binaries_skip_nothing(self):
        self.run_runner(os.path.join(self.deps, "stella_rain_core-abc"))
        self.assertNotIn("--skip", self.final_words())

    def test_forwarded_variables_are_env_words(self):
        self.run_runner(os.path.join(self.deps, "integration-abc"),
                        env={"PROPTEST_CASES": "7", "RUST_BACKTRACE": "1"})
        words = self.final_words()
        self.assertIn("PROPTEST_CASES=7", words)
        self.assertIn("RUST_BACKTRACE=1", words)
        self.assertNotIn("RUST_TEST_THREADS", "".join(words))
        self.assertIn("INSTA_WORKSPACE_ROOT=" + self.root, words)
        self.assertIn("INSTA_UPDATE=no", words)

    def test_unset_variables_are_not_forwarded(self):
        self.run_runner(os.path.join(self.deps, "integration-abc"))
        text = "".join(self.final_words())
        for name in ("PROPTEST_CASES", "RUST_BACKTRACE", "RUST_TEST_THREADS"):
            self.assertNotIn(name, text)

    def test_the_command_starts_outside_the_tree(self):
        self.run_runner(os.path.join(self.deps, "integration-abc"))
        self.assertTrue(self.shells()[-1].startswith("cd /data/local/tmp "))

    def test_a_binary_path_with_a_space_and_a_quote_survives_the_device_shell(self):
        weird = os.path.join(self.deps, "it's here", "integration-abc")
        self.run_runner(weird, "a b")
        words = self.final_words()
        self.assertIn(weird, words)
        self.assertIn("a b", words)

    def test_the_exit_status_of_the_test_binary_is_the_runners(self):
        for status in (0, 7):
            r = self.run_runner(os.path.join(self.deps, "integration-abc"), env={"FAKE_ADB_EXIT": str(status)})
            self.assertEqual(r.returncode, status)

    def test_selfcheck_passes_when_the_exit_code_travels(self):
        r = self.run_runner("--selfcheck")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.shells(), ["exit 7"])
        self.assertEqual(self.pushes(), [])

    def test_selfcheck_fails_when_the_exit_code_is_lost(self):
        r = self.run_runner("--selfcheck", env={"FAKE_ADB_SELF": "0"})
        self.assertEqual(r.returncode, 1)
        self.assertIn("7", r.stderr)


if __name__ == "__main__":
    unittest.main()
