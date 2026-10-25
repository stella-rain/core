---
paths:
  - "src/rules_v0/**"
  - "src/engine/**"
  - "src/inline.rs"
  - "tests/alloc.rs"
  - "src/trig.rs"
  - "src/presets.rs"
  - "src/hash.rs"
  - "src/spike.rs"
---

# Simulation code

Sources: ADR-SIM-1, ADR-SIM-3, ADR-SIM-4, ADR-SIM-7, ADR-REPL-3. The determinism rules are in
`determinism.md`; changes to outcomes follow the `schema-change` skill.

## What is where

- `engine/`: the engine (`Engine`): `mod.rs` steps, chooses, spends and hashes; `snapshot.rs`
  builds the snapshot; its tests sit beside them (`horde_tests.rs`, `move_index_tests.rs`).
- `rules_v0/`: the rules of `sim_version` 0. `state.rs` is the run's `State`; `world.rs` the
  run's definitions, resolved once and read-only (`DefIndex`, `AgentDef`, `AttackDef`).
  `schedule.rs` holds `TICK`, the one tick order: a step is added or moved there, and
  `tick_doc_lists_the_schedule` keeps the table in the `rules_v0` module doc equal to it.
  Each part says what it does not do yet. It holds
  agents and the enemies' push-apart, attacks and their hit boxes, skills, the main character's
  movement, facing and evade, the weapon's combos made by hand with their charged steps and
  smash effects (ADR-SIM-4), aim assist's target pick and auto mode, boss phases and parts,
  experience, levels and stat points, the runes' effects (`runes.rs`), the falling stars and
  their offers (`stars.rs`, ADR-REPL-3) and the expertise offers (`expertise.rs`).
- `trig.rs`: the interpolated sine table. `presets.rs`: attack and weapon presets, written in
  the stage's own JSON.
- `inline.rs`: `InlineVec<T, N>`, a bounded list with no heap allocation, for per-enemy lists
  and attack frames.
- `hash.rs`: the FNV-1a 64 state hash. `spike.rs`: the ADR-PLAT-1 spike simulation, not a
  frozen `rules_vN`.

## Rules

- A steady tick allocates nothing (T1b): `rules_v0/scratch.rs`'s `Scratch` buffers keep their
  memory from tick to tick, are not hashed or cloned, and hold nothing the rules read between
  ticks. `tests/alloc.rs` counts allocations; a new per-tick `Vec` goes into `Scratch`, a
  bounded per-entity list into `InlineVec`.

- The caps (enemies, zones, orbs), the push-apart and the collision cost are ADR-SIM-7's; the benches in
  `benches-fuzz.md` count them.
- `tests/spike.rs` and `tests/engine.rs` pin golden hashes every platform must match.
- `tests/properties.rs` checks `proptest` invariants on the corpus stages
  (`PROPTEST_CASES=2000` for a longer run).
