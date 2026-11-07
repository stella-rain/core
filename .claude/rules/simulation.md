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

Sources: ADR-SIM-1, ADR-SIM-3, ADR-SIM-4, ADR-SIM-5, ADR-SIM-7, ADR-REPL-2, ADR-REPL-5, ADR-CONT-3. See `determinism.md`;
outcome changes follow the `schema-change` skill.

## What is where

- `engine/`: the engine (`Engine`): `mod.rs` starts (`start_party` and `start_party_with` take 1
  to 3 players, `start` is 1; another size is `Error::Invalid` at `$.party`, and `sim_version`
  1 takes only 1, `run.rs`), steps (`step(&[Input; MAX_PLAYERS])`: one entry per slot, an absent
  one ignored), joins and leaves a slot (`join`, `leave`, `present`, `PresenceError`), chooses,
  spends and hashes. An open offer stops the world only in a `stop` run; a `timed` run takes the
  picks first, then ticks (`offer_mode`, `RunSetup`; `content.md`); `snapshot.rs` builds the
  snapshot, `places` (ids, kinds, positions per tick) and `tally` (the HUD tallies, `Counted` in
  `ledger.rs`, outside the hashed state).
- `sim/`: the live rules, run for the newest `sim_version` (2, open while `DEVELOPMENT` is true,
  `src/version.rs`; ADR-REPL-2 points 2 and 3): an outcome change is a gate on the version, a full
  copy only for a rewrite (`determinism.md`). `state.rs` is the run's `State`; `world.rs` the
  run's definitions, resolved once and read-only (`DefIndex`, `AgentDef`, `AttackDef`).
  `schedule.rs` holds `TICK`, the one tick order (`tick_doc_lists_the_schedule` keeps the `sim`
  module doc's table equal to it): a step is added or moved there. `sim/` holds agents and the
  enemies' push-apart, attacks and hit boxes, skills, the main character's movement, facing and
  evade, the weapon's combos (ADR-SIM-4), aim assist, boss phases and parts, experience and
  levels, the runes (`runes.rs`), the falling stars and their offers (`stars.rs`, ADR-REPL-5)
  and the expertise offers (`expertise.rs`). A `timed` run keeps a deadline per slot
  (`sync_deadline`, `pick_due`). `offer_rating.rs` rates the candidates of an open star or boss
  offer: `Engine::offer_ratings(player)` returns an `OfferRating` each (categories, `dps_bp`,
  `set_tier`, the stat changes); read-only, nothing hashed, saved or recorded. Equipment mode
  (`World.equipment_mode`): `build.rs`, `slots.rs` (the swap rules: hit-point fraction, entry
  cooldown, the savepoint check), `starlight.rs`, `drops.rs`, `equipment.rs`, `loadout.rs`:
  `equipment.md`; `expertise.rs`: `weapon-tree.md`; a party: `party-equipment.md`.
- `Players::party` (`state.rs`) copies slot 0's post-loadout player into each slot, a member
  from `Career::member` (slot 0's start career; `party-equipment.md`). Every present
  slot acts in slot order (`player_update` in `schedule.rs`; down, revive, tether and rewards:
  `party-play.md`). A player's number is `generation << 24 | slot`; a rejoin gets a new generation (`state-hash.md`).
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
  ADR-CONT-3 point 6) apart into attack phases. `inline.rs`: `InlineVec<T, N>`, a bounded list, no heap.
  `hash.rs`: the FNV-1a 64 hasher.
- The state hash, the savepoint's mirror of it and the entity numbers: `state-hash.md`.

## Rules

- A steady tick allocates nothing (T1b): `sim/scratch.rs`'s `Scratch` buffers keep their memory
  from tick to tick, are not hashed or cloned, and hold nothing the rules read between ticks.
  `tests/alloc.rs` asserts 0 allocations and frees for a steady tick and the bench tick; a new
  per-tick `Vec` goes into `Scratch`, a bounded per-entity list into `InlineVec`.
- The caps (enemies, zones, orbs), push-apart and collision cost are set by ADR-SIM-5; see `benches-fuzz.md`.
- A new constant in `sim/` is code only when changing it needs code review: a cap, the grid, the
  push-apart, the view or a unit (ADR-CONT-3). A designer-facing number is a `rules.json` key
  read from `Tuning`. Code to content is a pack-hash change alone; content to code is a rules
  change, gated on `sim_version` if an outcome moves (ADR-REPL-2 point 2). Never put a cap or a feel
  number in `limits.rs` and `rules.json` both.
- A run end made inside the hit pass is deferred: `end_run` and `end_runs_of` only record it in
  `Scratch::ends_pending` while `ends_pinned` is set; `hit_boxes::land` lands every swing, then
  `apply_ends` ends the runs in order and rebuilds the move index once. A caller in the pass
  uses those two, never `retain_runs`, which debug-asserts it is not called there.
