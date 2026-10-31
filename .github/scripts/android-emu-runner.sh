#!/usr/bin/env bash
# Cargo runner for `x86_64-linux-android` (android-emu.yml): runs a test binary on the booted
# emulator. Usage, set by cargo: android-emu-runner.sh <binary> [arguments...]
#        android-emu-runner.sh --selfcheck
#
# The device path of everything equals the build host's: the workflow builds from a copy of the
# tree at /data/local/tmp/core, so the CARGO_MANIFEST_DIR that cargo bakes into the tests names a
# folder the device has too. The tree is pushed once per device, then each binary at its own path.
# `adb shell` passes no environment and no working directory, so both are built into the command;
# the directory is not the tree on purpose: the tests must not depend on the cwd.
set -eu

# Modules of the one `integration` binary that start a host process through CARGO_BIN_EXE_*,
# a host path the device does not have. libtest --skip matches a substring of the test name.
SKIP_INTEGRATION=(
  stage_verify::          # runs the host stage-verify binary
  content_pack::          # runs the host content-pack binary
  content_pack_compare::  # runs the host content-pack binary
  content_pack_unpack::   # runs the host content-pack binary
  tuning::                # runs a host binary
)

# Variables the tests may use; adb shell passes none of the host's.
FORWARD=(PROPTEST_CASES RUST_BACKTRACE RUST_TEST_THREADS)

fail() {
  echo "android-emu-runner: $*" >&2
  exit 1
}

# One word for the device shell, single-quoted.
q() {
  local s=$1
  printf "'%s'" "${s//\'/\'\\\'\'}"
}

dev() {
  adb shell "$1" || fail "adb shell failed: $1"
}

if [ "${1:-}" = --selfcheck ]; then
  status=0
  adb shell 'exit 7' || status=$?
  [ "$status" = 7 ] || fail "adb shell 'exit 7' returned $status: the exit status does not travel"
  exit 0
fi

root=${CARGO_MANIFEST_DIR:?CARGO_MANIFEST_DIR is not set; run this as a Cargo runner}
[ "$#" -ge 1 ] || fail "usage: android-emu-runner.sh <binary> [arguments...]"
binary=$1
shift

if ! adb shell "test -e $(q "$root/.android-emu-tree")"; then
  dev "mkdir -p $(q "$root")"
  shopt -s dotglob nullglob
  for path in "$root"/*; do
    case ${path##*/} in .git | target) continue ;; esac
    adb push "$path" "$root/" > /dev/null || fail "adb push $path failed"
  done
  dev "touch $(q "$root/.android-emu-tree")"
fi

dev "mkdir -p $(q "$root/target/tmp") $(q "$root/target/corpus-diff") $(q "$(dirname "$binary")")"
adb push "$binary" "$binary" > /dev/null || fail "adb push $binary failed"
dev "chmod 755 $(q "$binary")"

args=("$@")
case $(basename "$binary") in
  integration-*)
    for s in "${SKIP_INTEGRATION[@]}"; do
      args+=(--skip "$s")
    done
    ;;
esac

cmd="cd /data/local/tmp && env"
for name in "${FORWARD[@]}"; do
  if [ -n "${!name:-}" ]; then
    cmd+=" $name=$(q "${!name}")"
  fi
done
cmd+=" INSTA_WORKSPACE_ROOT=$(q "$root") INSTA_UPDATE=no $(q "$binary")"
for a in "${args[@]}"; do
  cmd+=" $(q "$a")"
done

exec adb shell "$cmd"
