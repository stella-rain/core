---
paths:
  - "benches/**"
  - "fuzz/**"
  - "src/bench_fixtures/**"
  - "tests/bench_fixture_pins.rs"
  - "tests/bench_pins.toml"
  - "Cargo.toml"
  - ".github/scripts/bench_*"
  - ".github/scripts/callgrind_diff.py"
---

# Benches and fuzz targets

Sources: ADR-SIM-3, ADR-SIM-7, ADR-SIM-10, ADR-SVC-1, ADR-REPL-3, ADR-CONT-3.

## Benches (`benches/`)

All count instructions with `iai-callgrind` and read their cases from `src/bench_fixtures/`.
Each bench target is a key of `benches/budgets.toml`, which CI reads for the list of benches.

- `worst_tick.rs`: one synthetic tick at the enemy, orb and zone caps of ADR-SIM-7 (derived
  cap of 260 enemies, 32 orbs, 128 zones, DIR1), with every rune, Rune Haste and a damage-over-time
  status of each kind on every enemy (core#180), a fresh stack of enemies to push apart and aim
  assist looking through all 260 for a target. It bounds the frame at the caps.
  `bench_fixtures/worst_case.rs` builds it; `tests/worst_case.rs` checks every cap (`Engine::load`).
- `hit_tick.rs`: the worst melee tick: 260 enemies under the main character's hit box, four
  companions' arcs and 128 zones (112 friendly), no orb in the air; `hit_tick_spread` is the same
  tick with the crowd spread across the view rectangle (`bench_fixtures/hit_case.rs`,
  `tests/hit_case.rs`).
- `time_limit_tick.rs`: the `time_limit` mode's heaviest tick: 260 enemies of the mode's kinds, a
  boss and the Main weapon hitting, no orb (the mode's content fires none since BR1a)
  (`bench_fixtures/time_limit_case.rs`, `tests/time_limit_case.rs`, core#212).
- `worst_minute.rs`: the 3,600 ticks of the mode's dense minute (BR1e-2: ticks 7,301 to 10,900,
  crowd held at target 200 up to cap 260, 2 elites, a mid-boss arriving at tick 9,000), with wave
  spawns and death bursts a one-tick bench misses; `worst_minute_peak.rs` is its heaviest tick
  (found dynamically by `worst_minute_peak()`, pinned in `tests/worst_minute.rs`), a target with its
  own budget (`bench_fixtures/worst_minute_case.rs`, `tests/worst_minute.rs`). The minute does not
  reach orb, zone and elite caps until Z3b brings content firing them; until then `worst_tick` bounds
  the frame there.
- Pins (`tests/bench_pins.toml`, core#360, written by `UPDATE_BENCH_PINS=1 cargo test
  bench_fixture_pins`): per case the state hash `before` and `after` the measured tick or ticks,
  and the `stage` and `input` digests. They are in `tests/`, not bench input. A PR that moves a pin
  updates it from the engine's own value and says so in its body; a pin that moves when the PR did
  not mean to change the case is a bug in that PR.

## Tick cost gate

- `cargo bench` runs the five targets (every key of `benches/budgets.toml`); it needs `valgrind`
  and `iai-callgrind-runner` 0.16.1.
- CI (`rust.yml`, ADR-SIM-7, ADR-SIM-10) on both architectures: the budgets of `benches/budgets.toml`
  block on every run (`bench_budget.py`); a case that rose over +2% against the base commit
  (`BENCH_ALARM_PCT`, `bench_alarm.py`) fails unless a new entry of `benches/accepted.toml` covers
  it (`[[accepted]]`: bench, case, arch, base, count, reason, pr; append-only; Kade merges). An
  unexplained rise is fixed, not accepted.
- A case is compared with the base commit when its `before`, `stage` and `input` pins are equal
  there and here and its bench file and `benches/support/` are unchanged (`bench_pins.py`,
  ADR-SIM-10); `after` is pinned only. A case whose pin moved is printed "case moved: not compared"
  and not gated (`bench_alarm.py --cases`); budgets still block it. Code and bench may change in
  one PR; `accepted.toml` covers rises of compared cases only.
- Every run uploads `bench-profile-<arch>` (`callgrind_diff.py`): per case `*.diff.txt` (exclusive)
  and `*.diff-inclusive.txt`, rows `delta base head function` by size, plus the `callgrind.*.out`
  files. A tick step inserted into `TICK` renumbers `TICK::{closure#N}`: a -X/+X pair, not a cost.
  Locally: `callgrind_annotate` on the downloaded `.out` files.
- The benches build at one codegen unit, base and head alike (`[profile.bench]`, core#192).
- Locally: `-- --save-baseline=base`, then `-- --baseline=base --callgrind-limits=ir=2%` (the CI alarm is the script).

## Fuzz (`fuzz/`)

- `cargo-fuzz` targets for the parsers and loaders (stage, run, replays, share code, run setup,
  content packs, minisign; ADR-SVC-1, ADR-REPL-3, ADR-CONT-3): every file in `fuzz_targets/`, in
  its own workspace, on nightly.
- `fuzz/tests/smoke.rs` runs them on stable; `regressions/<target>/` keeps the inputs of crashes
  that were fixed.
- Gate, in `fuzz/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, then `cargo +nightly fuzz build` (all targets). A PR that touches parsing also
  runs `cargo +nightly fuzz run <target> -- -max_total_time=60` for each target in the
  `fuzz-short` list of `rust.yml`, seeded from the corpus, bundled content and `content-pack export`.
- Weekly, each target in the `fuzz-long.yml` matrix runs 7 minutes on a growing cached corpus,
  and a crash opens a hash-only issue (`.github/scripts/fuzz-report.sh`).
