---
paths:
  - ".github/**"
---

# Workflows and scripts

Sources: ADR-SIM-3, ADR-PLAT-1, ADR-REL-3, ADR-REL-6.

## `.github/workflows/`

- Rust gates on x86_64 and arm64 (debug and release legs; Windows and macOS release only), the mobile `cargo check`,
  the fuzz jobs and the tick benchmarks (`rust.yml`, every target in `benches/budgets.toml`; the bench job uploads `bench-profile-<arch>`: one shard per bench target and arch, merged into that artifact; a PR restores its base's counts from main's push run when the toolchain fingerprint matches, else counts the base itself; a case is compared when its pre-tick digest in `tests/bench_pins.toml` is unchanged). A PR of only `*.md` files skips every
  Rust job. A push to `main` runs `what changed`, the bench and its profile; the rest only when
  `Cargo.lock`, `fuzz/Cargo.lock`, `rust-toolchain.toml` or `rust.yml` changed (they refresh the caches), and weekly.
  `rust checks` passes when each job ran or was skipped by right. Every job installs the toolchain with `rustup toolchain install` (reads `rust-toolchain.toml`).
- `generated sets` (job `bless`) runs `cargo xtask bless --check` on Linux: it fails, naming the
  stale sets (schema, corpus, goldens, content pack, pack pins, fuzz regression, bench pins), when regenerating them changes a file.
  `rust checks` requires it. Fix: `cargo xtask bless`, review the diff, commit it.
- Tests run under cargo-nextest, profile `ci` (`.config/nextest.toml`): every failure is
  reported, a test past 30 s is listed SLOW, and each run writes a JUnit file, uploaded as the
  `junit-<arch>-<profile>` artifact. nextest skips doctests, so `cargo test --doc` runs in the x86_64
  debug leg and fails the day one is added and breaks.
- `tests on x86_64 (release)` also runs the ignored `command_cap::measure_the_longest_run` in release on
  every run of the job (md-only PRs skip it): it counts the commands of the frozen `tests/corpus/v1/runs` replays, a folder
  that never changes, so `MEASURED_MOST` in `tests/command_cap.rs` can only fall; a PR that lowers the count re-pins it.
- The deep tier (`deep.yml`): 2,000 property cases and the `#[ignore]`d measurements, in release,
  weekly, on demand and called by Candidate; not a PR check. The nextest test group
  `deep` runs those long tests one at a time so their times stay comparable.
- The weekly long fuzz run (`fuzz-long.yml`), the iOS Simulator tests (`ios-sim.yml`, runner
  `.github/scripts/ios-sim-runner.sh`) and the Android emulator tests (`android-emu.yml`, runner
  `.github/scripts/android-emu-runner.sh`), all called by Candidate (ADR-REL-3 point 5). ADR-REL-6 point 1
  makes the emulator a Candidate gate, but the job is advisory today: `android-emulator` is not in the `tag` job's `needs` in
  `candidate.yml`, so a red emulator shows on the run and does not stop the rc.
- The device corpus (the Android phones, the iPhone; `docs/device-gates.md`) is not a Candidate check: ADR-REL-6 points 2 and 3
  run it after Promote on the released tag for each `0.N.0` release or on request. The iPhone run is by hand on Kade's Mac; no
  workflow opens the device-check issue yet (ADR-REL-6 point 4).
- Candidate, Promote and Candidate guard (`candidate.yml`, `promote.yml`,
  `candidate-guard.yml`); script tests (`scripts.yml`).
- Promote ends with its `notify` job: after a final release (not a prerelease or draft) it sends
  app a `core-release` dispatch with `client_payload.tag` (a bridge App token that can write only
  to app). A failed dispatch leaves the release and fails the run: re-run the failed job `notify`,
  or run Core bump in app by hand. `bump-app.yml` sends `core-main` on every push to `main`.
- Project sync, the `CLAUDE.md` check, auto-merge, branch cleanup, actionlint, and the secret and
  key-file scan (`repo-check.yml`).
- A new PR check's workflow name goes into `auto-merge.yml` (see its header).

## `.github/scripts/`

`release.py` (the release's version, checks and notes), `bench_budget.py` (the tick budgets),
`bench_alarm.py` (the rise alarm), `bench_pins.py` (which cases compare), `bench_drop_guard.py` (the teardown
guard) and `callgrind_diff.py` (the bench profile), tested by `test_release.py`, `test_bench_budget.py`,
`test_bench_alarm.py`, `test_bench_pins.py`, `test_bench_drop_guard.py` and `test_callgrind_diff.py`; `corpus_paths.py` (where the replay corpus is; tested by
`test_corpus_paths.py`); `fuzz_seconds.py` (fuzz-long's minutes to whole seconds; tested by
`test_fuzz_seconds.py`); `test_candidate_workflow.py`
checks the release workflows' text. The Android runner is tested by `test_android_emu_runner.py` and its
workflow text by `test_android_emu_workflow.py`. The iOS Simulator runner (`ios-sim-runner.sh`) and the
fuzz report (`fuzz-report.sh`) have no tests. Run: `python3 -m unittest discover -s .github/scripts`.

## Gates in CI

- iOS readiness: `cargo check --target aarch64-apple-ios` on every PR (ADR-PLAT-1). The tests
  and the corpus run in the iOS Simulator by hand or from Candidate.
- The replay corpus runs on x86_64, on an arm64 runner, and (tests only) on Windows x86_64 and
  macOS arm64 runners, every PR and weekly on `main` (ADR-SIM-3, ADR-PLAT-1). `frozen corpus unchanged` fails a PR
  that changes a file in a frozen `tests/corpus/v<N>/`: the manifest test runs in every `tests` leg,
  and the `frozen` job (PRs only) is the git diff.
- `cargo deny` is one job on a pinned binary (version and sha256 in `rust.yml`) and checks both manifests.
