---
paths:
  - "src/engine/mod.rs"
  - "src/engine/presence_tests.rs"
  - "src/savepoint/file.rs"
  - "tests/party_run.rs"
  - "tests/party_join.rs"
  - "tests/corpus/current/parties/**"
  - "fuzz/src/lib.rs"
---

# Parties, joining and leaving

Sources: ADR-SIM-7, ADR-SIM-8, ADR-REPL-5, ADR-SAVE-6. The log bytes and the recorder:
`step-log.md`; the hash and `id_gap`: `simulation.md`, `determinism.md`; the party in play:
`party-play.md`.

## Parties

- Slots 1 and 2 move, attack, orbit and regenerate like slot 0 (`party-play.md`: acting, down,
  tether, rewards). `Engine::join(slot)` makes the joiner a `Career::member`: the run's starting player at
  slot 0's place, at level 1 (equipment: `party-equipment.md`), with no event; `leave(slot)` keeps the leaver's in-flight attacks and closes its open
  offer. Slot 0 never leaves, so a hole (`0b101`) arises only by LEAVE. A `sim_version` 1 run
  refuses both (`PresenceError::SoloRules`).
- In a `timed` run every present slot has its own deadline (`State.offer_timer.due`, one
  `Option<u32>` per slot): the timer takes the first option of an offer nobody answers, with a
  manual pick's events (`pick_due`), and a leaver's deadline is cleared with its offer. A `stop`
  run waits for every slot. `timed_two` (slot 1 never answers) is the case that shows it.
- Only a hole at tick 0 is refused by the recorder and verifier; the step log reaches a hole by
  LEAVE. The app never resumes a party run: `resume_*` for a party serve tests, replay and
  "continue alone" (ADR-SAVE-6).

## Tests

- `tests/party_run.rs`: the start, the recorder and its refusals, resume with and without an
  open offer, `replay_at`, savepoint round trips and size mismatches, the solo views, the solo
  run cases re-recorded byte-equal, and the party cases (`tests/corpus/current/parties/`).
- `tests/party_join.rs` (declared in `tests/integration.rs`): join and leave in the engine and
  the recorder, resume, `replay_at`, savepoints at every cut, a savepoint of another mask refused.
- `tests/schema2_range.rs`, `tests/schema2_write.rs`; fuzz targets `step_log_range` and
  `replay_load`. Their builders `recorder_until` and `savepoint_recorder` (`fuzz/src/lib.rs`)
  re-record a verified run: the party from the `present` byte, then its joins and leaves.
