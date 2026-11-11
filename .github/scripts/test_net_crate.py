"""Tests for the `net/` crate and its workspace. Run: python3 -m unittest discover -s .github/scripts

The root stays a real package whose default member is core alone, so every existing job keeps its
meaning; `stella-rain-net` has its own manifest, lints, deny rules, CI job and rules file.
"""

import os
import re
import tomllib
import unittest

from test_rust_workflow import ROOT, job_block, read, read_root

BANNED = [
    "ggrs", "bevy_ggrs", "backroll", "matchbox_socket",
    "tokio", "async-std", "mio", "socket2", "reqwest", "hyper", "tungstenite",
]
MOBILE = "aarch64-apple-ios aarch64-linux-android x86_64-linux-android"
# Commands of the jobs that must not change when net is added.
UNCHANGED = {
    "lint": [
        "cargo fmt --check",
        "cargo clippy --all-targets --locked -- -D warnings",
        "cargo clippy --all-targets --all-features --locked -- -D warnings",
    ],
    "bless": ["cargo xtask bless --check"],
    "frozen": ["cargo test --locked --test integration corpus::the_frozen"],
    "test": [
        "cargo nextest run --locked --release --profile ci",
        "cargo test --doc --locked --all-features",
    ],
    "cross": ["cargo check --locked --all-targets --target \"$t\""],
}


def toml_of(path):
    return tomllib.loads(read_root(path))


class Workspace(unittest.TestCase):
    def setUp(self):
        self.root = toml_of("Cargo.toml")

    def test_the_root_is_a_workspace_with_core_as_the_default_member(self):
        ws = self.root["workspace"]
        self.assertEqual(ws["members"], [".", "net"])
        self.assertEqual(ws["exclude"], ["fuzz", "xtask"])
        self.assertNotIn("default-members", ws)

    def test_core_is_unchanged(self):
        p = self.root["package"]
        self.assertEqual(p["name"], "stella-rain-core")
        self.assertIs(p["publish"], False)
        self.assertEqual(set(self.root["features"]), {"record", "bench"})

    def test_core_depends_on_net_only_as_a_bench_dev_dependency(self):
        self.assertNotIn("stella-rain-net", self.root.get("dependencies", {}))
        self.assertNotIn("stella-rain-net", self.root.get("build-dependencies", {}))
        dev = self.root.get("dev-dependencies", {})
        self.assertIn("stella-rain-net", dev, "the rollback_session bench needs it")
        self.assertEqual(dev["stella-rain-net"], {"path": "net"})
        # The one allowed dev-dependency is the rollback Session bench's: nothing else under
        # src/, tests/, examples/ or benches/ names the crate, except the bench's own helpers and
        # the test that pins the bench's cases.
        allowed = (
            "benches/rollback_session.rs",
            "benches/session_case/",
            "tests/bench_fixture_pins.rs",
        )
        users = []
        for top in ("src", "tests", "examples", "benches"):
            for dirpath, _, names in os.walk(os.path.join(ROOT, top)):
                for name in names:
                    if not name.endswith(".rs"):
                        continue
                    path = os.path.join(dirpath, name)
                    rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
                    with open(path, encoding="utf-8") as f:
                        if "stella_rain_net" in f.read() and not rel.startswith(allowed):
                            users.append(rel)
        self.assertEqual(users, [], "files other than the bench name stella_rain_net")


class NetManifest(unittest.TestCase):
    def setUp(self):
        self.m = toml_of("net/Cargo.toml")

    def test_package(self):
        p = self.m["package"]
        self.assertEqual(p["name"], "stella-rain-net")
        self.assertEqual(p["version"], "0.0.0")
        self.assertIs(p["publish"], False)
        self.assertEqual(p["license"], "MIT OR Apache-2.0")

    def test_only_core_as_a_dependency(self):
        self.assertEqual(set(self.m["dependencies"]), {"stella-rain-core"})
        self.assertEqual(self.m["dependencies"]["stella-rain-core"]["path"], "..")
        for table in ("dev-dependencies", "build-dependencies", "features"):
            self.assertNotIn(table, self.m)

    def test_lints(self):
        self.assertEqual(self.m["lints"]["rust"]["unsafe_code"], "forbid")
        self.assertEqual(self.m["lints"]["rust"]["missing_docs"], "deny")
        for lint in ("cast_possible_truncation", "cast_sign_loss", "cast_possible_wrap", "cast_lossless"):
            self.assertEqual(self.m["lints"]["clippy"][lint], "deny")

    def test_lib_declares_only_its_modules_and_is_documented(self):
        src = read_root("net/src/lib.rs")
        self.assertIn("#![forbid(unsafe_code)]", src)
        self.assertIn("#![deny(missing_docs)]", src)
        self.assertTrue(src.startswith("//!"))
        self.assertEqual(
            [l for l in src.splitlines() if l.startswith("pub ")],
            ["pub mod save;", "pub mod session;", "pub mod transport;", "pub mod wire;"],
        )


