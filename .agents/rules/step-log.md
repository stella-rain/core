---
globs:
  - "src/step_log*"
  - "src/step_log/**"
  - "src/run_replay*"
  - "src/run_replay/**"
  - "src/recording.rs"
  - "tests/schema2_range.rs"
  - "tests/schema2_write.rs"
  - "tests/run_replay.rs"
  - "tests/common/run_steps.rs"
---

# The step log and the recorder

Sources: ADR-REPL-1, ADR-REPL-5, ADR-SAVE-4, ADR-SAVE-6, ADR-SIM-7, ADR-SIM-8. File formats,
hashes and the corpus: `replay.md`; join, leave and the party tests: `party.md`.

## What is where

- `step_log.rs`: the in-memory model of a run's inputs for both schemas. Schema 2 `inputs` bytes: a `present` byte (bit `i` is slot
  `i`, at least one set, bits 3-7 zero), then records. `0x00` TICKS: a varint `run` >= 1, then a 3-byte block (`flags`, `move_x`,
  `move_y`) per present slot, for `run` ticks; two TICKS in a row never hold equal blocks (a JOIN or LEAVE between them resets the
  check; so does an ACTIONS step). `0x01` PICKS is reserved, refused at schema 2 (`BadInputs("a PICKS record is reserved at schema
  2")`); a schema 1 replay's picks migrate into `Record::Picks` in memory, and `encode` refuses a log that holds one. `0x02 <slot>`
  JOIN and `0x03 <slot>` LEAVE change the mask between two records (before the first, after the last); the records after one hold a
  block per slot of the mask in force. Refused: slot 0 leaving, a join of a present slot, a leave of an absent one, a slot past 2, and
  more than `MAX_PRESENCE_CHANGES` (64). Runs sum to `ticks`; at most `MAX_COMMANDS` (1000, `run_replay.rs`) schema 1 picks. `0x04`
  ACTIONS: one step that does not advance the world, one entry per present slot in slot order: `0x00` (none) or a kind byte and varint
  `u32` operands, `0x01` Equip (item, slot), `0x02` Replace (item, slot), `0x03` Extract (item), `0x04` Craft (star), `0x05` TreePick
  (option); an unknown kind is `BadInputs("an action kind is unknown")`, a record of only `0x00` is refused ("a record has no
  action"). At most `MAX_ACTION_STEPS` (4096, `run_replay.rs`), counted apart from `MAX_COMMANDS`. Record bytes `0x05` to `0xFF` are
  unknown; a retired kind keeps its byte. Bit 6 of `flags` is `interact`; an axis of -128 is written -127 (`block_of`, `replay.rs`).
  `SCHEMA_VERSION` 2 is written, 1 and 2 read (`version.rs`); schema 1 migrates in memory (`from_schema1`).
- The party: `StepLog::party_of(present)` is the one validator of tick 0's `present` byte
  (`0b001`, `0b011`, `0b111` are parties of 1 to 3; a hole or a bit past slot 2 is `None`,
  refused): a hole arises only by LEAVE. The non-test reader of the party at the end is
  `RunReplay::presence_at_end` over `StepLog::presence_at_end`: the mask after the last JOIN or
  LEAVE, from one scan (the savepoint restore reads it). `resimulate` starts the party the log
  names, then calls `Engine::join` and `leave`; the engine's refusal is
  `RunReplayError::PresenceRefused { tick, error }`.
- Solo views: `RunReplay::unpack_inputs` and `unpack_commands` refuse a party, a log with a
  JOIN or LEAVE (`has_changes`) as `BadInputs("a party has no solo view")` and a log with an
  ACTIONS step as `BadInputs("a log with equipment actions has no solo view")`; the recorder does
  not use them. A test replays a run case through `common::run_steps` (`tests/common/run_steps.rs`:
  `RunSteps::of` over `RunReplay::step_range`, then `apply`, `apply_to` or `commands`).
- `read_step_range(bytes, ticks, max_ticks, a, b)` and `StepLog::range`: ticks 0-based, range
  half-open. A TICKS run overlapping it is returned once, clipped, never expanded (memory is
  bounded by the records in the range). An ACTIONS, JOIN or LEAVE (or a schema 1 PICKS) after `p`
  ticks is in iff `a <= p < b`; a range with `b == ticks` also holds those after the last tick. A step is a
  `RangeStep` (`Ticks`, `Picks` (only a schema 1 log fills it), `Actions`, `Join`, `Leave`), dense for the mask in force there;
  `StepRange.present` is tick 0's mask, `present_at_first` the mask after every change before
  tick `a`. The whole log is validated first; `a > b` and `b > ticks` are `BadInputs`, before the
  bytes are read.

## The recorder

- `RunRecorder` (`run_replay.rs`) keeps `present` and the step log's records, no solo lists.
  `start` is a party of 1 and writes the bytes it always wrote; `start_party`, `start_party_with`
  and `start_party_with_pack` take a size of 1 to 3 (another is `Setup`, from
  `Engine::start_party_with`, `$.party`).
- `step(&[Input; MAX_PLAYERS])` takes one entry per slot, present or not: an absent slot's is
  ignored and the log keeps one block per present slot. Each input is stored as its block bytes
  read back, so an axis of -128 then -127 is one run. An `Input.action` the engine accepts
  (`check_action`) is recorded first as one ACTIONS step and the tick runs on the input with it
  cleared, so a TICKS block never holds one.
  `actions(&[Option<EquipAction>; MAX_PLAYERS])` is one ACTIONS step (refused after the end, past
  `MAX_ACTION_STEPS` or when `check_action` refuses). `step` refuses an input while an offer is
  open (`RunReplayError::OfferOpen`; only a `sim_version` 1 run opens one), and `resimulate`
  refuses a TICKS run over an open offer.
- `join(slot)` and `leave(slot)` call the engine and write a JOIN or LEAVE at the tick the run
  is on; a refusal (`PresenceRefused`, the end of the run, the cap) records nothing.
- `replay_at` cuts the log with `StepLog::prefix` and reads nothing from the engine.
  `resume_with_pack` and `resume_from_savepoint` keep the whole log; the savepoint restore builds
  the party the prefix ends on, not the one it names (`replay.md`, Pins). `prefix` keeps a JOIN
  or LEAVE at world tick `p` iff `p` <= the cut.
