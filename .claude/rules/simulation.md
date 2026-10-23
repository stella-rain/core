---
paths:
  - "src/rules_v0/**"
  - "src/engine*"
  - "src/engine/**"
  - "src/trig.rs"
  - "src/presets.rs"
  - "src/hash.rs"
  - "src/spike.rs"
---

# Simulation code

Sources: ADR-SIM-1, ADR-SIM-3, ADR-SIM-4, ADR-SIM-5, ADR-REPL-3. The determinism rules are in
`determinism.md`; changes to outcomes follow the `schema-change` skill.

## What is where

- The engine (`Engine`): step, choose, spend, events, snapshot, hash.
- `rules_v0/`: the rules of `sim_version` 0. Each part says what it does not do yet. It holds
  agents and the enemies' push-apart, attacks and their hit boxes, skills, the main character's
  movement, facing and evade, the weapon's combos made by hand with their charged steps and
  smash effects (ADR-SIM-4), aim assist's target pick and auto mode, boss phases and parts,
  experience, levels and stat points, the runes' effects (`runes.rs`), the falling stars and
  their offers (`stars.rs`, ADR-REPL-3) and the expertise offers (`expertise.rs`).
- `trig.rs`: the interpolated sine table. `presets.rs`: attack and weapon presets, written in
  the stage's own JSON.
- `hash.rs`: the FNV-1a 64 state hash. `spike.rs`: the ADR-PLAT-1 spike simulation, not a
  frozen `rules_vN`.

## Rules

- The horde caps, the push-apart and the collision cost are ADR-SIM-5's; the benches in
  `benches-fuzz.md` count them.
- `tests/spike.rs` and `tests/engine.rs` pin golden hashes every platform must match.
- `tests/properties.rs` checks `proptest` invariants on the corpus stages
  (`PROPTEST_CASES=2000` for a longer run).
