"""Unit tests for scripts/ios_device_runner.py (the iPhone corpus run, docs/device-gates.md).

Linux-safe: the whole-run tests put fake `xcrun`, `security` and `codesign` first on PATH. Every value
is invented (UDID, team, name, certificates); no real profile bytes.
"""

import datetime
import hashlib
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(__file__))

import ios_device_runner as runner

SCRIPT = Path(__file__).resolve().parent / "ios_device_runner.py"
REPO = SCRIPT.parent.parent
UDID = "00008110-0000000000000001"
OTHER_UDID = "00008110-0000000000000002"
TEAM = "ABCDE12345"
APP_ID = "com.stellarain.Dinghy"
CERT = b"cert-one"
CERT_SHA = hashlib.sha1(CERT).hexdigest().upper()
NOW = datetime.datetime(2026, 10, 31, 12, 0, 0)
ENTITLEMENTS = {
    "application-identifier": TEAM + "." + APP_ID,
    "com.apple.developer.team-identifier": TEAM,
    "get-task-allow": True,
}
GOOD_LOG = [
    "Launched application with com.stellarain.Dinghy bundle identifier.",
    "running 26 tests",
    "test corpus::the_corpus_matches ... ok",
    "test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 1808 filtered out; finished in 0.45s",
    "The app terminated with the exit code 0.",
]


def device(platform="iOS", reality="physical", tunnel="connected", udid=UDID, name="Test iPhone",
           device_type="iPhone", cpu=True):
    hardware = {"platform": platform, "udid": udid, "deviceType": device_type, "marketingName": name}
    if reality:
        hardware["reality"] = reality
    if cpu:
        hardware["cpuType"] = {"name": "arm64e"}
    return {
        "hardwareProperties": hardware,
        "connectionProperties": {"tunnelState": tunnel, "transportType": "wired"},
        "deviceProperties": {"name": name, "osVersionNumber": "27.2"},
        "identifier": "11111111-2222-3333-4444-555555555555",
    }


def listing(*devices):
    return {"result": {"devices": list(devices)}}


WATCH = device(platform="watchOS", reality=None, tunnel="disconnected", name="Test Watch",
               device_type="appleWatch", cpu=False, udid="watch-udid")
SIM = device(reality="simulated", tunnel="disconnected", name="Sim", udid="sim-udid")


def profile_data(app_id=TEAM + "." + APP_ID, devices=(UDID,), certs=(CERT,), expires=datetime.datetime(2026, 11, 7, 10, 5, 45)):
    return {
        "Name": "iOS Team Provisioning Profile: " + APP_ID,
        "TeamIdentifier": [TEAM],
        "ProvisionedDevices": list(devices),
        "DeveloperCertificates": list(certs),
        "ExpirationDate": expires,
        "Entitlements": dict(ENTITLEMENTS, **{"application-identifier": app_id}),
    }


def profile(name="p.mobileprovision", **kw):
    return runner.Profile(Path(name), profile_data(**kw))


IDENTITIES = {CERT_SHA: "Apple Development: someone@example.invalid (ABC123)"}


class TestSelectDevice(unittest.TestCase):
    def test_skips_watch_and_simulator(self):
        d = runner.select_device(listing(WATCH, device(), SIM))
        self.assertEqual(d.udid, UDID)
        self.assertEqual(d.name, "Test iPhone")
        self.assertEqual(d.os_version, "27.2")

    def test_disconnected_phone(self):
        with self.assertRaises(runner.RunnerError) as cm:
            runner.select_device(listing(device(tunnel="disconnected")))
        self.assertIn("no physical iPhone", str(cm.exception))
        self.assertIn("disconnected", str(cm.exception))

    def test_empty_and_malformed(self):
        for doc in (listing(), {}, {"result": {}}):
            with self.assertRaises(runner.RunnerError) as cm:
                runner.select_device(doc)
            self.assertIn("no physical iPhone", str(cm.exception))

    def test_two_phones(self):
        with self.assertRaises(runner.RunnerError) as cm:
            runner.select_device(listing(device(), device(udid=OTHER_UDID, name="Second")))
        msg = str(cm.exception)
        for part in ("Test iPhone", "Second", UDID, OTHER_UDID, "disconnect all but one"):
            self.assertIn(part, msg)

    def test_an_ipad_is_accepted(self):
        d = runner.select_device(listing(device(device_type="iPad", name="Test iPad")))
        self.assertEqual(d.name, "Test iPad")


