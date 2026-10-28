---
paths:
  - "src/replay.rs"
  - "src/run_replay.rs"
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

- `replay.rs`, `run_replay.rs`: the `.replay` file and its packed inputs, record and verify
  (ADR-REPL-4, ADR-BRDG-3). A run's replay holds the setup, the content SHA-256, the inputs and
  one packed stream of commands between ticks (`choose`, `spend`); it is verified from the file
  and its run stats (ADR-REPL-3).
- `share.rs`, `base64.rs`: share codes (`miniz_oxide`, outside the simulation path).
  `recording.rs`: the `record` feature's JSON Lines.
- `content_hash.rs`: the stage content hash, SHA-256 of the two file hashes (ADR-SVC-6); `sha2`
  lives outside the simulation path.
- `parsed_stage_hash.rs`: the hash the blocklist uses, SHA-256 of a tag and the parsed stage's
  compact JSON without `id` (ADR-SVC-6); its bytes are pinned in `tests/parsed_stage_hash.rs`.
- `stage-verify` (`src/bin/stage_verify.rs`): re-simulates a replay, checks the clear and the
  final hash, and prints both stage hashes (ADR-REPL-4, ADR-SVC-6).

## Tests

- `tests/integration.rs` declares every other file as a module (`autotests = false`), so
  `cargo test --test integration <file>::` runs one file.
- `tests/corpus/`: `v0/<case>/` holds `stage.json`, `run.replay` and `hashes.bin`;
  `runs/v0/<case>/` holds a run's `run.replay` and `hashes.bin` (ADR-REPL-3); `corpus.rs`
  checks it (ADR-SIM-3). `tests/corpus/runs/v0/stage_skill/run.replay` is read by the `app` repo from
  Cargo's checkout of core: it is not renamed or moved without the app's bump
  (`tests/corpus_contract.rs` fails on it). An intended change: `cargo xtask bless`
  (it sets `UPDATE_CORPUS=1` itself); CI runs `cargo xtask bless --check`.
- `golden.rs` keeps the recordings as `insta` snapshots in `tests/golden/`
  (`cargo xtask bless` regenerates them too, and `tests/bench_pins.toml`;
  `cargo insta test --features record --review` to review one by hand).
