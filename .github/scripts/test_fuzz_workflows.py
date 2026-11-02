"""Tests for the fuzz runs' workflow text (fuzz-short in rust.yml, fuzz-long.yml).

Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflows are read as lines. pack_load's seed is the bundled
content pack (content/pack.json), larger than the 64 KiB the other targets take; libFuzzer cuts
a seed to -max_len, so a smaller limit fuzzes a broken pack that fails before the code behind it.
"""

import os
import re
import unittest

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
WORKFLOWS = os.path.join(ROOT, ".github", "workflows")


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


def job_block(text, job_id):
    m = re.search(rf"^  {re.escape(job_id)}:\n((?:(?:   .*)?\n)*)", text, re.M)
    assert m, f"no job {job_id}"
    return m.group(1)


def pack_max_bytes():
    """Pack::MAX_BYTES, from src/pack.rs (a product of integer literals)."""
    m = re.search(r"pub const MAX_BYTES: usize = ([0-9_ *]+);", read("src/pack.rs"))
    assert m, "no Pack::MAX_BYTES in src/pack.rs"
    value = 1
    for factor in m.group(1).split("*"):
        value *= int(factor.strip().replace("_", ""))
    return value


FUZZ_JOBS = [("rust.yml", "fuzz-short"), ("fuzz-long.yml", "fuzz")]


class PackLoadLength(unittest.TestCase):
    def test_pack_load_takes_a_whole_pack_and_one_byte_more(self):
        want = pack_max_bytes() + 1
        self.assertGreaterEqual(want, os.path.getsize(os.path.join(ROOT, "content", "pack.json")))
        for workflow, job in FUZZ_JOBS:
            with self.subTest(workflow=workflow):
                block = job_block(read(f".github/workflows/{workflow}"), job)
                m = re.search(r"^\s+pack_load\)[^\n]*\bmax_len=(\d+)", block, re.M)
                self.assertIsNotNone(m, "pack_load sets no max_len")
                self.assertEqual(int(m.group(1)), want)

    def test_the_run_uses_the_target_s_length(self):
        for workflow, job in FUZZ_JOBS:
            with self.subTest(workflow=workflow):
                block = job_block(read(f".github/workflows/{workflow}"), job)
                self.assertIn('-max_len="$max_len"', block)
                self.assertRegex(block, r"(?m)^\s+max_len=65536$", "the other targets keep 64 KiB")
                self.assertNotIn("-max_len=65536", block)


def step(block, name):
    """The step named `name`, from its `- name:` line to the next step."""
    m = re.search(rf"^      - name: {re.escape(name)}\n((?:(?!      - ).*\n)*)", block, re.M)
    assert m, f"no step '{name}'"
    # Comments above the next step sit before its `- ` line: not part of this one.
    return re.sub(r"(?:^ *#.*\n)+\Z", "", m.group(0), flags=re.M)


SHORT_STEP = "Fuzz each target for 60 seconds, one per CPU"


class ShortRunInParallel(unittest.TestCase):
    """Ten targets one after another took 10 of the job's 13.5 minutes on one of four CPUs."""

    def setUp(self):
        self.step = step(job_block(read(".github/workflows/rust.yml"), "fuzz-short"), SHORT_STEP)

    def test_it_builds_once_then_runs_one_target_per_cpu(self):
        build = self.step.index('cargo "+$FUZZ_NIGHTLY" fuzz build\n')
        parallel = self.step.index('xargs -P "$(nproc)"')
        self.assertLess(build, parallel)
        self.assertIn("python3 .github/scripts/fuzz_targets.py words", self.step)

    def test_each_target_still_runs_60_seconds(self):
        self.assertIn("-max_total_time=60 ", self.step)

    def test_every_log_is_shown_and_a_failure_fails_the_step(self):
        self.assertIn('echo "::group::$target"', self.step)
        self.assertRegex(self.step, r"::error[^\n]*\$target")
        self.assertTrue(self.step.rstrip().endswith('exit "$status"'))


def rust_cache(block):
    """The `with:` lines of the job's Swatinem/rust-cache step, as a dict."""
    m = re.search(r"uses: Swatinem/rust-cache@v2\n        with:\n((?:          .*\n)+)", block)
    assert m, "no rust-cache step"
    return dict(l.strip().split(": ", 1) for l in m.group(1).splitlines())


class ShortRunCache(unittest.TestCase):
    """rust-cache keys a cache by job unless shared-key is set, and saves only on main; the short
    run runs only on PRs, so its own cache was never saved and every run built from nothing."""

    def test_the_short_run_reads_the_build_job_s_cache(self):
        text = read(".github/workflows/rust.yml")
        build = rust_cache(job_block(text, "fuzz-build"))
        short = rust_cache(job_block(text, "fuzz-short"))
        self.assertIn("shared-key", build)
        self.assertEqual(short.get("shared-key"), build["shared-key"])
        self.assertEqual(short.get("key"), build.get("key"))
        self.assertEqual(build["save-if"], "${{ github.ref == 'refs/heads/main' }}")
        self.assertEqual(short["save-if"], build["save-if"])

    def test_the_build_job_runs_on_main(self):
        header = job_block(read(".github/workflows/rust.yml"), "fuzz-build").split("    steps:")[0]
        self.assertNotIn("pull_request", header)


if __name__ == "__main__":
    unittest.main()
