---
paths:
  - "src/sim/**"
  - "src/sim_v1/**"
  - "src/engine/**"
  - "src/inline.rs"
  - "tests/alloc.rs"
  - "src/trig.rs"
  - "src/presets.rs"
  - "src/hash.rs"
---

# Simulation code

Sources: ADR-SIM-15, ADR-SIM-17, ADR-SIM-18, ADR-SIM-19, ADR-REPL-13, ADR-REPL-16, ADR-CONT-13. See `determinism.md`;
outcome changes follow the `schema-change` skill.

## What is where

- `engine/`: the engine (`Engine`): `mod.rs` starts (`start_party` and `start_party_with` take 1
  to 3 players, `start` is 1; another size is `Error::Invalid` at `$.party`, and `sim_version`
  1 takes only 1, `run.rs`), steps (`step(&[Input])`: one input per present player, any other
  length panics), chooses, spends and hashes; `snapshot.rs` builds the snapshot, `places` (ids,
  kinds, positions per tick) and `tally` (the HUD tallies, `Counted` in `ledger.rs`, outside
  the hashed state).
- `sim/`: the live rules, run for the newest `sim_version` (2, open while `DEVELOPMENT` is true,
  `src/version.rs`; ADR-REPL-13 points 2 and 3): an outcome change is a gate on the version, a full
  copy only for a rewrite (`determinism.md`). `state.rs` is the run's `State`; `world.rs` the
  run's definitions, resolved once and read-only (`DefIndex`, `AgentDef`, `AttackDef`).
  `schedule.rs` holds `TICK`, the one tick order (`tick_doc_lists_the_schedule` keeps the `sim`
  module doc's table equal to it): a step is added or moved there. `sim/` holds agents and the
  enemies' push-apart, attacks and hit boxes, skills, the main character's movement, facing and
  evade, the weapon's combos (ADR-SIM-18), aim assist, boss phases and parts, experience and
  levels, the runes (`runes.rs`), the falling stars and their offers (`stars.rs`, ADR-REPL-16)
  and the expertise offers (`expertise.rs`).
- `Players::party` (`state.rs`) copies slot 0's post-loadout player into each slot. Slots 1 and
  2 are passive in the rules: `Ctx.me` is `PlayerIdx::SOLO` (`schedule.rs`); their inputs are
  logged and read back, the world does not read them. They act through offers and picks.
- `sim_v1/`: the frozen rules of `sim_version` 1. It owns its `World`, `State`, snapshot view
  and events; a change to its behaviour is refused. It is edited only to keep compiling against
  a live type it still imports (fixed point, `id`, `rune`, `rewards`, `rng`,
  `run_content`, `snapshot` views, `stage`, `weapon`, `validate`; `grep -rh "^use crate::"
  src/sim_v1`): `SIM_V1_SOURCE` (`src/sim/frozen_tests.rs`) moves once, with the reason in the
  same commit, and `tests/corpus/v1/` is the guard. Its hash keeps the single-player layout,
  no presence byte (`src/sim_v1/hash_layout_tests.rs`).
- Events: `sim_v1/event.rs` holds the 43-variant `DomainEvent`, `HitKind` and `HitSource` as
  version 1 emitted them. The one adapter is `engine/v1_events.rs`:
  `From<sim_v1::event::DomainEvent> for DomainEvent`, an exhaustive `match` with no `_` arm,
  player fields `None`; `Run::V1` (`engine/run.rs`) drains frozen events through it. A live
  event change edits the adapter, never `sim_v1` (`sim_v1_imports_nothing_from_the_live_event`).
- `trig.rs`: the sine table. `presets.rs`: takes an attack preset (`content/attack_presets.json`,
  ADR-CONT-13 point 6) apart into attack phases. `inline.rs`: `InlineVec<T, N>`, a bounded list, no heap.
  `hash.rs`: the FNV-1a 64 hasher.

## The state hash

`src/sim/state_hash.rs` (FNV-1a 64, `hash.rs`) writes, in this order: the shared fields (`tick`,
the four streams, `outcome`, `boss`, `companions`, `enemies`, `tasks`, `runs`, `ranged_cooling`,
`orbs`, `zones`, `spawned`, `next_id`, `next_orb_group`, the star field's shared part, `heal_pickups`,
the tuning percentages, `horde`, `crit_seed`, `echoes`), then the presence byte `players.mask()`
(bit `i` is slot `i`, the step log's byte 0), then each present player's block in slot order
(`hash_player`: `PlayerState`, `progress`, offer generator, `offers`, `expertise`, `stardust`,
`regen`). An absent slot writes nothing; a hole mask (0b101) hashes and saves, but the recorder
and verifier refuse a hole. Each impl destructures its type without `..`; a skipped field is `_`
with `// not hashed: <why>` (`engine/hash_coverage_tests.rs`). The savepoint mirrors it:
`decode_over` (`src/sim/state_codec.rs`) refuses a mask without slot 0, past the last slot, or
not the fresh engine's.

## Rules

- A steady tick allocates nothing (T1b): `sim/scratch.rs`'s `Scratch` buffers keep their memory
  from tick to tick, are not hashed or cloned, and hold nothing the rules read between ticks.
  `tests/alloc.rs` asserts 0 allocations and frees for a steady tick and the bench tick; a new
  per-tick `Vec` goes into `Scratch`, a bounded per-entity list into `InlineVec`.
- The caps (enemies, zones, orbs), push-apart and collision cost are set by ADR-SIM-19; see `benches-fuzz.md`.
- A new constant in `sim/` is code only when changing it needs code review: a cap, the grid, the
  push-apart, the view or a unit (ADR-CONT-13). A designer-facing number is a `rules.json` key
  read from `Tuning`. Code to content is a pack-hash change alone; content to code is a rules
  change, gated on `sim_version` if an outcome moves (ADR-REPL-13 point 2). Never put a cap or a feel
  number in `limits.rs` and `rules.json` both.
