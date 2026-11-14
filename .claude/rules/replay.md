---
paths:
  - "src/replay.rs"
  - "src/run_replay*"
  - "src/run_replay/**"
  - "src/step_log*"
  - "src/step_log/**"
  - "src/savepoint/**"
  - "src/share.rs"
  - "src/base64.rs"
  - "src/recording.rs"
  - "src/content_hash.rs"
  - "src/parsed_stage_hash.rs"
  - "src/bin/stage_verify.rs"
  - "tests/corpus/**"
  - "tests/golden/**"
  - "tests/*.rs"
  - "tests/common/run_steps.rs"
---

# Replays, hashes and the tests

Sources: ADR-REPL-5, ADR-REPL-6, ADR-BRDG-3, ADR-CONT-6, ADR-SVC-7, ADR-SIM-3, ADR-SAVE-4.

## What is where

- `replay.rs`, `run_replay.rs`: the `.replay` file, record and verify (ADR-REPL-6, ADR-BRDG-3).
  A run's replay holds the setup, the content SHA-256 and the inputs (the run's picks are ACTIONS
  steps of the step log; a schema 1 replay's `commands`); it is verified from the file and its run stats (ADR-REPL-5).
- The step log, the range reader and the recorder (`step_log.rs`, `RunRecorder`):
  `step-log.md`.
- `share.rs`, `base64.rs`: share codes; `recording.rs`: the `record` feature's JSON Lines.
  `content_hash.rs`: SHA-256 of the two file hashes (ADR-CONT-6).
  `parsed_stage_hash.rs`: the blocklist's hash (ADR-SVC-7 point 4), pinned in `tests/parsed_stage_hash.rs`.
- `stage-verify` (`src/bin/stage_verify.rs`): re-simulates a replay, checks the clear and hash.

## Tests

- `tests/integration.rs` declares the other files: `cargo test --test integration <file>::`. The equipment mode:
  `tests/equipment_slots.rs`, `equipment_drops.rs`, `equipment_input.rs`, `equipment_party.rs`, `equipment_run.rs` (a solo
  run played and recorded by `play_equipment_run*`, `tests/run_play.rs`, over `common::equip_run`), `equipment_pack.rs`
  (the pack's rules equal what the code was tested with) and `tests/weapon_tree.rs`. A test replays a run case through
  `common::run_steps` (`tests/common/run_steps.rs`; `step-log.md`), never the solo views, which refuse a log with an
  equipment action, a join or a leave.
- `tests/corpus/current/` is the blessed tree: `stages/<case>/` holds `stage.json`, `run.replay`,
  `hashes.bin` (the state hash after every tick) and `case.json` (the pack it pins); `runs/<case>/`
  holds a run's `run.replay` (`checkpoints` every 60th tick, ADR-REPL-5) and `case.json`: 11 cases,
  every one an equipment run (ACTIONS steps) but `tutorial_opening`, the tutorial cut at 240 ticks;
  `parties/<case>/` holds a party run's `run.replay` and `case.json` only (no `hashes.bin`, no
  `stage.json`): 19 cases, all of `sim_version` 2: `two_time_limit` (2 players), `three_time_limit`,
  `three_tutorial` (3), `join_two` (a solo run that takes slot 1 at tick 600), `leave_hole` (3 players;
  slot 1 leaves at tick 900, slot 2 leaves, then rejoins), `two_revive` and `three_revive` (a downed
  player revived), the nine equipment cases `equipment_solo`, `_two`, `_party_1` to `_party_3`, `_equip`,
  `_craft`, `_levels` and `_lumine` (actions, drops, Starlight, levels; `equipment.md`) and the building
  rig `equipment_rig_1` to `_3` (`party-play.md`); bless writes them, `tests/party_run.rs` pins the table,
  and the solo guards (`runs/` only) skip them. The party bot hunts the boss once no enemy is left on the field.
  `packs/<sha256>.json` the pinned packs besides the bundled one. `tests/corpus/v<N>/`
  (N >= 1) is a frozen version of the same shape (no `parties/`: `sim_version` 1 has one player)
  plus `manifest.sha256`: never edited (CI `frozen
  corpus unchanged`, `corpus::the_frozen_corpus_is_unchanged`); `v1/` is recorded by `cargo xtask
  bless --freeze v1`. `corpus.rs` checks it all (ADR-SIM-3). The `app` repo reads one file,
  `tests/corpus/current/runs/stage_skill/run.replay`; it is not renamed without the app's bump
  (`tests/corpus_contract.rs`). CI readers use `.github/scripts/corpus_paths.py`. An intended
  change: `cargo xtask bless` (it also regenerates `tests/golden/`, `tests/bench_pins.toml` and the fuzz regression);
  CI runs `cargo xtask bless --check`. `cargo insta test --features record --review` reviews one.
- `tests/savepoint_proof.rs`: for every solo run case (current and frozen `v1/`) a savepoint
  restored mid-run gives the uninterrupted run's hashes (ADR-SAVE-4); the party round trips are
  in `tests/party_run.rs`. `savepoint_load` in `fuzz/` fuzzes it, with a party run among its seeds.

## Pins

A pinned format or hash is never re-pinned: a change is a new version.

- Savepoint: `savepoint.md`.
- `GOLDEN_HASH` (`tests/engine.rs`) and the extremes pin (`tests/extremes.rs`): a rules change is
  a new `sim_version`. Counts that may only fall: `tests/alloc.rs`, `MEASURED_MOST`
  (`tests/command_cap.rs`), `tests/bench_pins.toml`. `tests/pick_pins.rs` was re-pinned once.
- The frozen `tests/corpus/v<N>/` and `SIM_V1_SOURCE` (`src/sim/frozen_tests.rs`): a constant
  that moves says why in the same commit. `tests/corpus_keeps.rs`: a re-blessed `current/` keeps
  each case's ticks, outcome, rewards and Stardust; its table is hand-written, never blessed.
