#!/usr/bin/env python3
"""Runs core's replay corpus and golden hash on one USB-connected Android phone.

`python3 scripts/android_device_gate.py` builds the release `integration` test binary for
aarch64-linux-android with the NDK's clang as linker, pushes it and the data folders the filter
reads to the phone, runs `corpus:: the_golden_hash_is_the_same_on_every_platform` there, exits with
the test's status and removes what it pushed. See docs/device-gates.md. Standard library only.
"""

from __future__ import annotations

import json
import os
import re

TRIPLE = "aarch64-linux-android"
API = 24
FILTERS = ("corpus::", "the_golden_hash_is_the_same_on_every_platform")
REMOTE_DIR = "/data/local/tmp/stella-core-gate"
# The folders of the repository the filter reads through data_root(); the one place the list lives.
DATA_FOLDERS = ("tests/corpus",)
FORWARDED = ("PROPTEST_CASES", "RUST_BACKTRACE")
NDK_VARIABLES = ("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_NDK_LATEST_HOME")
SDK_VARIABLES = ("ANDROID_HOME", "ANDROID_SDK_ROOT")


class GateError(Exception):
    """A setup error: reported as `android-gate: ...`, exit status 2."""


def _version_key(name: str) -> list[int]:
    return [int(part) for part in re.findall(r"\d+", name)]


def ndk_linker(env, host_tag: str) -> str:
    """The NDK's aarch64 clang (API 24, as android-emu.yml) for `host_tag`, e.g. linux-x86_64."""
    root = next((env[v] for v in NDK_VARIABLES if env.get(v)), None)
    if root is None:
        for var in SDK_VARIABLES:
            ndks = os.path.join(env.get(var) or "", "ndk")
            if env.get(var) and os.path.isdir(ndks):
                versions = sorted((d for d in os.listdir(ndks)
                                   if os.path.isdir(os.path.join(ndks, d))), key=_version_key)
                if versions:
                    root = os.path.join(ndks, versions[-1])
                    break
    if root is None:
        raise GateError("android-gate: no NDK found: set ANDROID_NDK_HOME to the NDK folder "
                        "(or install it under $ANDROID_HOME/ndk)")
    suffix = ".cmd" if host_tag.startswith("windows") else ""
    return "/".join([root.rstrip("/\\"), "toolchains", "llvm", "prebuilt", host_tag, "bin",
                     "%s%d-clang%s" % (TRIPLE, API, suffix)])


def parse_devices(output: str) -> list[tuple[str, str]]:
    """(serial, state) per line of `adb devices`."""
    devices = []
    lines = output.splitlines()
    for line in lines:
        parts = line.split()
        if len(parts) >= 2 and not line.startswith(("*", "List of")):
            devices.append((parts[0], parts[1]))
    return devices


def choose_device(devices, serial) -> str:
    if serial:
        states = dict(devices)
        if serial not in states:
            raise GateError("android-gate: device %s is not connected (saw: %s)"
                            % (serial, ", ".join(s for s, _ in devices) or "none"))
        if states[serial] != "device":
            raise GateError("android-gate: device %s is %s: unlock the phone and accept "
                            "\"Allow USB debugging\", or replug it" % (serial, states[serial]))
        return serial
    usable = [s for s, state in devices if state == "device"]
    if not usable:
        raise GateError("android-gate: no device is connected and authorized (saw: %s)"
                        % (", ".join("%s %s" % d for d in devices) or "none"))
    if len(usable) > 1:
        raise GateError("android-gate: more than one device is connected (%s): pass --serial "
                        "or set ANDROID_SERIAL" % ", ".join(usable))
    return usable[0]


def cargo_command(debug: bool) -> list[str]:
    command = ["cargo", "test", "--locked", "--all-features", "--test", "integration"]
    if not debug:
        command.append("--release")
    return command + ["--target", TRIPLE, "--no-run", "--message-format=json"]


def find_test_binary(json_lines) -> str:
    """The path of the `integration` test executable in cargo's JSON messages."""
    found = None
    for line in json_lines:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if (isinstance(message, dict) and message.get("reason") == "compiler-artifact"
                and message.get("executable")
                and (message.get("target") or {}).get("name") == "integration"
                and (message.get("profile") or {}).get("test")):
            found = message["executable"]
    if found is None:
        raise GateError("android-gate: cargo built no `integration` test binary")
    return found


def push_plan(root: str, binary: str, remote: str) -> list[tuple[str, str]]:
    """(local, remote) pairs: the test binary, then each data folder at its repo-relative name."""
    plan = [(binary, remote + "/integration")]
    plan += [(root.rstrip("/") + "/" + f, remote + "/" + f) for f in DATA_FOLDERS]
    return plan


def quote(word: str) -> str:
    """One word for the phone's shell (`adb shell` joins its arguments and re-parses them)."""
    return "'" + word.replace("'", "'\\''") + "'"


def run_command(remote: str, env) -> str:
    """The one `adb shell` string: adb passes no environment and no cwd, so both are built in."""
    assignments = ["STELLA_TEST_DATA_ROOT=" + quote(remote),
                   "STELLA_TEST_TMP_DIR=" + quote(remote + "/tmp")]
    assignments += ["%s=%s" % (name, quote(env[name])) for name in FORWARDED if name in env]
    words = [quote(remote + "/integration")] + [quote(f) for f in FILTERS]
    return "cd /data/local/tmp && env " + " ".join(assignments + words)


def cleanup_command(remote: str) -> str:
    """`rm -rf` of exactly the gate's folder, never of /data/local/tmp itself."""
    if not remote.startswith("/data/local/tmp/") or remote.rstrip("/") == "/data/local/tmp":
        raise GateError("android-gate: refusing to remove %r" % remote)
    return "rm -rf " + quote(remote)
