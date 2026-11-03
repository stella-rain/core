---
paths:
  - "src/sim/state_hash.rs"
  - "src/sim/state_codec.rs"
  - "src/sim/hash_fields_tests.rs"
  - "src/sim/hash_layout_tests.rs"
  - "src/sim/codec_fields_tests.rs"
  - "src/engine/hash_coverage_tests.rs"
  - "src/hash.rs"
  - "src/id.rs"
  - "src/sim/entity.rs"
---

# The state hash and entity numbers

Sources: ADR-SIM-3, ADR-SIM-7, ADR-SIM-8, ADR-REPL-5, ADR-SAVE-4. Also `replay.md`.

## The state hash

`src/sim/state_hash.rs` (FNV-1a 64, `hash.rs`) writes, in this order:

- the shared fields (`tick`, the four streams, `outcome`, `boss`, `companions`, `enemies`, `tasks`,
  `runs`, `ranged_cooling`, `orbs`, `zones`, `spawned`, `next_id`, `next_orb_group`, the star
  field's shared part, `heal_pickups`, the tuning percentages, `horde`, `crit_seed`, `echoes`);
- the presence byte `players.mask()` (bit `i` is slot `i`, the step log's byte 0), then three
  bytes: the generation of every slot, present or not (an absent slot's next occupant depends
  on it);
- each present player's block in slot order (`hash_player`: `PlayerState`, `progress`, offer
  generator, `offers`, `expertise`, `stardust`, `regen`). An absent slot writes no block;
- in a `timed` run only, the tail: `offer_timer.due` of each present slot. A `stop` run writes
  nothing there, so its hash is what it was (ADR-SIM-7 point 4). The encoding is prefix-free
  within a mode; the mode is fixed for a run and states of the two are never compared.

A hole mask (0b101) hashes and saves; the step log reaches one by LEAVE (only a hole at tick 0 is
refused). Each impl destructures its type without `..`; a skipped field is `_` with
`// not hashed: <why>` (`engine/hash_coverage_tests.rs`).

The savepoint mirrors it (`src/sim/state_codec.rs`): the same bytes in the same order.
`decode_over` refuses a mask without slot 0, past the last slot, or not the fresh engine's; a
`next_id` below `MAX_PLAYERS` or at or past `MAX_ENTITY_NUMBER`; and in a `timed` run a deadline
that disagrees with its slot's offer, or lies more than `MAX_OFFER_TIMER_TICKS` past the tick.

## Entity numbers

- A player's number is `generation << 24 | slot` (`src/id.rs`, `GENERATION_SHIFT`; `Players::id`
  in `sim/state.rs`). Slots are 0 to 2, present or not: a slot's first occupant has the slot as its
  number. The boss, the companions and the enemies are numbered from 3 and
  stay below `MAX_ENTITY_NUMBER` (`1 << 24`), so `player_of` (a number's slot and generation,
  `None` for any other entity) never reads one as a player.
- A leave adds one to the slot's generation: nothing the leaver owned binds to the next
  occupant of the slot, a rejoin. A slot at generation 255 refuses `Engine::leave` as
  `PresenceError::Exhausted`.
- The roll key (`determinism.md`): `to_compact(id, id_gap)` maps a player's number of any
  generation to its slot and every other entity `id_gap` lower; `id_gap = MAX_PLAYERS - present`
  (`id::id_gap`; `State::id_gap`, `sim/entity.rs`, counts the set bits of the mask). It is a
  rule of the simulation, not an encoding: hash and savepoint write numbers as they are.
