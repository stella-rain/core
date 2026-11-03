---
paths:
  - "src/step_log.rs"
  - "src/run_replay.rs"
  - "src/recording.rs"
  - "tests/party_run.rs"
  - "tests/schema2_range.rs"
  - "tests/schema2_write.rs"
  - "tests/run_replay.rs"
  - "tests/corpus/current/parties/**"
---

# The step log and the recorder

Sources: ADR-REPL-12, ADR-REPL-16, ADR-SAVE-14, ADR-SAVE-16. File formats, hashes and the corpus: `replay.md`.

## What is where

- `step_log.rs`: the in-memory model of a run's inputs for both schemas. Schema 2 `inputs` bytes:
  a `present` byte (bit `i` is slot `i`, at least one set, bits 3-7 zero), then records.
  `0x00` TICKS: a varint `run` >= 1, then a 3-byte block (`flags`, `move_x`, `move_y`) per present
  slot, for `run` ticks; two TICKS in a row never hold equal blocks. `0x01` PICKS: one step that
  does not advance the world, one byte per present slot (`0x00` none, `0x01` and a varint index),
  at least one. `0x02` JOIN and `0x03` LEAVE are reserved and refused. Runs sum to `ticks`; at
  most `MAX_COMMANDS` (1000, `run_replay.rs`) PICKS. Bit 6 of `flags` is `interact`; an axis of
  -128 is written -127 (`block_of`, `replay.rs`). `SCHEMA_VERSION` 2 is written, 1 and 2 read
  (`version.rs`); schema 1 migrates in memory (`from_schema1`).
- The party: `StepLog::party_of(present)` is the one validator of a `present` byte (`0b001`,
  `0b011`, `0b111` are parties of 1 to 3; a hole or a bit past slot 2 is `None`, refused).
  `RunReplay::party_size()` reads the first byte only (1 for schema 1). `resimulate` starts the
  party the log names and refuses a hole as `BadInputs("the party does not match the run")`.
- Solo views: `RunReplay::unpack_inputs` and `unpack_commands` refuse a party as
  `BadInputs("a party has no solo view")`; the recorder does not use them.
- `read_step_range(bytes, ticks, max_ticks, a, b)` and `StepLog::range`: ticks 0-based, range
  half-open. A TICKS run overlapping it is returned once, clipped, never expanded (memory is
  bounded by the records in the range). A PICKS step after `p` ticks is in iff `a <= p < b`; a
  range with `b == ticks` also holds the picks after the last tick. The whole log is validated
  first; `a > b` and `b > ticks` are `BadInputs`, before the bytes are read.

## The recorder

- `RunRecorder` (`run_replay.rs`) keeps `present` and the step log's records, no solo lists.
  `start` is a party of 1 and writes the bytes it always wrote; `start_party`, `start_party_with`
  and `start_party_with_pack` take a size of 1 to 3 (another is `Setup`, from
  `Engine::start_party_with`, `$.party`).
- `step(&[Input])` takes one input per player; another length is `BadInputs("one input per
  player")` and records nothing. Each input is stored as its block bytes read back, so an axis of
  -128 then -127 is one run. `choose(player, i)` is one pick; `picks(&[Option<u32>])` is one step
  with several picks, at least one.
- `replay_at` cuts the log with `StepLog::prefix` and reads nothing from the engine.
  `resume_with_pack` and `resume_from_savepoint` keep the whole log; the savepoint restore builds
  the party the prefix names (`replay.md`, Pins).

## Parties

- Slots 1 and 2 are passive in the rules (`simulation.md`): their inputs are logged and read
  back. The recorder and verifier refuse a hole; joining and leaving do not exist yet (JOIN and
  LEAVE above). The app never resumes a party run: `resume_*` for a party serve tests, replay
  and "continue alone" (ADR-SAVE-16).

## Tests

- `tests/party_run.rs`: the start, the recorder and its refusals, resume with and without an
  open offer, `replay_at`, savepoint round trips and size mismatches, the solo views, the solo
  run cases re-recorded byte-equal, and the party cases (`tests/corpus/current/parties/`).
- `tests/schema2_range.rs`, `tests/schema2_write.rs`; fuzz targets `step_log_range` and
  `replay_load`. Their builders `recorder_until` and `savepoint_recorder` (`fuzz/src/lib.rs`)
  re-record a verified run from its own step range, the party from the `present` byte.
