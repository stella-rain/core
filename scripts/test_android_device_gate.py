"""Unit tests for scripts/android_device_gate.py (the Android corpus run, docs/device-gates.md).

Linux-safe, no device: the whole-run tests put a fake `adb` and a fake `cargo` first on PATH.
"""

import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(__file__))

import android_device_gate as gate

SCRIPT = Path(__file__).resolve().parent / "android_device_gate.py"
REMOTE = "/data/local/tmp/stella-core-gate"
LINKER_TAIL = "toolchains/llvm/prebuilt/{}/bin/aarch64-linux-android24-clang"
FILTERS = ["corpus::", "the_golden_hash_is_the_same_on_every_platform"]

LIST = (
    "List of devices attached\nR58M123\tdevice usb:1\nemulator-5554\tdevice\nABC\tunauthorized\n\n"
)

# Fake adb: logs its arguments, answers from FAKE_* variables. `shell 'exit 7'` returns 7.
FAKE_ADB = """#!@PYTHON@
import os, sys
args = sys.argv[1:]
with open(os.environ["FAKE_LOG"], "a") as f:
    f.write(" ".join(args) + "\\n")
if args[:1] == ["devices"]:
    sys.stdout.write(os.environ["FAKE_DEVICES"])
    sys.exit(0)
assert args[0] == "-s", args
rest = args[2:]
if rest[0] == "shell":
    cmd = rest[1]
    if cmd == "exit 7":
        sys.exit(7)
    if cmd.startswith("getprop"):
        print(os.environ.get("FAKE_ABI", "arm64-v8a"))
        sys.exit(0)
    if "integration'" in cmd and "STELLA_TEST_DATA_ROOT" in cmd:
        sys.exit(int(os.environ.get("FAKE_RUN_STATUS", "0")))
sys.exit(0)
"""

# Fake cargo: prints the JSON lines of a finished build whose test binary is FAKE_BINARY.
FAKE_CARGO = """#!@PYTHON@
import json, os, sys
with open(os.environ["FAKE_LOG"], "a") as f:
    f.write("cargo " + " ".join(sys.argv[1:]) + " linker=" +
            os.environ.get("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", "") + "\\n")
print(json.dumps({"reason": "compiler-artifact", "target": {"name": "integration"},
                  "profile": {"test": True}, "executable": os.environ["FAKE_BINARY"]}))
print(json.dumps({"reason": "build-finished", "success": True}))
"""


def artifact(name, executable, test):
    return json.dumps({
        "reason": "compiler-artifact",
        "target": {"name": name},
        "profile": {"test": test},
        "executable": executable,
    })