class TestParseIdentities(unittest.TestCase):
    def test_only_development_identities(self):
        out = (
            '  1) {sha} "Apple Development: someone@example.invalid (ABC123)"\n'
            '  2) {other} "Developer ID Application: Someone (XYZ)"\n'
            "     2 valid identities found\n\n"
        ).format(sha=CERT_SHA.lower(), other="B" * 40)
        self.assertEqual(
            runner.parse_identities(out),
            {CERT_SHA: "Apple Development: someone@example.invalid (ABC123)"},
        )

    def test_old_prefix(self):
        out = '  1) {} "iPhone Developer: x (Y)"\n'.format("A" * 40)
        self.assertEqual(runner.parse_identities(out), {"A" * 40: "iPhone Developer: x (Y)"})


class TestChooseProfile(unittest.TestCase):
    def choose(self, profiles, udid=UDID, identities=IDENTITIES):
        return runner.choose_profile(profiles, udid, identities, NOW)

    def test_valid(self):
        c = self.choose([profile()])
        self.assertEqual(c.team, TEAM)
        self.assertEqual(c.identity, CERT_SHA)
        self.assertEqual(c.bundle_id, APP_ID)
        self.assertEqual(c.entitlements, ENTITLEMENTS)
        self.assertEqual(c.path, Path("p.mobileprovision"))

    def test_latest_expiry_wins(self):
        later = datetime.datetime(2026, 11, 20)
        c = self.choose([profile("a"), profile("b", expires=later), profile("c")])
        self.assertEqual(c.path, Path("b"))

    def test_expired(self):
        old = datetime.datetime(2026, 10, 30)
        with self.assertRaises(runner.RunnerError) as cm:
            self.choose([profile(expires=old)])
        self.assertIn("expired on 2026-10-30", str(cm.exception))
        self.assertIn("7 days", str(cm.exception))
        c = self.choose([profile("old", expires=old), profile("new")])
        self.assertEqual(c.path, Path("new"))

    def test_other_app_and_wildcard(self):
        for app_id in (TEAM + ".com.other.App", TEAM + ".*"):
            with self.assertRaises(runner.RunnerError) as cm:
                self.choose([profile(app_id=app_id)])
            self.assertIn("no provisioning profile for com.stellarain.Dinghy", str(cm.exception))
        with self.assertRaises(runner.RunnerError):
            self.choose([])

    def test_device_not_listed(self):
        with self.assertRaises(runner.RunnerError) as cm:
            self.choose([profile(devices=(OTHER_UDID,))])
        self.assertIn("does not list this iPhone", str(cm.exception))
        self.assertIn(UDID, str(cm.exception))

    def test_no_identity(self):
        with self.assertRaises(runner.RunnerError) as cm:
            self.choose([profile()], identities={"F" * 40: "Apple Development: x"})
        self.assertIn("no signing identity", str(cm.exception))

    def test_expiry_is_reported_before_the_device(self):
        with self.assertRaises(runner.RunnerError) as cm:
            self.choose([profile(devices=(), expires=datetime.datetime(2026, 1, 1))])
        self.assertIn("expired", str(cm.exception))

    def test_load_profiles_reads_both_folders(self):
        with tempfile.TemporaryDirectory() as home:
            home = Path(home)
            a = home / runner.PROFILE_DIRS[0]
            b = home / runner.PROFILE_DIRS[1]
            a.mkdir(parents=True)
            b.mkdir(parents=True)
            (a / "one.mobileprovision").write_bytes(b"1")
            (b / "two.mobileprovision").write_bytes(b"2")
            (b / "notes.txt").write_bytes(b"3")
            seen = []

            def decode(path):
                seen.append(path.name)
                return {"file": path.name}

            profiles = runner.load_profiles(home, decode)
            self.assertEqual(sorted(p.path.name for p in profiles), ["one.mobileprovision", "two.mobileprovision"])
            self.assertEqual(sorted(seen), ["one.mobileprovision", "two.mobileprovision"])
            shutil.rmtree(a)
            self.assertEqual(len(runner.load_profiles(home, decode)), 1)


