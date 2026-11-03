---
paths:
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

Sources: ADR-REPL-5, ADR-REPL-6, ADR-BRDG-3, ADR-CONT-6, ADR-SVC-7, ADR-SIM-3, ADR-SAVE-4.

## What is where

- `replay.rs`, `run_replay.rs`: the `.replay` file, record and verify (ADR-REPL-6, ADR-BRDG-3).
  A run's replay holds the setup, the content SHA-256 and the inputs (picks, `choose` and
  `spend`, are steps of the step log); it is verified from the file and its run stats (ADR-REPL-5).
- The step log, the range reader and the recorder (`step_log.rs`, `RunRecorder`):
  `step-log.md`.
- `share.rs`, `base64.rs`: share codes; `recording.rs`: the `record` feature's JSON Lines.
  `content_hash.rs`: SHA-256 of the two file hashes (ADR-CONT-6).
  `parsed_stage_hash.rs`: the blocklist's hash (ADR-SVC-7 point 4), pinned in `tests/parsed_stage_hash.rs`.
- `stage-verify` (`src/bin/stage_verify.rs`): re-simulates a replay, checks the clear and hash.

## Tests

- `tests/integration.rs` declares the other files: `cargo test --test integration <file>::`.
  `tests/offer_mode.rs`: the two offer modes (setup reading, the timer, the recorder, a savepoint
  with an offer open); `tests/offer_rating.rs`: `Engine::offer_ratings`.
- `tests/corpus/current/` is the blessed tree: `stages/<case>/` holds `stage.json`, `run.replay`,
  `hashes.bin` (the state hash after every tick) and `case.json` (the pack it pins); `runs/<case>/`
  holds a run's `run.replay` (`checkpoints` every 60th tick, ADR-REPL-5) and `case.json`;
  `parties/<case>/` holds a party run's `run.replay` and `case.json` only (no `hashes.bin`, no
  `stage.json`): `two_time_limit` (2 players), `three_time_limit`, `three_tutorial` (3),
  `join_two` (a solo run that takes slot 1 at tick 600), `leave_hole` (3 players; slot 1 leaves
  at tick 900, slot 2 leaves, then rejoins), `timed_one` (1 player) and `timed_two` (2) in a
  `timed` run: slot 0 answers an offer 100 ticks after it opens, the timer closes the rest; bless
  writes them, `tests/party_run.rs` pins the table, and the solo guards (`runs/` only) skip them.
  `packs/<sha256>.json` the pinned packs besides the bundled one. `tests/corpus/v<N>/`
  (N >= 1) is a frozen version of the same shape (no `parties/`: `sim_version` 1 has one player)
  plus `manifest.sha256`: never edited (CI `frozen
  corpus unchanged`, `corpus::the_frozen_corpus_is_unchanged`); `v1/` is recorded by `cargo xtask
  bless --freeze v1`. `corpus.rs` checks it all (ADR-SIM-3). The `app` repo reads one file,
  `tests/corpus/current/runs/stage_skill/run.replay`; it is not renamed without the app's bump
  (`tests/corpus_contract.rs`). CI readers use `.github/scripts/corpus_paths.py`. An intended
  change: `cargo xtask bless` (it also regenerates `tests/golden/` and `tests/bench_pins.toml`);
  CI runs `cargo xtask bless --check`. `cargo insta test --features record --review` reviews one.
- `tests/savepoint_proof.rs`: for every solo run case (current and frozen `v1/`) a savepoint
  restored mid-run gives the uninterrupted run's hashes (ADR-SAVE-4); the party round trips are
  in `tests/party_run.rs`. `savepoint_load` in `fuzz/` fuzzes it, with a party run among its seeds.

## Pins

A pinned format or hash is never re-pinned: a change is a new version.

- Savepoint: `tests/fixtures/savepoint/v<N>.sha256` per `SAVEPOINT_FORMAT` (now 3,
  `savepoint/file.rs`; `v1`, `v2` and `v3` are pinned and `tests/savepoint.rs` wants one per
  older format); `UPDATE_SAVEPOINT_PIN=1` refuses to overwrite an existing file. A file of format
  1 or 2 is refused as `Format`. The payload starts with a fixed 60-byte `PlayerStats` record
  (per slot `damage_dealt` u64, `kills` u32, `support` u64; `STATS_RECORD_LEN`, `ledger.rs`),
  unhashed and trusted like the replay prefix, and a slot outside the party may hold a leaver's
  totals (`decode_for` ignores the mask). Then the state bytes (`state-hash.md`: the presence
  byte, the generations, a block per present player, a `timed` run's deadlines). `restore` builds
  its fresh engine as the party the prefix log ends on, after its last join or leave
  (`presence_at_end`); a size mismatch is `Damaged` or `PrefixMismatch`.
- `GOLDEN_HASH` (`tests/engine.rs`) and the extremes pin (`tests/extremes.rs`): a rules change is
  a new `sim_version`. Counts that may only fall: `tests/alloc.rs`, `MEASURED_MOST`
  (`tests/command_cap.rs`), `tests/bench_pins.toml`. `tests/pick_pins.rs` was re-pinned once.
- The frozen `tests/corpus/v<N>/` and `SIM_V1_SOURCE` (`src/sim/frozen_tests.rs`): a constant
  that moves says why in the same commit. `tests/corpus_keeps.rs`: a re-blessed `current/` keeps
  each case's ticks, outcome, rewards and Stardust; its table is hand-written, never blessed.