class PureLogic(unittest.TestCase):
    def test_ndk_linker_variables_in_order(self):
        tail = LINKER_TAIL.format("linux-x86_64")
        self.assertEqual(gate.ndk_linker({"ANDROID_NDK_HOME": "/n"}, "linux-x86_64"), "/n/" + tail)
        self.assertEqual(gate.ndk_linker({"ANDROID_NDK_ROOT": "/r"}, "linux-x86_64"), "/r/" + tail)
        self.assertEqual(
            gate.ndk_linker({"ANDROID_NDK_LATEST_HOME": "/l"}, "linux-x86_64"), "/l/" + tail)
        both = {"ANDROID_NDK_HOME": "/n", "ANDROID_NDK_ROOT": "/r"}
        self.assertEqual(gate.ndk_linker(both, "linux-x86_64"), "/n/" + tail)

    def test_ndk_linker_windows_is_a_cmd_file(self):
        linker = gate.ndk_linker({"ANDROID_NDK_HOME": "/n"}, "windows-x86_64")
        self.assertTrue(linker.endswith("aarch64-linux-android24-clang.cmd"), linker)
        self.assertIn("windows-x86_64", linker)

    def test_ndk_linker_takes_the_highest_sdk_ndk(self):
        with tempfile.TemporaryDirectory() as sdk:
            for version in ("25.2.9519653", "26.1.10909125"):
                os.makedirs(os.path.join(sdk, "ndk", version))
            for var in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
                linker = gate.ndk_linker({var: sdk}, "linux-x86_64")
                self.assertIn("26.1.10909125", linker)
                self.assertNotIn("25.2", linker)

    def test_ndk_linker_numeric_not_text_order(self):
        with tempfile.TemporaryDirectory() as sdk:
            for version in ("9.0.1", "26.1.10909125"):
                os.makedirs(os.path.join(sdk, "ndk", version))
            self.assertIn("26.1.10909125", gate.ndk_linker({"ANDROID_HOME": sdk}, "linux-x86_64"))

    def test_ndk_linker_none_found(self):
        with self.assertRaisesRegex(gate.GateError, "android-gate: no NDK"):
            gate.ndk_linker({}, "linux-x86_64")

    def test_parse_devices(self):
        self.assertEqual(
            gate.parse_devices(LIST),
            [("R58M123", "device"), ("emulator-5554", "device"), ("ABC", "unauthorized")],
        )

    def test_parse_devices_skips_daemon_noise(self):
        text = "* daemon not running; starting now\n* daemon started successfully\n" + LIST
        self.assertEqual(len(gate.parse_devices(text)), 3)

    def test_choose_device(self):
        devices = gate.parse_devices(LIST)
        with self.assertRaises(gate.GateError) as caught:
            gate.choose_device(devices, None)
        self.assertIn("R58M123", str(caught.exception))
        self.assertIn("emulator-5554", str(caught.exception))
        self.assertEqual(gate.choose_device(devices, "R58M123"), "R58M123")
        with self.assertRaisesRegex(gate.GateError, "unauthorized"):
            gate.choose_device(devices, "ABC")
        with self.assertRaisesRegex(gate.GateError, "not connected"):
            gate.choose_device(devices, "nope")
        self.assertEqual(gate.choose_device([("A", "device"), ("B", "offline")], None), "A")
        with self.assertRaisesRegex(gate.GateError, "no device"):
            gate.choose_device([("B", "offline")], None)

    def test_cargo_command(self):
        release = gate.cargo_command(False)
        self.assertEqual(
            release,
            ["cargo", "test", "--locked", "--all-features", "--test", "integration", "--release",
             "--target", "aarch64-linux-android", "--no-run", "--message-format=json"],
        )
        self.assertNotIn("--release", gate.cargo_command(True))

    def test_find_test_binary(self):
        lines = [
            artifact("other", "/t/other-1", True),
            artifact("integration", None, False),
            artifact("integration", "/t/integration-abc", True),
            json.dumps({"reason": "build-finished", "success": True}),
            "not json at all",
        ]
        self.assertEqual(gate.find_test_binary(lines), "/t/integration-abc")
        with self.assertRaisesRegex(gate.GateError, "android-gate:"):
            gate.find_test_binary(lines[:2] + lines[3:])

    def test_push_plan(self):
        plan = gate.push_plan("/repo", "/t/integration-abc", REMOTE)
        expected = [("/t/integration-abc", REMOTE + "/integration")]
        expected += [("/repo/" + f, REMOTE + "/" + f) for f in gate.DATA_FOLDERS]
        self.assertEqual(plan, expected)
        for src, _ in plan:
            self.assertNotIn(".git", src.split("/"))
            self.assertNotIn("target", src.split("/"))

    def test_run_command(self):
        cmd = gate.run_command(REMOTE, {})
        self.assertIn("STELLA_TEST_DATA_ROOT='%s'" % REMOTE, cmd)
        self.assertIn("STELLA_TEST_TMP_DIR='%s/tmp'" % REMOTE, cmd)
        self.assertIn("'%s/integration' 'corpus::' 'the_golden_hash_is_the_same_on_every_platform'" % REMOTE, cmd)
        self.assertTrue(cmd.startswith("cd /data/local/tmp && env "))
        self.assertNotIn("PROPTEST_CASES", cmd)
        self.assertIn("PROPTEST_CASES='7'", gate.run_command(REMOTE, {"PROPTEST_CASES": "7"}))
        self.assertIn("RUST_BACKTRACE='1'", gate.run_command(REMOTE, {"RUST_BACKTRACE": "1"}))

    def test_run_command_escapes_quotes(self):
        cmd = gate.run_command(REMOTE, {"PROPTEST_CASES": "a'b"})
        self.assertIn("PROPTEST_CASES='a'\\''b'", cmd)

    def test_cleanup_command(self):
        self.assertEqual(gate.cleanup_command(REMOTE), "rm -rf '%s'" % REMOTE)
        for bad in ("/data/local/tmp", "/data/local/tmp/", "/", ""):
            with self.assertRaises(gate.GateError):
                gate.cleanup_command(bad)