class Lock(unittest.TestCase):
    def test_one_new_package_and_core_not_stamped(self):
        pkgs = {p["name"]: p for p in toml_of("Cargo.lock")["package"]}
        net = pkgs["stella-rain-net"]
        self.assertEqual(net["version"], "0.0.0")
        self.assertEqual(net["dependencies"], ["stella-rain-core"])
        self.assertEqual(pkgs["stella-rain-core"]["version"], toml_of("Cargo.toml")["package"]["version"])


class Deny(unittest.TestCase):
    def test_banned_crates_have_reasons(self):
        bans = {b["crate"]: b for b in toml_of("deny.toml")["bans"]["deny"]}
        for name in BANNED:
            self.assertIn(name, bans)
            self.assertTrue(bans[name].get("reason", "").strip(), name)


class Workflow(unittest.TestCase):
    def setUp(self):
        self.text = read("rust.yml")

    def test_net_job(self):
        j = job_block(self.text, "net")
        header = j.split("    steps:")[0]
        self.assertIn("needs: changes", header)
        self.assertIn("if: needs.changes.outputs.code == 'true'", header)
        self.assertIn("contents: read", j)
        self.assertIn("persist-credentials: false", j)
        for c in (
            "cargo fmt -p stella-rain-net --check",
            "cargo clippy -p stella-rain-net --all-targets --locked -- -D warnings",
            "cargo test -p stella-rain-net --locked",
            'RUSTDOCFLAGS="-D warnings" cargo doc -p stella-rain-net --no-deps --locked',
            'cargo check -p stella-rain-net --locked --all-targets --target "$t"',
        ):
            self.assertIn(c, j)
        # The toolchain file holds the components and the mobile targets: `rustup toolchain install`.
        self.assertIn("rustup toolchain install\n", j)
        self.assertIn(f"for t in {MOBILE}; do", j)

    def test_net_is_required_by_rust_checks(self):
        self.assertRegex(job_block(self.text, "checks"), r"needs:.*\bnet\b")

    def test_existing_jobs_keep_their_commands(self):
        for job, cmds in UNCHANGED.items():
            block = job_block(self.text, job)
            for c in cmds:
                self.assertIn(c, block, job)
            self.assertNotIn("--workspace", block, job)
            self.assertNotIn("stella-rain-net", block, job)

    def test_cargo_deny_checks_both_manifests(self):
        block = job_block(self.text, "cargo-deny")
        for manifest in ("Cargo.toml", "fuzz/Cargo.toml"):
            self.assertIn(f"cargo deny --manifest-path {manifest} check", block)


class Rules(unittest.TestCase):
    def body(self, path, key):
        text = read_root(path)
        m = re.match(rf"---\n{key}:\n((?:  - .*\n)+)---\n(.*)", text, re.S)
        self.assertTrue(m, path)
        self.assertIn('"net/**"', m.group(1))
        self.assertLessEqual(len(text.splitlines()), 80, path)
        return m.group(2)

    def test_both_copies_agree(self):
        a = self.body(".claude/rules/net.md", "paths")
        b = self.body(".agents/rules/net.md", "globs")
        self.assertEqual(a, b)

    def test_agents_md(self):
        text = read_root("AGENTS.md")
        self.assertLessEqual(len(text.splitlines()), 100)
        gates = text.split("## Gates")[1].split("\n## ")[0]
        self.assertIn("net/", gates)
        self.assertIn("| `net/` |", text.split("## Layout")[1].split("## Gates")[0])


if __name__ == "__main__":
    unittest.main()
