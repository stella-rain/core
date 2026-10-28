#!/usr/bin/env python3
# playtest: plays this checkout's content pack in a debug build of the side-by-side app (ADR-131, ADR-CONT-5).
#
#   python scripts/playtest.py [--app ../app] [--profile test-env] [--build]
#
# It refuses on a version mismatch between core and app; runs `content-pack verify pack.json`
# when content-pack is on PATH (else says how to install it at that ref); with --build runs the
# app's debug build (cargo build --features record in godot-ext); then launches Godot on the app
# with --content-path and --profile (debug builds only, ADR-104). The Godot binary is $GODOT,
# else `godot` on PATH. It changes nothing in either checkout; the profile keeps the pack, save
# and replays apart.

import os
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
PROFILE = re.compile(r"^[A-Za-z0-9_-]{1,40}$")
CORE_URL = "https://github.com/stella-rain/core"
TAG = re.compile(r"^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")


def parse_args(argv):
    opts = {"app": "../app", "profile": "test-env", "build": False}
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--build":
            opts["build"] = True
        elif a in ("--app", "--profile"):
            if i + 1 >= len(argv):
                raise ValueError(f"{a} needs a value")
            i += 1
            opts[a[2:]] = argv[i]
        else:
            raise ValueError(f"unknown option {a}")
        i += 1

    if not PROFILE.match(opts["profile"]):
        raise ValueError("--profile is 1 to 40 letters, digits, - or _")
    return opts


def app_pin(cargo_toml):
    pins = set()
    found = 0
    for line in cargo_toml.splitlines():
        if not re.match(r"^\s*stella-rain-core\s*=", line):
            continue
        found += 1
        m = re.search(r'\b(rev|tag)\s*=\s*"([^"]+)"', line)
        if not m:
            raise ValueError(
                "the app's stella-rain-core has neither rev or tag (a branch pin cannot be play-tested)"
            )
        kind = "commit" if m.group(1) == "rev" else "tag"
        pins.add(f"{kind} {m.group(2)}")

    if not found:
        raise ValueError("no stella-rain-core dependency in godot-ext/Cargo.toml")
    if len(pins) > 1:
        raise ValueError(
            f"the app's stella-rain-core pins disagree: {', '.join(sorted(pins))}"
        )
    kind, ref = next(iter(pins)).split(" ")
    return {"kind": kind, "ref": ref}


def parse_pin(text):
    ref = text.strip()
    if TAG.match(ref):
        return {"kind": "tag", "ref": ref}
    if COMMIT.match(ref):
        return {"kind": "commit", "ref": ref}
    raise ValueError(
        "core-version must hold a core tag vX.Y.Z or a full 40-hex core commit"
    )


def read_pin(root=ROOT):
    core_version_file = os.path.join(root, "core-version")
    if os.path.exists(core_version_file):
        with open(core_version_file, "r", encoding="utf-8") as f:
            return parse_pin(f.read().strip())

    cargo_file = os.path.join(root, "Cargo.toml")
    if os.path.exists(cargo_file):
        with open(cargo_file, "r", encoding="utf-8") as f:
            for line in f:
                m = re.match(r'^\s*version\s*=\s*"([^"]+)"', line)
                if m:
                    ver = m.group(1)
                    ref = f"v{ver}" if not ver.startswith("v") else ver
                    return {"kind": "tag", "ref": ref}
    raise ValueError("cannot determine core version")


def check_pins(core, app):
    # Refuses on a version mismatch, not on pin equality (during balancing in core,
    # commits may differ).
    if core["kind"] == "tag" and app["kind"] == "tag" and core["ref"] != app["ref"]:
        raise ValueError(
            f"core-version holds {core['kind']} {core['ref']}, but the app pins core "
            f"{app['kind']} {app['ref']}; version mismatch (bump the app's pin or try again)"
        )


def install_hint(pin):
    flag = "--tag" if pin["kind"] == "tag" else "--rev"
    return f"cargo install --git {CORE_URL} {flag} {pin['ref']} --bin content-pack"


def godot_binary(env):
    return env.get("GODOT") or "godot"


def build_command(app):
    return {
        "cmd": "cargo",
        "args": ["build", "--features", "record"],
        "cwd": os.path.join(app, "godot-ext"),
    }


def launch_command(godot, app, pack_path, profile):
    return {
        "cmd": godot,
        "args": [
            "--path",
            os.path.join(app, "godot"),
            "--",
            f"--content-path={pack_path}",
            f"--profile={profile}",
        ],
    }


def main(argv=None, env=None):
    if argv is None:
        argv = sys.argv[1:]
    if env is None:
        env = os.environ

    opts = parse_args(argv)
    app = os.path.abspath(os.path.join(ROOT, opts["app"]))
    pack = os.path.join(ROOT, "content", "pack.json")
    if not os.path.exists(pack):
        pack = os.path.join(ROOT, "pack.json")

    core = read_pin(ROOT)
    cargo = os.path.join(app, "godot-ext", "Cargo.toml")
    if not os.path.exists(cargo):
        raise ValueError(
            f"{cargo} not found; --app names the app checkout (default ../app)"
        )
    with open(cargo, "r", encoding="utf-8") as f:
        check_pins(core, app_pin(f.read()))

    try:
        verify = subprocess.run(["content-pack", "verify", pack])
        if verify.returncode != 0:
            raise ValueError(
                "content-pack verify refused pack.json; fix it before play-testing"
            )
    except FileNotFoundError:
        hint = install_hint(core)
        raise ValueError(
            f"content-pack is not on PATH; install it at this ref and run again:\n  {hint}"
        )

    if opts["build"]:
        build = build_command(app)
        res = subprocess.run(
            [build["cmd"]] + build["args"],
            cwd=build["cwd"],
        )
        if res.returncode != 0:
            raise ValueError("the app's debug build failed")

    launch = launch_command(godot_binary(env), app, pack, opts["profile"])
    print(f"playtest: {launch['cmd']} {' '.join(launch['args'])}")
    try:
        res = subprocess.run([launch["cmd"]] + launch["args"])
        return res.returncode
    except FileNotFoundError:
        raise ValueError(
            f"Godot not found ({launch['cmd']}); set GODOT to the Godot 4 binary or put it on PATH"
        )


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ValueError as e:
        print(f"playtest: {e}", file=sys.stderr)
        sys.exit(1)
