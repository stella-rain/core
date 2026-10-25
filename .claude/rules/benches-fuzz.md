---
paths:
  - "benches/**"
  - "fuzz/**"
  - "src/bench_fixtures/**"
  - "tests/bench_fixture_pins.rs"
  - "Cargo.toml"
---

# Benches and fuzz targets

Sources: ADR-SIM-3, ADR-SIM-6, ADR-SVC-1, ADR-REPL-3, ADR-CONT-3.

## Benches (`benches/`)

All count instructions with `iai-callgrind` and read their cases from `src/bench_fixtures/`.
Each bench target is a key of `benches/budgets.toml`, which CI reads for the list of benches.

- `worst_tick.rs`: one synthetic tick at the enemy, orb and zone caps of ADR-SIM-6 (500 enemies,
  32 orbs, 128 zones), with every rune, Rune Haste and a damage-over-time status of each kind on
  every enemy (core#180), a fresh stack of enemies to push apart and aim assist looking through
  all 500 for a target. It bounds the frame at the caps. `bench_fixtures/worst_case.rs` builds it;
  `tests/worst_case.rs` checks every cap (`Engine::load`).
- `hit_tick.rs`: the worst melee tick: 500 enemies under the main character's hit box, four
  companions' arcs and 128 zones (112 friendly), no orb in the air; `hit_tick_spread` is the same
  tick with the crowd spread over the largest map (`bench_fixtures/hit_case.rs`,
  `tests/hit_case.rs`).
- `time_limit_tick.rs`: the `time_limit` mode's heaviest tick: 500 enemies of the mode's kinds, a
  boss and the Main weapon hitting, no orb (the mode's content fires none since BR1a)
  (`bench_fixtures/time_limit_case.rs`, `tests/time_limit_case.rs`, core#212).
- `worst_minute.rs`: the 3,600 ticks of the mode's dense minute (BR1e-2: ticks 7,300 to 10,900,
  500 enemies, 2 elites, a mid-boss arriving at tick 9,000), with the wave spawns and death bursts
  a one-tick bench misses; `worst_minute_peak.rs` is its heaviest tick (10,800), a target of its
  own so it has a budget of its own (`bench_fixtures/worst_minute_case.rs`,
  `tests/worst_minute.rs`). The minute does not reach the orb, zone and elite caps (about 0 of 32
  orbs, 5 to 14 of 128 zones, 2 of 32 elites) until Z3b (core#319) brings content that fires them
  and BR1e-3 (core#370) re-scripts it (DS55); until then `worst_tick` bounds the frame there.
- Pins (`tests/bench_fixture_pins.rs`, core#360): the state hash before and after the measured
  tick or ticks of each case. They are in `tests/`, not bench input, so the bench guard does not
  refuse a `src/` change that moves one: that PR updates the pin from the engine's own value (the
  failing `assert_eq` prints it) and says so in its body. A pin that moves when the PR did not
  mean to change the case is a bug in that PR.

## Tick cost gate

- `cargo bench --bench worst_tick --bench hit_tick --bench time_limit_tick --bench worst_minute
  --bench worst_minute_peak` needs `valgrind` and `iai-callgrind-runner` 0.16.1.
- CI (`rust.yml`, ADR-SIM-6) compares every bench target with the base commit's on both architectures and
  fails on more than +2% instructions in any of them. It warns on `worst_tick` over its ceiling
  per arch in `benches/budgets.toml` (`bench_budget.py --warn-only` until the T1 refactor, DS27);
  a PR changing bench input
  (`benches/` or `src/bench_fixtures/`) and code (`src/` outside
  `bench_fixtures`, `Cargo.toml`, `Cargo.lock`) fails, since no comparison could tell which moved
  the count (D-BUDGET): change the bench in its own PR first. A PR of bench input alone is allowed,
  uncompared.
- The benches build at one codegen unit, base and head alike (`[profile.bench]`, core#192).
- Locally: `-- --save-baseline=base`, then `-- --baseline=base --callgrind-limits=ir=2%`.

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
