---
paths:
  - ".github/**"
---

# Workflows and scripts

Sources: ADR-SIM-3, ADR-PLAT-1, ADR-REL-3.

## `.github/workflows/`

- Rust gates on x86_64 and arm64 (tests also on Windows and macOS), the mobile `cargo check`,
  the fuzz jobs and the tick benchmarks (`rust.yml`; the bench job uploads `bench-profile-<arch>`). A PR of only `*.md` files skips every
  Rust job, and `rust checks` passes when each job ran or was skipped by right.
- `generated sets` (job `bless`) runs `cargo xtask bless --check` on Linux: it fails, naming the
  stale sets (schema, corpus, goldens, content pack), when regenerating them changes a file.
  `rust checks` requires it. Fix: `cargo xtask bless`, review the diff, commit it.
- Tests run under cargo-nextest, profile `ci` (`.config/nextest.toml`): every failure is
  reported, a test past 30 s is listed SLOW, and each run writes a JUnit file, uploaded as the
  `junit-<arch>` artifact. nextest skips doctests, so `cargo test --doc` runs beside it
  (x86_64 only) and fails the day one is added and breaks.
- The deep tier (`deep.yml`): 2,000 property cases and the `#[ignore]`d measurements, in release,
  weekly, on demand and called by Candidate; not a PR check. The nextest test group
  `deep` runs those long tests one at a time so their times stay comparable.
- The weekly long fuzz run (`fuzz-long.yml`) and the iOS Simulator tests (`ios-sim.yml`, runner
  `.github/scripts/ios-sim-runner.sh`), both called by Candidate (ADR-REL-3).
- Candidate, Promote and Candidate guard (`candidate.yml`, `promote.yml`,
  `candidate-guard.yml`); script tests (`scripts.yml`).
- Project sync, the `CLAUDE.md` check, auto-merge, branch cleanup, actionlint, and the secret and
  key-file scan (`repo-check.yml`).
- A new PR check's workflow name goes into `auto-merge.yml` (see its header).

## `.github/scripts/`

`release.py` (the release's version, checks and notes), `bench_budget.py`, `bench_alarm.py`,
`callgrind_diff.py`, the iOS Simulator runner and the fuzz report, with their tests: `python3 -m unittest discover -s .github/scripts`.

## Gates in CI

- iOS readiness: `cargo check --target aarch64-apple-ios` on every PR (ADR-PLAT-1). The tests
  and the corpus run in the iOS Simulator by hand or from Candidate.
- The replay corpus runs on x86_64, on an arm64 runner, and (tests only) on Windows x86_64 and
  macOS arm64 runners, every PR and weekly on `main` (ADR-SIM-3, ADR-PLAT-1).
