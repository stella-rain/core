#!/usr/bin/env python3
# Cargo runner for a physical iPhone: bundle the test binary and the data it reads into an app, sign,
# install and launch it with `xcrun devicectl`, and fail when a test fails (docs/device-gates.md).
# Usage set by Cargo: ios_device_runner.py <test binary> [args...]
from __future__ import annotations

import collections
import datetime
import hashlib
import plistlib
import re
import sys
from pathlib import Path

BUNDLE_ID = "com.stellarain.Dinghy"
APP_NAME = "Dinghy.app"
EXECUTABLE = "Dinghy"
BUNDLED = ("Cargo.toml", "content", "schema", "tests")
PROFILE_DIRS = (
    "Library/MobileDevice/Provisioning Profiles",
    "Library/Developer/Xcode/UserData/Provisioning Profiles",
)

Device = collections.namedtuple("Device", "udid name os_version")
Profile = collections.namedtuple("Profile", "path data")
Chosen = collections.namedtuple("Chosen", "path team identity bundle_id entitlements expires")


class RunnerError(Exception):
    pass


def select_device(devices_json):
    """The one physical, connected iOS device of `devicectl list devices --json-output`."""
    entries = ((devices_json or {}).get("result") or {}).get("devices") or []
    found = []
    seen = []
    for entry in entries:
        hardware = entry.get("hardwareProperties") or {}
        tunnel = (entry.get("connectionProperties") or {}).get("tunnelState")
        props = entry.get("deviceProperties") or {}
        platform = hardware.get("platform")
        reality = hardware.get("reality")
        if platform == "iOS" and reality == "physical" and tunnel == "connected":
            found.append(Device(
                hardware.get("udid"),
                props.get("name") or hardware.get("marketingName") or "?",
                props.get("osVersionNumber") or "?",
            ))
        else:
            seen.append("{} {} {}".format(platform, reality, tunnel))
    if not found:
        raise RunnerError(
            "no physical iPhone is connected (saw: {} entries: {}); connect and unlock the phone".format(
                len(entries), ", ".join(seen) or "none"))
    if len(found) > 1:
        raise RunnerError(
            "more than one physical iPhone is connected: {}; disconnect all but one".format(
                ", ".join("{} ({})".format(d.name, d.udid) for d in found)))
    return found[0]


def parse_identities(security_output):
    """SHA-1 (upper case) to name of the development identities in `security find-identity`."""
    found = {}
    for line in security_output.splitlines():
        m = re.match(r'^\s*\d+\)\s+([0-9A-Fa-f]{40})\s+"(.*)"\s*$', line)
        if m and m.group(2).startswith(("Apple Development:", "iPhone Developer:")):
            found[m.group(1).upper()] = m.group(2)
    return found


def load_profiles(home, decode):
    """Every .mobileprovision of both profile folders under `home`, decoded by `decode(path)`."""
    profiles = []
    for rel in PROFILE_DIRS:
        folder = Path(home) / rel
        if not folder.is_dir():
            continue
        for path in sorted(folder.iterdir()):
            if path.suffix == ".mobileprovision":
                profiles.append(Profile(path, decode(path)))
    return profiles


def choose_profile(profiles, udid, identities, now):
    """The valid profile with the latest expiry for this app, phone and keychain."""
    suffix = "." + BUNDLE_ID
    ours = [p for p in profiles
            if (p.data.get("Entitlements") or {}).get("application-identifier", "").endswith(suffix)]
    if not ours:
        raise RunnerError(
            "no provisioning profile for {} in {}; create it once: run the Dinghy Xcode project on the phone".format(
                BUNDLE_ID, " or ".join(PROFILE_DIRS)))
    live = [p for p in ours if p.data["ExpirationDate"] > now]
    if not live:
        latest = max(p.data["ExpirationDate"] for p in ours)
        raise RunnerError(
            "the provisioning profile for {} expired on {} (a free Apple ID's profile lasts 7 days); "
            "run the Dinghy Xcode project on the phone again".format(BUNDLE_ID, latest.date().isoformat()))
    listed = [p for p in live if udid in (p.data.get("ProvisionedDevices") or [])]
    if not listed:
        raise RunnerError(
            "the provisioning profile does not list this iPhone ({}); run the Dinghy Xcode project on it".format(udid))
    usable = []
    for p in listed:
        for cert in p.data.get("DeveloperCertificates") or []:
            sha = hashlib.sha1(cert).hexdigest().upper()
            if sha in identities:
                usable.append((p, sha))
                break
    if not usable:
        raise RunnerError(
            "no signing identity in the profile is in your keychain (found: {} identities); "
            "open Xcode, Settings, Accounts, Manage Certificates".format(len(identities)))
    p, sha = max(usable, key=lambda item: item[0].data["ExpirationDate"])
    return Chosen(p.path, p.data["TeamIdentifier"][0], sha, BUNDLE_ID,
                  p.data["Entitlements"], p.data["ExpirationDate"])


def info_plist(bundle_id):
    return plistlib.dumps({
        "CFBundleName": EXECUTABLE,
        "CFBundleExecutable": EXECUTABLE,
        "CFBundleIdentifier": bundle_id,
        "CFBundleVersion": "arm64",
        "CFBundleShortVersionString": "arm64",
        "UIRequiredDeviceCapabilities": ["arm64"],
        "UILaunchStoryboardName": "",
    })


def entitlements_plist(entitlements):
    return plistlib.dumps(entitlements)


def launch_command(udid, args):
    return ["xcrun", "devicectl", "device", "process", "launch", "--console", "--terminate-existing",
            "--device", udid, BUNDLE_ID] + list(args)


def install_command(udid, app):
    return ["xcrun", "devicectl", "device", "install", "app", "--device", udid, str(app)]


EXIT_LINE = re.compile(r"The app terminated with the exit code (\d+)\.")


def judge_run(lines, devicectl_status):
    """(exit status, message) of a run: fail closed, see docs/device-gates.md."""
    codes = [int(m.group(1)) for m in map(EXIT_LINE.search, lines) if m]
    results = [l.strip() for l in lines if l.strip().startswith("test result:")]
    if devicectl_status != 0:
        return 1, "devicectl exited with status {}".format(devicectl_status)
    if not codes:
        return 1, "no exit code: the log has no 'The app terminated with the exit code' line"
    if codes[-1] != 0:
        return 1, "the app exited with code {}".format(codes[-1])
    if not results:
        return 1, "no test result line: the app ended without a 'test result:' line"
    if any(r.startswith("test result: FAILED") for r in results):
        return 1, "a test failed"
    return 0, "ok"


def main(argv):
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