class TestPlists(unittest.TestCase):
    def test_info_plist(self):
        info = plistlib.loads(runner.info_plist(APP_ID))
        self.assertEqual(info["CFBundleIdentifier"], APP_ID)
        self.assertEqual(info["CFBundleExecutable"], "Dinghy")
        self.assertEqual(info["CFBundleName"], "Dinghy")
        self.assertEqual(info["CFBundleVersion"], "arm64")
        self.assertEqual(info["UIRequiredDeviceCapabilities"], ["arm64"])

    def test_entitlements_round_trip(self):
        back = plistlib.loads(runner.entitlements_plist(ENTITLEMENTS))
        self.assertEqual(back, ENTITLEMENTS)
        self.assertIs(back["get-task-allow"], True)


class TestCommands(unittest.TestCase):
    def test_launch(self):
        self.assertEqual(
            runner.launch_command(UDID, ["corpus::", "the_golden_hash_is_the_same_on_every_platform"]),
            ["xcrun", "devicectl", "device", "process", "launch", "--console", "--terminate-existing",
             "--device", UDID, APP_ID, "corpus::", "the_golden_hash_is_the_same_on_every_platform"],
        )
        self.assertEqual(runner.launch_command(UDID, ["a b"])[-1], "a b")

    def test_install(self):
        self.assertEqual(
            runner.install_command(UDID, "/t/Dinghy.app"),
            ["xcrun", "devicectl", "device", "install", "app", "--device", UDID, "/t/Dinghy.app"],
        )


class TestJudgeRun(unittest.TestCase):
    def log(self, result=None, code="0"):
        lines = [l for l in GOOD_LOG if not l.startswith(("test result", "The app"))]
        if result:
            lines.append(result)
        if code is not None:
            lines.append("The app terminated with the exit code %s." % code)
        return lines

    def test_good(self):
        self.assertEqual(runner.judge_run(GOOD_LOG, 0)[0], 0)

    def test_failed_result(self):
        code, msg = runner.judge_run(self.log("test result: FAILED. 25 passed; 1 failed;", "101"), 0)
        self.assertNotEqual(code, 0)
        self.assertIn("101", msg)

    def test_apps_code_wins(self):
        code, msg = runner.judge_run(self.log("test result: ok. 26 passed; 0 failed;", "3"), 0)
        self.assertNotEqual(code, 0)
        self.assertIn("3", msg)

    def test_no_result_line(self):
        code, msg = runner.judge_run(self.log(None, "0"), 0)
        self.assertNotEqual(code, 0)
        self.assertIn("no test result line", msg)

    def test_no_exit_code_line(self):
        code, msg = runner.judge_run(self.log("test result: ok. 26 passed;", None), 0)
        self.assertNotEqual(code, 0)
        self.assertIn("no exit code", msg)

    def test_devicectl_failed(self):
        self.assertNotEqual(runner.judge_run(GOOD_LOG, 1)[0], 0)

    def test_zero_passed_is_cargo_parity(self):
        self.assertEqual(runner.judge_run(self.log("test result: ok. 0 passed; 0 failed;"), 0)[0], 0)

    def test_one_failed_of_two_results(self):
        lines = self.log("test result: ok. 26 passed; 0 failed;")
        lines.insert(0, "test result: FAILED. 0 passed; 1 failed;")
        self.assertNotEqual(runner.judge_run(lines, 0)[0], 0)

    def test_result_text_after_a_missing_exit_line(self):
        lines = ["test result: ok. 26 passed;", "the app was killed", "test result: ok. noise"]
        self.assertNotEqual(runner.judge_run(lines, 0)[0], 0)


