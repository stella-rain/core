#!/usr/bin/env bash
# Cargo runner for `aarch64-apple-ios-sim` (ios-sim.yml, ADR-039): runs a test binary inside the
# booted iOS Simulator. Usage, set by cargo: ios-sim-runner.sh <binary> [arguments...]
#
# The binary is a host path: a Simulator process runs on the Mac and sees its files, so the tests
# read tests/corpus and write to the target directory as they do everywhere else. The Simulator
# passes on only the variables that start with SIMCTL_CHILD_, so the ones the tests may use are
# forwarded that way.
set -eu

for name in PROPTEST_CASES RUST_BACKTRACE RUST_TEST_THREADS; do
  if [ -n "${!name:-}" ]; then
    export "SIMCTL_CHILD_$name=${!name}"
  fi
done

exec xcrun simctl spawn booted "$@"
