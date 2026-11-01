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

Sources: ADR-SIM-1, ADR-SIM-3, ADR-SIM-4, ADR-SIM-7, ADR-REPL-3, ADR-CONT-10. The determinism rules are in
`determinism.md`; changes to outcomes follow the `schema-change` skill.

## What is where

- `engine/`: the engine (`Engine`): `mod.rs` steps, chooses, spends and hashes; `snapshot.rs`
  builds the snapshot, `places`, the light read of ids, kinds and positions for the per-tick
  frame, and `tally`, the read-only view of the run's HUD tallies (kills, rewards taken, named
  stars, constellation tiers; `Counted` in `ledger.rs`, outside the hashed state); its tests sit
  beside them (`horde_tests.rs`, `move_index_tests.rs`).
- `sim/`: the live rules, run for the newest `sim_version` (ADR-REPL-7, ADR-REPL-9): an outcome
  change is a gate on the version, a full copy only for a rewrite (`determinism.md`).
- `sim_v1/`: the frozen rules of `sim_version` 1, a copy of `sim` that owns its `World`, `State`
  and snapshot view. A change to its behaviour is refused; it is edited only to keep compiling
  against a shared type, and the frozen corpus `tests/corpus/v1/` is the guard. In both:
  `state.rs` is the run's `State`; `world.rs` the
  run's definitions, resolved once and read-only (`DefIndex`, `AgentDef`, `AttackDef`).
  `schedule.rs` holds `TICK`, the one tick order: a step is added or moved there, and
  `tick_doc_lists_the_schedule` keeps the table in the `sim` module doc equal to it.
  Each part says what it does not do yet. It holds
  agents and the enemies' push-apart, attacks and their hit boxes, skills, the main character's
  movement, facing and evade, the weapon's combos made by hand with their charged steps and
  smash effects (ADR-SIM-4), aim assist's target pick and auto mode, boss phases and parts,
  experience, levels and stat points, the runes' effects (`runes.rs`), the falling stars and
  their offers (`stars.rs`, ADR-REPL-3) and the expertise offers (`expertise.rs`).
- `trig.rs`: the interpolated sine table. `presets.rs`: takes an attack preset apart into the
  stage's own attack phases; the presets themselves are content, `content/attack_presets.json`
  read by `attack_presets.rs` from the run's pack (ADR-CONT-8).
- `inline.rs`: `InlineVec<T, N>`, a bounded list with no heap allocation, for per-enemy lists
  and attack frames.
- `hash.rs`: the FNV-1a 64 state hash.

## Rules

- A steady tick allocates nothing (T1b): `sim/scratch.rs`'s `Scratch` buffers keep their
  memory from tick to tick, are not hashed or cloned, and hold nothing the rules read between
  ticks. `tests/alloc.rs` counts allocations and frees (a steady tick and the bench tick
  assert 0 of both); a new per-tick `Vec` goes into `Scratch`, a bounded per-entity list into
  `InlineVec`.

- The caps (enemies, zones, orbs), the push-apart and the collision cost are ADR-SIM-7's; the benches in
  `benches-fuzz.md` count them.
- A new constant in `sim/` is code only when changing it needs code review: a cap, the grid, the
  push-apart, the view or a unit (ADR-CONT-10). A designer-facing number is a `rules.json` key read
  from `Tuning`. Code to content is a pack-hash change alone; content to code is a rules change,
  gated on `sim_version` if an outcome moves (ADR-REPL-7). Do not add a cap or a feel number to
  `limits.rs` and `rules.json` both.
- `tests/engine.rs` pins the golden hash every platform must match
  (`the_golden_hash_is_the_same_on_every_platform`).
- `tests/properties.rs` checks `proptest` invariants on the corpus stages
  (`PROPTEST_CASES=2000` for a longer run).
