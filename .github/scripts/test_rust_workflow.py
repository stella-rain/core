"""Tests for rust.yml's test job. Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflow is read as lines. The x86_64 test job also runs the
ignored command-cap measurement, so a rise of the longest bot run fails in the PR that causes it.
"""

import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
WORKFLOWS = os.path.join(ROOT, ".github", "workflows")
STEP_NAME = "Command cap measurement (longest bot run)"
RELEASE_RUN = "cargo nextest run --locked --release --profile ci"
CAP_RUN = RELEASE_RUN + " --run-ignored only -E 'test(=command_cap::measure_the_longest_run)'"


def read(name):
    with open(os.path.join(WORKFLOWS, name), encoding="utf-8") as f:
        return f.read()


def read_root(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


def job_block(text, job_id):
    m = re.search(rf"^  {re.escape(job_id)}:\n((?:(?:   .*)?\n)*)", text, re.M)
    assert m, f"no job {job_id}"
    return m.group(1)


def step(block, name):
    """The step named `name`, from its `- name:` line to the next step."""
    m = re.search(
        rf"^      - name: {re.escape(name)}\n((?:(?!      - ).*\n)*)", block, re.M
    )
    assert m, f"no step '{name}'"
    # Comments above the next step sit before its `- ` line: not part of this one.
    return re.sub(r"(?:^ *#.*\n)+\Z", "", m.group(0), flags=re.M)


def script_after(block, anchor):
    """The `run: |` script of the step that starts at the line `anchor`, dedented."""
    lines = block.splitlines()
    i = lines.index(anchor)
    j = next(n for n in range(i, len(lines)) if lines[n].strip() == "run: |")
    indent = len(lines[j]) - len(lines[j].lstrip()) + 2
    body = []
    for l in lines[j + 1:]:
        if l.strip() and len(l) - len(l.lstrip()) < indent:
            break
        body.append(l[indent:])
    return "\n".join(body) + "\n"


class CommandCapStep(unittest.TestCase):
    def setUp(self):
        self.text = read("rust.yml")
        self.test = job_block(self.text, "test")

    def test_the_test_job_runs_the_command_cap_measurement(self):
        s = step(self.test, STEP_NAME)
        self.assertEqual(re.findall(r"^\s+(?:- )?if:.*$", s, re.M), ["        if: matrix.arch == 'x86_64' && matrix.profile == 'release'"])
        self.assertIn(CAP_RUN, s)
        for banned in ("--all-features", "--run-ignored all", "continue-on-error"):
            self.assertNotIn(banned, s)

    def test_it_reuses_the_release_build(self):
        lines = self.test.splitlines()
        plain = lines.index("        run: " + RELEASE_RUN)
        self.assertEqual(lines[plain - 1], "      - if: matrix.profile == 'release'")
        cap = next(i for i, l in enumerate(lines) if l == f"      - name: {STEP_NAME}")
        self.assertLess(plain, cap)

    def test_it_is_gated_like_the_other_test_steps(self):
        header = self.test.split("    steps:")[0]
        self.assertIn("needs: changes", header)
        self.assertIn("if: needs.changes.outputs.code == 'true'", header)
        self.assertRegex(job_block(self.text, "checks"), r"needs:.*\btest\b")
        self.assertIn(r"grep -qv '\.md$'", job_block(self.text, "changes"))

    def test_the_failure_tells_the_author_to_re_pin_in_the_pr(self):
        s = step(self.test, STEP_NAME)
        self.assertRegex(s, r"::error[^\n]*MEASURED_MOST")
        self.assertIn("in this PR", s)
        self.assertIn("tests/command_cap.rs", s)
        self.assertTrue(s.rstrip().endswith('exit "$status"'))

    def test_the_junit_files_stay_apart(self):
        s = step(self.test, STEP_NAME)
        first = s.index("mv target/nextest/ci/junit.xml target/nextest/ci/junit-release.xml")
        run = s.index(CAP_RUN)
        last = s.index("mv target/nextest/ci/junit.xml target/nextest/ci/junit-command-cap.xml")
        self.assertLess(first, run)
        self.assertLess(run, last)
        self.assertIn("path: target/nextest/ci/junit*.xml", self.test)

    def test_the_test_it_names_exists_and_says_re_pin(self):
        src = read_root("tests/command_cap.rs")
        self.assertRegex(src, r"#\[ignore[^\n]*\]\nfn measure_the_longest_run")
        self.assertIn("re-pin MEASURED_MOST", src)
        self.assertIn("--run-ignored all", read("deep.yml"))

    def test_the_pin_is_exact_so_a_fall_re_pins_too(self):
        src = read_root("tests/command_cap.rs")
        self.assertIn("most == MEASURED_MOST,", src)
        self.assertNotIn("most <= MEASURED_MOST", src)

    def test_workflow_rules_hold(self):
        lines = self.text.splitlines()
        self.assertLess(lines.index("permissions: {}"), lines.index("jobs:"))
        self.assertIn("contents: read", self.test)
        for i, l in enumerate(lines):
            if "uses: actions/checkout@" in l:
                self.assertIn("persist-credentials: false", "\n".join(lines[i : i + 6]))


class CiSpeed(unittest.TestCase):
    """The CI cost rules of rust.yml: one toolchain install, no duplicate builds, pushes to main."""

    def setUp(self):
        self.text = read("rust.yml")

    def test_one_toolchain_install_that_reads_rust_toolchain_toml(self):
        self.assertNotIn("toolchain install stable", self.text)
        self.assertNotIn("default stable", self.text)
        for job in ("lint", "bless", "test", "cross", "net", "fuzz-stable", "bench",
                    "fuzz-build", "fuzz-short"):
            block = job_block(self.text, job)
            lines = block.split("    steps:\n", 1)[1].splitlines()
            self.assertIn("      - run: rustup toolchain install", lines, job)
            install = lines.index("      - run: rustup toolchain install")
            later = [i for i, l in enumerate(lines)
                     if "Swatinem/rust-cache" in l or re.search(r"\bcargo\b", l)]
            self.assertTrue(later and install < min(later), job)
        for job in ("fuzz-build", "fuzz-short"):
            self.assertIn('rustup toolchain install "$FUZZ_NIGHTLY" --profile minimal',
                          job_block(self.text, job))
        for job in ("cross", "net"):
            self.assertNotIn("--target", "\n".join(
                l for l in job_block(self.text, job).splitlines() if "rustup" in l), job)

    def test_cargo_deny_is_one_job_on_the_pinned_binary(self):
        block = job_block(self.text, "cargo-deny")
        self.assertNotIn("strategy", block)
        self.assertNotIn("cargo-deny-action", block)
        self.assertIn('CARGO_DENY_VERSION: "0.20.2"', block)
        self.assertRegex(block, r'CARGO_DENY_SHA256: "?[0-9a-f]{64}"?\n')
        self.assertLess(block.index("sha256sum -c"), block.index("tar -xzf"))
        self.assertIn("      - run: rustup toolchain install\n", block)
        for manifest in ("Cargo.toml", "fuzz/Cargo.toml"):
            self.assertIn(f"cargo deny --manifest-path {manifest} check", block)
        self.assertRegex(job_block(self.text, "checks"), r"needs:.*\bcargo-deny\b")

    def test_fuzz_build_is_skipped_when_the_short_run_builds_the_same_targets(self):
        header = job_block(self.text, "fuzz-build").split("    steps:")[0]
        self.assertIn("needs: [changes, fuzz-changes]", header)
        self.assertIn("!cancelled()", header)
        self.assertIn("needs.changes.outputs.code == 'true'", header)
        self.assertIn("needs.fuzz-changes.outputs.parsing != 'true'", header)
        # It still saves the nightly cache on main, which the short run only reads.
        self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' }}", job_block(self.text, "fuzz-build"))

    def test_the_frozen_manifest_test_runs_in_every_tests_leg(self):
        frozen = job_block(self.text, "frozen")
        self.assertIn("fetch-depth: 0", frozen)
        self.assertIn("Nothing in a frozen corpus folder changed", frozen)
        self.assertIn("github.event_name == 'pull_request'", frozen)
        # The unfiltered release run covers it: no filter, no skip.
        lines = job_block(self.text, "test").splitlines()
        i = lines.index("        run: " + RELEASE_RUN)
        self.assertEqual(lines[i - 1], "      - if: matrix.profile == 'release'")
        src = read_root("tests/corpus.rs")
        self.assertRegex(src, r"#\[test\]\nfn the_frozen_corpus_is_unchanged")
        self.assertNotRegex(src, r"#\[ignore[^\n]*\]\nfn the_frozen_corpus_is_unchanged")

    def test_the_bench_head_builds_where_rust_cache_keeps_the_target(self):
        block = job_block(self.text, "bench")
        self.assertNotIn("echo \"CARGO_TARGET_DIR=", block)
        self.assertIn('CARGO_TARGET_DIR="$RUNNER_TEMP/target-base" cargo bench', block)
        cache = block[block.index("uses: Swatinem/rust-cache@v2"):]
        cache = cache.split("      - name:")[0]
        self.assertIn("key: bench\n", cache)
        self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' }}", cache)

    def test_debug_and_release_tests_are_parallel_legs(self):
        block = job_block(self.text, "test")
        legs = re.findall(
            r"- arch: (\S+)\n\s+runner: (\S+)\n\s+profile: (\S+)\n", block)
        self.assertEqual(legs, [
            ("x86_64", "ubuntu-latest", "debug"),
            ("x86_64", "ubuntu-latest", "release"),
            ("arm64", "ubuntu-24.04-arm", "debug"),
            ("arm64", "ubuntu-24.04-arm", "release"),
            ("windows-x86_64", "windows-latest", "release"),
            ("macos-arm64", "macos-latest", "release"),
        ])
        self.assertNotIn("matrix.debug", block)
        self.assertIn("name: tests on ${{ matrix.arch }} (${{ matrix.profile }})", block)
        debug_run = "cargo nextest run --locked --all-features --profile ci"
        i = block.index(debug_run)
        self.assertIn("if: matrix.profile == 'debug'", block[:i].split("      - ")[-1])
        self.assertNotIn("mv target/nextest", block[i:].split("      - ")[0])
        doc = block.index("cargo test --doc --locked --all-features")
        self.assertIn("matrix.arch == 'x86_64' && matrix.profile == 'debug'",
                      block[:doc].split("      - ")[-1])
        cache = block[block.index("uses: Swatinem/rust-cache@v2"):].split("      - ")[0]
        self.assertIn("key: ${{ matrix.profile }}", cache)
        self.assertIn("name: junit-${{ matrix.arch }}-${{ matrix.profile }}", block)
        self.assertIn("name: corpus-diff-${{ matrix.arch }}-${{ matrix.profile }}", block)


SUITE_JOBS = ("lint", "bless", "test", "cross", "net", "fuzz-stable", "fuzz-build", "cargo-deny")
ALL_JOBS = ("changes", "fuzz-changes", "lint", "bless", "frozen", "test", "cross", "net", "fuzz-stable",
            "fuzz-build", "fuzz-short", "bench-list", "bench", "bench-profile", "cargo-deny")
ZEROS = "0" * 40


class PushGating(unittest.TestCase):
    """A push to main runs the suite only when a cache-refreshing file changed (the bench always)."""

    def setUp(self):
        if not all(shutil.which(t) for t in ("bash", "git", "jq")):
            self.skipTest("bash, git or jq is missing")
        self.text = read("rust.yml")

    def git(self, tmp, *args):
        return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=tmp,
                              check=True, capture_output=True, text=True).stdout.strip()

    def changes(self, event, files=(), before="parent", base_ref="main", head_ref="feature"):
        """Run the `what changed` script after a commit that writes `files`; returns its outputs."""
        script = script_after(job_block(self.text, "changes"), "      - id: diff")
        with tempfile.TemporaryDirectory() as tmp:
            self.git(tmp, "init", "-q")
            for name in ("README.md", "src/lib.rs"):
                os.makedirs(os.path.dirname(os.path.join(tmp, name)), exist_ok=True)
                with open(os.path.join(tmp, name), "w") as f:
                    f.write("base")
            self.git(tmp, "add", "-A")
            self.git(tmp, "commit", "-q", "-m", "base")
            parent = self.git(tmp, "rev-parse", "HEAD")
            for name in files:
                os.makedirs(os.path.dirname(os.path.join(tmp, name)) or tmp, exist_ok=True)
                with open(os.path.join(tmp, name), "w") as f:
                    f.write("changed")
            self.git(tmp, "add", "-A")
            self.git(tmp, "commit", "-q", "-m", "change", "--allow-empty")
            sha = self.git(tmp, "rev-parse", "HEAD")
            out = os.path.join(tmp, "out")
            env = {**os.environ, "EVENT": event, "BASE_REF": base_ref, "HEAD_REF": head_ref,
                   "BASE": parent, "BEFORE": parent if before == "parent" else before, "SHA": sha,
                   "GITHUB_OUTPUT": out, "RUNNER_TEMP": tmp}
            subprocess.run(["bash", "-c", script], cwd=tmp, env=env, check=True, capture_output=True)
            with open(out) as f:
                return dict(l.strip().split("=", 1) for l in f if "=" in l)

    def test_a_push_of_only_code_skips_the_suite(self):
        got = self.changes("push", ["src/x.rs"])
        self.assertEqual((got["code"], got["suite"]), ("true", "false"))

    def test_a_push_of_a_cache_refreshing_file_runs_the_suite(self):
        for f in ("Cargo.lock", "fuzz/Cargo.lock", "rust-toolchain.toml", ".github/workflows/rust.yml"):
            self.assertEqual(self.changes("push", [f])["suite"], "true", f)

    def test_a_push_the_step_cannot_judge_runs_the_suite(self):
        for before in (ZEROS, "1" * 40):
            self.assertEqual(self.changes("push", ["src/x.rs"], before=before)["suite"], "true", before)

    def test_the_weekly_run_runs_the_suite(self):
        got = self.changes("schedule", base_ref="")
        self.assertEqual((got["code"], got["suite"]), ("true", "true"))

    def test_a_docs_only_pull_request_has_no_code_and_the_suite_flag_true(self):
        got = self.changes("pull_request", ["README.md"])
        self.assertEqual((got["code"], got["suite"]), ("false", "true"))
        got = self.changes("pull_request", ["src/x.rs"])
        self.assertEqual((got["code"], got["suite"]), ("true", "true"))

    def checks(self, event, results, code="true", suite="true", parsing="", tune="false"):
        """Run the `rust checks` script; `results` maps a job to its result (default success)."""
        needs = {}
        for job in ALL_JOBS:
            needs[job] = {"result": results.get(job, "success"), "outputs": {}}
        if code is not None:
            needs["changes"]["outputs"] = {"code": code, "suite": suite, "tune-changes": tune}
        needs["fuzz-changes"]["outputs"] = {"parsing": parsing} if parsing else {}
        script = script_after(job_block(self.text, "checks"), "      - env:")
        done = subprocess.run(["bash", "-c", script], capture_output=True, text=True,
                              env={**os.environ, "EVENT": event, "NEEDS": json.dumps(needs)})
        return done.returncode

    PUSH_SKIPS = {j: "skipped" for j in SUITE_JOBS + ("fuzz-changes", "fuzz-short", "frozen")}

    def test_checks_accepts_the_skips_of_a_push_that_changed_no_suite_file(self):
        self.assertEqual(self.checks("push", self.PUSH_SKIPS, suite="false"), 0)

    def test_checks_rejects_a_skipped_job_that_had_to_run(self):
        self.assertEqual(self.checks("push", {**self.PUSH_SKIPS, "lint": "skipped"}, suite="true"), 1)
        self.assertEqual(self.checks("pull_request", {"lint": "skipped"}), 1)
        self.assertEqual(self.checks("push", {"lint": "skipped"}, code=None, suite=""), 1)
        self.assertEqual(self.checks("push", {**self.PUSH_SKIPS, "bench": "skipped"}, suite="false"), 1)

    def test_checks_expects_frozen_exactly_on_a_pull_request(self):
        self.assertEqual(self.checks("push", {"frozen": "skipped", "fuzz-changes": "skipped",
                                              "fuzz-short": "skipped"}), 0)
        self.assertEqual(self.checks("pull_request", {"frozen": "skipped", "fuzz-short": "skipped"}), 1)

    def test_checks_accepts_fuzz_build_skipped_only_when_parsing_changed(self):
        # parsing=true: the short run builds the targets, so the separate build may be skipped.
        self.assertEqual(self.checks("pull_request", {"fuzz-build": "skipped"}, parsing="true"), 0)
        # parsing=false: the short run is skipped and the build must have run.
        self.assertEqual(self.checks("pull_request", {"fuzz-build": "skipped", "fuzz-short": "skipped"},
                                     parsing="false"), 1)
        self.assertEqual(self.checks("pull_request", {"fuzz-short": "skipped"}, parsing="false"), 0)

    def test_the_suite_jobs_wait_for_the_suite_flag_and_the_bench_does_not(self):
        for job in SUITE_JOBS:
            header = job_block(self.text, job).split("    steps:")[0]
            self.assertIn("needs.changes.outputs.suite == 'true'", header, job)
        for job in ("bench-list", "bench", "bench-profile", "frozen", "fuzz-changes", "fuzz-short"):
            header = job_block(self.text, job).split("    steps:")[0]
            self.assertNotIn("outputs.suite", header, job)

    def test_every_push_has_its_own_concurrency_group(self):
        self.assertIn("github.event.pull_request.number || github.sha", self.text)
        self.assertNotIn("github.ref }}", self.text.split("concurrency:")[1].split("env:")[0])


if __name__ == "__main__":
    unittest.main()