class WholeRun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.bin = self.dir / "bin"
        self.bin.mkdir()
        self.log = self.dir / "log"
        self.binary = self.dir / "integration-abc"
        self.binary.write_bytes(b"\x7fELF")
        self.write_tool("adb", FAKE_ADB)
        self.write_tool("cargo", FAKE_CARGO)

    def write_tool(self, name, body):
        path = self.bin / name
        path.write_text(body.replace("@PYTHON@", sys.executable))
        path.chmod(path.stat().st_mode | stat.S_IEXEC)

    def run_script(self, *args, env=None, use_path=True):
        full = {
            "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", "") if use_path else "",
            "FAKE_LOG": str(self.log),
            "FAKE_BINARY": str(self.binary),
            "FAKE_DEVICES": "List of devices attached\nSER1\tdevice usb:1\n",
            "ANDROID_NDK_HOME": str(self.dir / "ndk"),
            "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
        }
        full.update(env or {})
        return subprocess.run([sys.executable, str(SCRIPT), *args], env=full, cwd=str(self.dir),
                              capture_output=True, text=True)

    def lines(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_dry_run_needs_nothing(self):
        result = self.run_script("--dry-run", "--serial", "X", env={"ANDROID_NDK_HOME": ""},
                                 use_path=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        out = result.stdout
        self.assertIn("cargo test", out)
        self.assertIn("--target aarch64-linux-android", out)
        self.assertEqual(out.count("adb -s X push"), 1 + len(gate.DATA_FOLDERS))
        self.assertIn("STELLA_TEST_DATA_ROOT", out)
        self.assertIn("rm -rf '%s'" % REMOTE, out)
        self.assertEqual(self.lines(), [])
        self.assertEqual(sorted(os.listdir(self.bin)), ["adb", "cargo"])

    def test_dry_run_without_serial_uses_placeholder(self):
        result = self.run_script("--dry-run", use_path=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("<the connected device>", result.stdout)
        self.assertIn("<test binary>", result.stdout)

    def test_dry_run_shows_the_linker_override(self):
        result = self.run_script("--dry-run", env={"ANDROID_NDK_HOME": "",
                                 "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER": "/my/clang"})
        self.assertIn("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=/my/clang cargo", result.stdout)

    def test_no_adb_is_reported_before_the_ndk(self):
        result = self.run_script("--serial", "SER1", env={"ANDROID_NDK_HOME": ""}, use_path=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("adb is not on PATH", result.stderr)

    def test_no_cargo_exits_2(self):
        (self.bin / "cargo").unlink()
        result = self.run_script("--serial", "SER1", env={"PATH": str(self.bin)})
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("android-gate:", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_good_run_cleans_up(self):
        result = self.run_script("--serial", "SER1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = self.lines()
        self.assertTrue(any(l.startswith("cargo ") and "--no-run" in l for l in lines))
        self.assertTrue(any("linker=" in l and "aarch64-linux-android24-clang" in l for l in lines))
        pushes = [l for l in lines if " push " in l]
        self.assertEqual(len(pushes), 1 + len(gate.DATA_FOLDERS))
        self.assertTrue(all(l.startswith("-s SER1 ") for l in lines
                            if not l.startswith(("cargo", "devices"))))
        self.assertEqual(lines[-1], "-s SER1 shell rm -rf '%s'" % REMOTE)

    def test_two_devices_without_serial_exit_2(self):
        text = "List of devices attached\nSER1\tdevice\nSER2\tdevice\n"
        result = self.run_script(env={"FAKE_DEVICES": text})
        self.assertEqual(result.returncode, 2)
        self.assertIn("SER1", result.stderr + result.stdout)
        self.assertIn("SER2", result.stderr + result.stdout)
        self.assertFalse(any(" push " in l for l in self.lines()))

    def test_serial_from_environment(self):
        text = "List of devices attached\nSER1\tdevice\nSER2\tdevice\n"
        result = self.run_script(env={"FAKE_DEVICES": text, "ANDROID_SERIAL": "SER2"})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(any(l.startswith("-s SER2 ") for l in self.lines()))

    def test_test_status_is_the_exit_status_and_cleanup_still_runs(self):
        result = self.run_script("--serial", "SER1", env={"FAKE_RUN_STATUS": "3"})
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
        self.assertEqual(self.lines()[-1], "-s SER1 shell rm -rf '%s'" % REMOTE)

    def test_wrong_abi_exits_2_before_any_push(self):
        result = self.run_script("--serial", "SER1", env={"FAKE_ABI": "armeabi-v7a"})
        self.assertEqual(result.returncode, 2)
        self.assertIn("arm64", result.stderr + result.stdout)
        self.assertFalse(any(" push " in l for l in self.lines()))
        self.assertFalse(any(l.startswith("cargo ") for l in self.lines()))

    def test_no_ndk_exits_2(self):
        result = self.run_script("--serial", "SER1", env={"ANDROID_NDK_HOME": ""})
        self.assertEqual(result.returncode, 2)
        self.assertIn("android-gate: no NDK", result.stderr + result.stdout)

    def test_debug_drops_release(self):
        result = self.run_script("--serial", "SER1", "--debug")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        cargo = [l for l in self.lines() if l.startswith("cargo ")][0]
        self.assertNotIn("--release", cargo)


if __name__ == "__main__":
    unittest.main()
