---
paths:
  - "src/sim/stun.rs"
  - "src/sim/hit_boxes.rs"
  - "src/sim/player.rs"
  - "src/sim/player_hits.rs"
  - "src/sim/collide.rs"
  - "src/sim/attacks/ranged.rs"
  - "src/sim/attacks/runs.rs"
  - "src/sim/state/foes.rs"
  - "src/sim/state/player_state.rs"
  - "src/threat_colours.rs"
  - "src/rules.rs"
  - "src/sim/{breaker,counter,gauge,magnet,perfect_evade,rally,repeat,spirit,tempo,trigger}_tests.rs"
  - "tests/{poise,stagger,stun,knockdown,counter_window,evade,rally,repeat,threat_colours}.rs"
  - "tests/ranged_*.rs"
  - "tests/expertise_*.rs"
  - "tests/common/mod.rs"
---

# Combat rules: how one is built and touched

Sources: ADR-SIM-3, ADR-SIM-9, ADR-SAVE-4, ADR-REPL-2. The weapon's sets and styles:
`weapon-tree.md`; the bytes: `state-hash.md`. This file holds how a rule is built, not its numbers.

## What a combat rule is

- A `rules.json` key read into `Tuning` (`src/rules.rs`; `COMBAT_RULE_KEYS` in
  `tests/common/mod.rs` lists them: poise damage, flinch, the hard-control list, `vulnerable`, the
  stagger numbers, knockdown, the counter stun, `ranged_grace_ticks`, the perfect evade,
  `orb_clear_radius`); a field of an enemy, boss or attack phase (`poise`, `stagger_max`,
  `counterable`, `poise_damage`, `advance`: `behaviour.rs`, `attack.rs`); or a style kind
  (`weapon-tree.md`).
- An absent key or a kind no style names is off: one bool test (`World::has_stagger`, `has_down`,
  `has_gauge`, ...), no state moved. `sim_version` 2 only (ADR-REPL-2).
- The bundled content ships them on: the combat keys, poise and `stagger_max` on every enemy and
  boss, the ranged grace, the perfect evade, the orb clear and the shipped styles' kinds; the
  `rally_*` keys are not shipped. A test that assumed a rule off starts from
  `dormant_rules_value()` and sets its own keys; a test that reads the styles uses
  `crate::before` (`weapon-tree.md`). A change that turns a rule on moves the corpus, seeds,
  scan constants, pins and generated sets in the same PR (ADR-SIM-3).

## State

- A rule's state is `PlayerCombat`, `FoeControl`, `Stagger` and `State.repeats`
  (`MAX_REPEATS_QUEUED`) and nothing else; a view or an event is derived (`state-hash.md`). A new
  field is a hash move and its own change: format bump, pins, corpus, bench (ADR-SAVE-4).
- Snapshot fields and events for the gauge, spirit, counter, tempo, trigger, rally, perfect evade
  and the stagger burst are derived: the app draws them, and a change to one is a schema change
  (`schema-change` skill).

## One path per effect

- Every stagger fill is `stun::fill` (the counter bonus's factor, then the breaker's, then the
  refill percent; a full gauge staggers); the breaker's burst, `player::stagger_burst`, runs
  right after with the filling hit's damage.
- A hard control is `stun::land_hard_control` (the n-th is shorter, then refused; knockdown reuses
  it). Flinch is `hit_boxes::flinch_by`.
- Every cancel (stun, flinch, knockdown, stagger, counter) ends runs through `cancel_runs_of` /
  `RunEnd::Cancel`; `counterable_run` reads a pending `Cancel` or `All` in `ends_pending`, so a
  new `RunEnd` variant keeps that read. A new super-armour condition goes in `player::super_armour`.
- A smash step spends a full gauge in `player::spend_gauge` (spirit and the giant trigger start
  there; the orb clear, `hit_boxes::clear_orbs`, follows).
- `statuses_and_aggro` runs `stagger_tick` and `down_tick`; `perfect_slow` computes the slowed set
  from the tick's start positions (`Scratch::slowed`, recomputed, not state); a slowed boss or
  enemy holds (`player::slowed`).
- Choices the code fixes, so a change is deliberate: a counter needs a smash step on a
  `counterable` wind-up (`attacks::counterable_run`); any hit on a player sets the ranged grace
  (`player_hits.rs`).
- The enemy threat colour table (`threat_colours.rs`, `Engine::threat_colours`) decides nothing in
  the simulation.

## Tests

- By mechanic: `tests/poise.rs`, `stagger.rs`, `knockdown.rs`, `counter_window.rs`, `evade.rs`,
  `rally.rs`, `repeat.rs`, `ranged_*.rs`, `expertise_*.rs`, and the `src/sim/*_tests.rs` units.
  The building bot (`tests/party_bot.rs`) spends a full gauge and picks styles by seed
  (`party-play.md`).
