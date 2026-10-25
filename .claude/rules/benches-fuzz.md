---
paths:
  - "benches/**"
  - "fuzz/**"
  - "src/bench_fixtures/**"
  - "Cargo.toml"
  - ".github/scripts/bench_*"
  - ".github/scripts/callgrind_diff.py"
---

# Benches and fuzz targets

Sources: ADR-SIM-3, ADR-SIM-7, ADR-SIM-8, ADR-SVC-1, ADR-REPL-3, ADR-CONT-3.

## Benches (`benches/`)

All count instructions with `iai-callgrind`. They read their stages from `src/bench_fixtures/`.

- `worst_tick.rs`: a worst-case tick in a run that holds every rune, with Rune Haste and a
  damage-over-time status of each kind on every enemy (core#180), every cap of ADR-SIM-7
  reached, a fresh stack of enemies to push apart and aim assist looking through all 500 enemies
  for a target. `bench_fixtures/worst_case.rs` builds the stage; `tests/worst_case.rs` checks it
  holds every cap (`Engine::load`).
- `hit_tick.rs`: friendly-bullet collision at the caps with the crowd at rest, clustered and
  spread over the map (`bench_fixtures/hit_case.rs`, `tests/hit_case.rs`).
- `time_limit_tick.rs`: the `time_limit` mode's horde at its heaviest: 500 enemies of the mode's
  kinds, about 3,950 bullets, a boss and the Main weapon hitting (`bench_fixtures/time_limit_case.rs`,
  `tests/time_limit_case.rs`, core#212).
- `worst_minute.rs`, `worst_minute_peak.rs`: the dense minute of the `time_limit` mode (3,600
  ticks) and its heaviest tick by load.

## Tick cost gate

- `cargo bench` runs the five targets (every key of `benches/budgets.toml`); it needs `valgrind`
  and `iai-callgrind-runner` 0.16.1.
- CI (`rust.yml`, ADR-SIM-7, ADR-SIM-8) on both architectures: the budgets of `benches/budgets.toml`
  block on every run (`bench_budget.py`); a case that rose over +2% against the base commit
  (`BENCH_ALARM_PCT`, `bench_alarm.py`) fails unless a new entry of `benches/accepted.toml` covers
  it (`[[accepted]]`: bench, case, arch, base, count, reason, pr; append-only; Kade merges). An
  unexplained rise is fixed, not accepted.
- A PR changing bench input (`benches/*.rs` or `src/bench_fixtures/`) and code (`src/` outside
  `bench_fixtures`, `Cargo.toml`, `Cargo.lock`) fails, since no comparison could tell which moved
  the count: change the bench in its own PR first. Bench input alone is allowed, uncompared.
  `benches/budgets.toml` and `benches/accepted.toml` are not bench input.
- Every run uploads `bench-profile-<arch>` (`callgrind_diff.py`): per case `*.diff.txt` (exclusive)
  and `*.diff-inclusive.txt`, rows `delta base head function` by size, plus the `callgrind.*.out`
  files. A tick step inserted into `TICK` renumbers `TICK::{closure#N}`: a -X/+X pair, not a cost.
  Locally: `callgrind_annotate` on the downloaded `.out` files.
- The benches build at one codegen unit, base and head alike (`[profile.bench]`, core#192).
- Locally: `-- --save-baseline=base`, then `-- --baseline=base --callgrind-limits=ir=2%` (the CI alarm is the script).

## Fuzz (`fuzz/`)

- `cargo-fuzz` targets for the stage parser, the run after validation, the replay loader (stage
  and run replays), the share-code decoder, the run setup and the content pack (`pack_load`;
  ADR-SVC-1, ADR-REPL-3, ADR-CONT-3). Its own workspace, on nightly.
- `fuzz/tests/smoke.rs` runs them on stable; `regressions/<target>/` keeps the inputs of crashes
  that were fixed.
- Gate, in `fuzz/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, then `cargo +nightly fuzz build`. A PR that touches parsing also runs
  `cargo +nightly fuzz run <target> -- -max_total_time=60` for each of the six targets, seeded
  from the corpus, the bundled content and `content-pack export` (CI: `rust.yml`).
- Weekly, each target runs 7 minutes on a growing cached corpus, and a crash opens a hash-only
  issue (`fuzz-long.yml`, `.github/scripts/fuzz-report.sh`).