FAKE = r'''#!{python}
import json, os, plistlib, stat, sys
TOOL = "{tool}"
conf = json.load(open(os.environ["FAKE_CONF"]))
args = sys.argv[1:]
def log(label, **extra):
    with open(os.environ["FAKE_LOG"], "a") as f:
        f.write(json.dumps(dict(label=label, argv=args, **extra)) + "\n")
if TOOL == "xcrun":
    if args[:3] == ["devicectl", "list", "devices"]:
        log("devicectl list")
        json.dump(conf["devices"], open(args[args.index("--json-output") + 1], "w"))
    elif args[:3] == ["devicectl", "device", "install"]:
        log("devicectl install")
        if conf.get("install_status"):
            print("install boom"); sys.exit(conf["install_status"])
    elif args[:3] == ["devicectl", "device", "process"]:
        log("devicectl launch")
        for line in conf["launch_lines"]:
            print(line, flush=True)
        sys.exit(conf.get("launch_status", 0))
    else:
        sys.exit(2)
elif TOOL == "security":
    if args[0] == "find-identity":
        log("security find-identity")
        print(conf["identities"])
    elif args[:2] == ["cms", "-D"]:
        log("security cms")
        sys.stdout.buffer.write(open(conf["profile_plist"], "rb").read())
    else:
        sys.exit(2)
elif TOOL == "codesign":
    if "--verify" in args:
        log("codesign verify")
    else:
        app = args[-1]
        files = []
        for root, dirs, names in os.walk(app):
            for n in names:
                files.append(os.path.relpath(os.path.join(root, n), app))
        ent = plistlib.load(open(args[args.index("--entitlements") + 1], "rb"))
        prof = open(os.path.join(app, "embedded.mobileprovision"), "rb").read()
        log("codesign sign", app=app, files=sorted(files), entitlements=ent,
            executable=bool(os.stat(os.path.join(app, "Dinghy")).st_mode & stat.S_IXUSR),
            profile=prof.decode("latin-1"))
        if conf.get("codesign_status"):
            print("codesign boom", file=sys.stderr); sys.exit(1)
'''


