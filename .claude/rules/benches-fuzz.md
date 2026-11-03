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

Sources: ADR-SIM-3, ADR-SIM-5, ADR-SIM-6, ADR-SVC-7, ADR-REPL-5, ADR-CONT-5.

## Benches (`benches/`)

All count instructions with `iai-callgrind`, read their cases from `src/bench_fixtures/`; each target
is a key of `benches/budgets.toml`, which CI reads for the list. They build stages at the live
`sim_version` (`bench_fixtures/common/mod.rs`), as `tests/alloc.rs` does: `sim_v1` and the v1
event adapter are not benched.

- `worst_tick.rs`: one synthetic tick at the caps of ADR-SIM-5 (260 enemies, 32 orbs, 128 zones,
  DIR1), every rune, Rune Haste and each damage-over-time status on every enemy, a fresh
  stack to push apart and aim assist over all 260. `bench_fixtures/worst_case.rs`; `tests/worst_case.rs`.
- `hit_tick.rs`: worst melee tick: 260 enemies under the main character's hit box, four
  companions' arcs, 128 zones (112 friendly), no orb; `hit_tick_spread` spreads the crowd
  (`bench_fixtures/hit_case.rs`, `tests/hit_case.rs`).
- `time_limit_tick.rs`: the `time_limit` mode's heaviest tick: 260 enemies, a boss and the Main
  weapon hitting, no orb (`bench_fixtures/time_limit_case.rs`).
- `worst_minute.rs`: the 3,600 ticks of the mode's dense minute (ticks 7,301 to 10,900: crowd held
  at 200 up to cap 260, up to 32 elites, a mid-boss at tick 9,000), with the spawns and death
  bursts a one-tick bench misses; `worst_minute_peak.rs` is its heaviest tick, with its own
  budget (`bench_fixtures/worst_minute_case.rs`; `tests/worst_minute.rs` pins it).
- Pins (`tests/bench_pins.toml`, written by `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins`):
  per case the state hash `before`/`after` the measured tick(s) and the `stage` and `input`
  digests. A PR that moves a pin updates it and says so; an unmeant move is a bug.

## Tick cost gate

- `cargo bench` runs the five targets (needs `valgrind`, `iai-callgrind-runner` 0.16.1). CI
  (`rust.yml`, ADR-SIM-5, ADR-SIM-6) runs both architectures: the budgets of `benches/budgets.toml`
  block on every run (`bench_budget.py`); a case that rose over +2% against the base commit
  (`BENCH_ALARM_PCT`, `bench_alarm.py`) fails unless a new `benches/accepted.toml` entry covers it
  (`[[accepted]]`: bench, case, arch, base, count, reason, pr; append-only; Kade merges).
- A case is compared with the base commit when its `before`, `stage` and `input` pins are equal
  there and here and its bench file and `benches/support/` are unchanged (`bench_pins.py`,
  ADR-SIM-6 point 5); `after` is pinned only. A moved case prints "case moved: not compared", is not
  gated (`bench_alarm.py --cases`); budgets still block it.
- Teardown guard (`bench_drop_guard.py`, after the budget check): `callgrind_annotate --tree=caller`
  fails a `drop_glue`/`drop_in_place` of an Engine, Run or Case called from
  `__iai_callgrind_wrapper_mod`. In 0.16 only that function is counted, not the teardown: a bench
  returns its fixture and ends with `teardown = support::forget_measured`; an unreadable profile
  fails. `tests/bench_teardown.rs` is its text-level twin.
- Every run uploads `bench-profile-<arch>` (`callgrind_diff.py`): per case `*.diff.txt`,
  `*.diff-inclusive.txt` (rows `delta base head function`), `*.steps.txt` (head Ir per tick step of
  `steps![..]` in `src/sim/schedule.rs`, plus `rest`; base and delta when compared), `callgrind.*.out`; nested frames give "step table unreliable on <arch>". Locally:
  `callgrind_annotate`, or `-- --save-baseline=base` then `-- --baseline=base --callgrind-limits=ir=2%`.

## Fuzz (`fuzz/`)

- `cargo-fuzz` targets for the parsers and loaders (stage, run, replays, share code, run setup,
  content packs, minisign, savepoints, step log ranges; ADR-SVC-7, ADR-REPL-5, ADR-CONT-5): every
  file in `fuzz_targets/`, in its own workspace, on nightly.
- `fuzz/tests/smoke.rs` runs them on stable; `regressions/<target>/` keeps the inputs of fixed
  crashes. `savepoint_load` and `step_log_range` are seeded from generated files
  (`write_savepoint_seeds`, `write_step_log_range_seeds`; both `#[ignore]`).
  `step_log_range` checks the range reader against an expanded-ticks oracle (`fuzz/src/lib.rs`).
- The root profile never reaches the fuzz workspace: `fuzz/Cargo.toml` sets `opt-level = 1` and
  `overflow-checks` (`cargo test` there 84 s, was about 13 minutes; `the_fuzz_crate_tests_are_built_optimised`).
  The sweeps are bounded by `STEP_LOG_RANGE_TICK_BUDGET` and `RUN_SEED_TICK_BUDGET`
  (`the_step_log_range_seeds_expand_a_bounded_number_of_ticks`,
  `the_run_seeds_simulate_a_bounded_number_of_ticks`); raising one takes CI minutes: Kade's call.
- Gate, in `fuzz/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  then `cargo +nightly fuzz build` (all targets). A PR that touches parsing also runs
  each target of `fuzz/Cargo.toml` 60 s, one per CPU (`pack_load` with a whole pack as `-max_len`)
  (`.github/scripts/fuzz_targets.py` lists them; a unit test fails when `fuzz_targets/` and it
  disagree), seeded from the corpus, bundled content, `content-pack export` and the seeds above.
- Weekly, each target runs 7 minutes on a growing cached corpus; a crash opens a hash-only issue.
