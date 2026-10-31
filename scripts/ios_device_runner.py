#!/usr/bin/env python3
# Cargo runner for a physical iPhone: bundle the test binary and the data it reads into an app, sign,
# install and launch it with `xcrun devicectl`, and fail when a test fails (docs/device-gates.md).
# Usage set by Cargo: ios_device_runner.py <test binary> [args...]
# STUB: every function returns a wrong value; the tests are written first.
from __future__ import annotations

import collections
import sys

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
    return None


def parse_identities(security_output):
    return {}


def load_profiles(home, decode):
    return []


def choose_profile(profiles, udid, identities, now):
    return None


def info_plist(bundle_id):
    return b""


def entitlements_plist(entitlements):
    return b""


def launch_command(udid, args):
    return []


def install_command(udid, app):
    return []


def judge_run(lines, devicectl_status):
    return (0, "")


def main(argv):
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
