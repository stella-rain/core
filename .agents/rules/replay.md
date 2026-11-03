---
globs:
  - "src/replay.rs"
  - "src/run_replay.rs"
  - "src/step_log.rs"
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
---

# Replays, hashes and the tests

Sources: ADR-REPL-3, ADR-REPL-4, ADR-BRDG-3, ADR-SVC-6, ADR-SIM-3.

## What is where

- `replay.rs`, `run_replay.rs`: the `.replay` file, record and verify (ADR-REPL-4, ADR-BRDG-3).
  A run's replay holds the setup, the content SHA-256 and the inputs (picks, `choose` and
  `spend`, are steps of the step log); it is verified from the file and its run stats (ADR-REPL-3).
- `step_log.rs`: the in-memory model of a run's inputs for both schemas. Schema 2 `inputs` bytes:
  a `present` byte (bit `i` is slot `i`, at least one set, bits 3-7 zero), then records.
  `0x00` TICKS: a varint `run` >= 1, then a 3-byte block (`flags`, `move_x`, `move_y`) per present
  slot, for `run` ticks; two TICKS in a row never hold equal blocks. `0x01` PICKS: one step that
  does not advance the world, one byte per present slot (`0x00` none, `0x01` and a varint index),
  at least one. `0x02` JOIN and `0x03` LEAVE are reserved and refused. Runs sum to `ticks`; at
  most `MAX_COMMANDS` (1000, `run_replay.rs`) PICKS. Bit 6 of `flags` is `interact`; an axis of
  -128 is written -127 (`block_of`, `replay.rs`). `SCHEMA_VERSION` 2 is written, 1 and 2 read
  (`version.rs`); schema 1 migrates in memory (`from_schema1`); `solo_commands` and `to_solo`
  refuse `present != 0b1`.
- `read_step_range(bytes, ticks, max_ticks, a, b)` and `StepLog::range`: ticks 0-based, range
  half-open. A TICKS run overlapping it is returned once, clipped, never expanded (memory is
  bounded by the records in the range). A PICKS step after `p` ticks is in iff `a <= p < b`; a
  range with `b == ticks` also holds the picks after the last tick. The whole log is validated
  first; `a > b` and `b > ticks` are `BadInputs`, before the bytes are read.
  Tests: `tests/schema2_range.rs`; fuzz target `step_log_range`.
- `share.rs`, `base64.rs`: share codes; `recording.rs`: the `record` feature's JSON Lines.
  `content_hash.rs`: SHA-256 of the two file hashes (ADR-SVC-6).
  `parsed_stage_hash.rs`: the blocklist's hash (ADR-SVC-6), pinned in `tests/parsed_stage_hash.rs`.
- `stage-verify` (`src/bin/stage_verify.rs`): re-simulates a replay, checks the clear and hash.

## Tests

- `tests/integration.rs` declares the other files: `cargo test --test integration <file>::`.
- `tests/corpus/current/` is the blessed tree: `stages/<case>/` holds `stage.json`, `run.replay`,
  `hashes.bin` (the state hash after every tick) and `case.json` (the pack it pins); `runs/<case>/`
  holds a run's `run.replay` (`checkpoints` every 60th tick, ADR-REPL-3) and `case.json`;
  `packs/<sha256>.json` the pinned packs besides the bundled one. `tests/corpus/v<N>/`
  (N >= 1) is a frozen version of the same shape plus `manifest.sha256`: never edited (CI `frozen
  corpus unchanged`, `corpus::the_frozen_corpus_is_unchanged`); `v1/` is recorded by `cargo xtask
  bless --freeze v1`. `corpus.rs` checks it all (ADR-SIM-3). The `app` repo reads one file,
  `tests/corpus/current/runs/stage_skill/run.replay`; it is not renamed without the app's bump
  (`tests/corpus_contract.rs`). CI readers use `.github/scripts/corpus_paths.py`. An intended
  change: `cargo xtask bless` (it also regenerates `tests/golden/` and `tests/bench_pins.toml`);
  CI runs `cargo xtask bless --check`. `cargo insta test --features record --review` reviews one.
- `tests/savepoint_proof.rs`: for every run case (current and frozen `v1/`) a savepoint restored
  mid-run gives the uninterrupted run's hashes (ADR-SAVE-3); `savepoint_load` in `fuzz/` fuzzes it.

## Pins

A pinned format or hash is never re-pinned: a change is a new version.

- Savepoint: `tests/fixtures/savepoint/v<N>.sha256` per `SAVEPOINT_FORMAT` (now 2,
  `savepoint/file.rs`); `UPDATE_SAVEPOINT_PIN=1` refuses to overwrite an existing file. A
  format 1 file is refused as `Format`. The format 2 payload starts with a fixed 60-byte
  `PlayerStats` record (per slot `damage_dealt` u64, `kills` u32, `support` u64;
  `STATS_RECORD_LEN`, `ledger.rs`), unhashed and trusted like the replay prefix, but a nonzero
  record for a slot outside the party is refused (`decode_for`). Then the state bytes.
- `GOLDEN_HASH` (`tests/engine.rs`) and the extremes pin (`tests/extremes.rs`): a rules change is
  a new `sim_version`. Counts that may only fall: `tests/alloc.rs`, `MEASURED_MOST`
  (`tests/command_cap.rs`), `tests/bench_pins.toml`. `tests/pick_pins.rs` was re-pinned once.
- The frozen `tests/corpus/v<N>/` and `SIM_V1_SOURCE` (`src/sim/frozen_tests.rs`): a constant
  that moves says why in the same commit. `tests/corpus_keeps.rs`: a re-blessed `current/` keeps
  each case's ticks, outcome, rewards and Stardust; its table is hand-written, never blessed.