class TestWholeRun(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        for tool in ("xcrun", "security", "codesign"):
            path = bin_dir / tool
            path.write_text(FAKE.format(python=sys.executable, tool=tool))
            path.chmod(0o755)
        self.bin = bin_dir
        self.home = self.tmp / "home"
        (self.home / runner.PROFILE_DIRS[1]).mkdir(parents=True)
        self.manifest = self.tmp / "crate"
        for rel in ("Cargo.toml", "content/c.json", "schema/s.json", "tests/corpus/x", "src/y.rs", ".git/z", "target/w"):
            p = self.manifest / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("x")
        self.binary = self.tmp / "integration-abc"
        self.binary.write_bytes(b"\x7fbinary")
        self.log = self.tmp / "calls.jsonl"
        self.profile_file = self.home / runner.PROFILE_DIRS[1] / "p.mobileprovision"
        self.profile_file.write_bytes(b"signed-cms-bytes")
        self.conf = {
            "devices": listing(WATCH, device(), SIM),
            "identities": '  1) {} "Apple Development: someone@example.invalid (ABC123)"\n     1 valid identities found\n'.format(CERT_SHA),
            "profile_plist": str(self.tmp / "profile.plist"),
            "launch_lines": GOOD_LOG,
        }
        self.set_profile(profile_data())

    def set_profile(self, data):
        (self.tmp / "profile.plist").write_bytes(plistlib.dumps(data))

    def run_runner(self, args=("corpus::", "the_golden_hash_is_the_same_on_every_platform")):
        (self.tmp / "conf.json").write_text(json.dumps(self.conf))
        env = dict(os.environ, HOME=str(self.home), CARGO_MANIFEST_DIR=str(self.manifest),
                   PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                   FAKE_CONF=str(self.tmp / "conf.json"), FAKE_LOG=str(self.log))
        return subprocess.run([sys.executable, str(SCRIPT), str(self.binary)] + list(args),
                              env=env, capture_output=True, text=True, cwd=str(self.manifest))

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(l) for l in self.log.read_text().splitlines()]

    def labels(self):
        return [c["label"] for c in self.calls()]

    def test_good_run(self):
        r = self.run_runner()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.labels(), [
            "devicectl list", "security find-identity", "security cms", "codesign sign",
            "codesign verify", "devicectl install", "devicectl launch"])
        launch = self.calls()[-1]["argv"]
        self.assertEqual(launch, runner.launch_command(UDID, ["corpus::", "the_golden_hash_is_the_same_on_every_platform"])[1:])
        self.assertIn("26 passed", r.stdout)

    def test_bundle_contents_and_signing(self):
        self.assertEqual(self.run_runner().returncode, 0)
        sign = [c for c in self.calls() if c["label"] == "codesign sign"][0]
        self.assertEqual(sorted(sign["files"]), sorted([
            "Dinghy", "Info.plist", "embedded.mobileprovision", "Cargo.toml",
            "content/c.json", "schema/s.json", "tests/corpus/x"]))
        self.assertTrue(sign["executable"])
        self.assertEqual(sign["profile"], "signed-cms-bytes")
        self.assertEqual(sign["entitlements"], ENTITLEMENTS)
        argv = sign["argv"]
        self.assertIn("--force", argv)
        self.assertEqual(argv[argv.index("--sign") + 1], CERT_SHA)
        self.assertFalse(Path(sign["app"]).exists())

    def test_no_phone(self):
        self.conf["devices"] = listing(WATCH, SIM)
        r = self.run_runner()
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue(r.stderr.startswith("ios-runner:"), r.stderr)
        self.assertEqual(self.labels(), ["devicectl list"])

    def test_expired_profile(self):
        self.set_profile(profile_data(expires=datetime.datetime(2020, 1, 1)))
        r = self.run_runner()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("expired", r.stderr)
        self.assertNotIn("devicectl install", self.labels())

    def test_sign_failure(self):
        self.conf["codesign_status"] = 1
        r = self.run_runner()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("ios-runner: sign failed", r.stderr)
        self.assertIn("codesign boom", r.stderr + r.stdout)
        self.assertNotIn("devicectl install", self.labels())
        self.assertFalse(Path([c for c in self.calls() if c["label"] == "codesign sign"][0]["app"]).exists())

    def test_failing_and_silent_apps(self):
        self.conf["launch_lines"] = [
            "test result: FAILED. 25 passed; 1 failed;", "The app terminated with the exit code 101."]
        self.assertNotEqual(self.run_runner().returncode, 0)
        self.conf["launch_lines"] = ["The app terminated with the exit code 0."]
        r = self.run_runner()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("ios-runner:", r.stderr)

    def test_other_profile_folder(self):
        old = self.home / runner.PROFILE_DIRS[0]
        old.mkdir(parents=True)
        shutil.move(str(self.profile_file), str(old / "p.mobileprovision"))
        shutil.rmtree(self.home / runner.PROFILE_DIRS[1])
        self.assertEqual(self.run_runner().returncode, 0)
        self.assertIn("devicectl launch", self.labels())


class TestRepository(unittest.TestCase):
    def test_script_is_executable_python3(self):
        self.assertTrue(os.access(SCRIPT, os.X_OK))
        self.assertEqual(SCRIPT.read_text().splitlines()[0], "#!/usr/bin/env python3")

    def test_cargo_config_points_at_it(self):
        config = (REPO / ".cargo" / "config.toml").read_text()
        self.assertIn('runner = "scripts/ios_device_runner.py"', config)
        self.assertTrue((REPO / "scripts" / "ios_device_runner.py").is_file())

    def test_bundled_folders_exist(self):
        for name in runner.BUNDLED:
            self.assertTrue((REPO / name).exists(), name)


if __name__ == "__main__":
    unittest.main()
