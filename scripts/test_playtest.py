"""Unit tests for scripts/playtest.py (RP2-1, ADR-CONT-5)."""

import os
import sys
import unittest

# Allow importing playtest from the same directory
sys.path.insert(0, os.path.dirname(__file__))

import playtest

COMMIT = "f108f4f718557c7d41ce8ab7900ff5180d9e09eb"
OTHER = "0123456789abcdef0123456789abcdef01234567"

TOML_TEMPLATE = """[dependencies]
# a comment mentioning stella-rain-core = {{ git = "x", rev = "{other}" }}
stella-rain-core = {{ git = "https://github.com/stella-rain/core", rev = "{commit}" }}
serde_json = "1"

[dev-dependencies]
stella-rain-core = {{ git = "https://github.com/stella-rain/core", rev = "{commit}", features = ["record"] }}
"""


class TestPlaytest(unittest.TestCase):
    def test_parse_args_defaults_and_options(self):
        self.assertEqual(
            playtest.parse_args([]),
            {"app": "../app", "profile": "test-env", "build": False},
        )
        self.assertEqual(
            playtest.parse_args(["--app", "/x/app", "--profile", "my_1", "--build"]),
            {"app": "/x/app", "profile": "my_1", "build": True},
        )

    def test_parse_args_refusals(self):
        with self.assertRaisesRegex(ValueError, "unknown option --nope"):
            playtest.parse_args(["--nope"])
        with self.assertRaisesRegex(ValueError, "--app needs a value"):
            playtest.parse_args(["--app"])
        with self.assertRaisesRegex(ValueError, "--profile"):
            playtest.parse_args(["--profile", "a b"])
        with self.assertRaisesRegex(ValueError, "--profile"):
            playtest.parse_args(["--profile", "x" * 41])
        with self.assertRaisesRegex(ValueError, "--profile"):
            playtest.parse_args(["--profile", ""])

    def test_app_pin_reads_rev_or_tag_ignoring_comments(self):
        toml = TOML_TEMPLATE.format(other=OTHER, commit=COMMIT)
        self.assertEqual(playtest.app_pin(toml), {"kind": "commit", "ref": COMMIT})
        self.assertEqual(
            playtest.app_pin('stella-rain-core = { git = "u", tag = "v0.0.15" }\n'),
            {"kind": "tag", "ref": "v0.0.15"},
        )

    def test_app_pin_refusals(self):
        with self.assertRaisesRegex(ValueError, "no stella-rain-core"):
            playtest.app_pin("[dependencies]\n")
        with self.assertRaisesRegex(ValueError, "rev or tag"):
            playtest.app_pin('stella-rain-core = { git = "u", branch = "main" }\n')
        with self.assertRaisesRegex(ValueError, "disagree"):
            playtest.app_pin(
                f'stella-rain-core = {{ git = "u", rev = "{COMMIT}" }}\n'
                f'stella-rain-core = {{ git = "u", rev = "{OTHER}" }}\n'
            )

    def test_check_pins_equal_and_version_mismatch(self):
        # Equal tags pass
        playtest.check_pins(
            {"kind": "tag", "ref": "v0.0.15"},
            {"kind": "tag", "ref": "v0.0.15"},
        )
        # Equal commits pass
        playtest.check_pins(
            {"kind": "commit", "ref": COMMIT},
            {"kind": "commit", "ref": COMMIT},
        )
        # Different commits pass (pin equality not required during local balancing in core)
        playtest.check_pins(
            {"kind": "commit", "ref": COMMIT},
            {"kind": "commit", "ref": OTHER},
        )
        # Mismatched tags raise ValueError mentioning both values
        with self.assertRaises(ValueError) as cm:
            playtest.check_pins(
                {"kind": "tag", "ref": "v0.0.15"},
                {"kind": "tag", "ref": "v0.1.0"},
            )
        self.assertIn("v0.0.15", str(cm.exception))
        self.assertIn("v0.1.0", str(cm.exception))

    def test_install_hint(self):
        base = "cargo install --git https://github.com/stella-rain/core"
        self.assertEqual(
            playtest.install_hint({"kind": "tag", "ref": "v0.0.15"}),
            f"{base} --tag v0.0.15 --bin content-pack",
        )
        self.assertEqual(
            playtest.install_hint({"kind": "commit", "ref": COMMIT}),
            f"{base} --rev {COMMIT} --bin content-pack",
        )

    def test_godot_binary(self):
        self.assertEqual(playtest.godot_binary({"GODOT": "/opt/godot"}), "/opt/godot")
        self.assertEqual(playtest.godot_binary({}), "godot")
        self.assertEqual(playtest.godot_binary({"GODOT": ""}), "godot")

    def test_build_and_launch_commands(self):
        app = os.path.abspath("/w/app")
        self.assertEqual(
            playtest.build_command(app),
            {
                "cmd": "cargo",
                "args": ["build", "--features", "record"],
                "cwd": os.path.join(app, "godot-ext"),
            },
        )
        pack = os.path.abspath("/w/content/pack.json")
        self.assertEqual(
            playtest.launch_command("godot", app, pack, "test-env"),
            {
                "cmd": "godot",
                "args": [
                    "--path",
                    os.path.join(app, "godot"),
                    "--",
                    f"--content-path={pack}",
                    "--profile=test-env",
                ],
            },
        )


if __name__ == "__main__":
    unittest.main()
