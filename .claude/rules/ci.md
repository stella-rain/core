---
paths:
  - ".github/**"
---

# Workflows and scripts

Sources: ADR-SIM-3, ADR-PLAT-1, ADR-REL-3.

## `.github/workflows/`

- Rust gates on x86_64 and arm64 (tests also on Windows and macOS), the mobile `cargo check`,
  the fuzz jobs and the tick benchmarks (`rust.yml`, every target in `benches/budgets.toml`; the bench job uploads `bench-profile-<arch>`: one shard per bench target and arch, merged into that artifact; a PR restores its base's counts from main's push run when the toolchain fingerprint matches, else counts the base itself; a case is compared when its pre-tick digest in `tests/bench_pins.toml` is unchanged). A PR of only `*.md` files skips every
  Rust job, and `rust checks` passes when each job ran or was skipped by right.
- `generated sets` (job `bless`) runs `cargo xtask bless --check` on Linux: it fails, naming the
  stale sets (schema, corpus, goldens, content pack, bench pins), when regenerating them changes a file.
  `rust checks` requires it. Fix: `cargo xtask bless`, review the diff, commit it.
- Tests run under cargo-nextest, profile `ci` (`.config/nextest.toml`): every failure is
  reported, a test past 30 s is listed SLOW, and each run writes a JUnit file, uploaded as the
  `junit-<arch>` artifact. nextest skips doctests, so `cargo test --doc` runs beside it
  (x86_64 only) and fails the day one is added and breaks.
- `tests on x86_64` also runs the ignored `command_cap::measure_the_longest_run` in release on
  every run of the job (md-only PRs skip it); a PR that raises the longest bot run's command
  count re-pins `MEASURED_MOST` in `tests/command_cap.rs` in the same PR.
- The deep tier (`deep.yml`): 2,000 property cases and the `#[ignore]`d measurements, in release,
  weekly, on demand and called by Candidate; not a PR check. The nextest test group
  `deep` runs those long tests one at a time so their times stay comparable.
- The weekly long fuzz run (`fuzz-long.yml`), the iOS Simulator tests (`ios-sim.yml`, runner
  `.github/scripts/ios-sim-runner.sh`) and the Android emulator tests (`android-emu.yml`, runner
  `.github/scripts/android-emu-runner.sh`), all called by Candidate (ADR-REL-3); the emulator job is
  advisory (not in the `tag` job's needs).
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
`bench_alarm.py` (the rise alarm), `bench_pins.py` (which cases compare) and `callgrind_diff.py` (the bench
profile), tested by `test_release.py`, `test_bench_budget.py`, `test_bench_alarm.py`, `test_bench_pins.py` and
`test_callgrind_diff.py`; `corpus_paths.py` (where the replay corpus is; tested by
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
  that changes a file in a frozen `tests/corpus/v<N>/`.
