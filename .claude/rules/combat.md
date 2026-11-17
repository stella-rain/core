---
paths:
  - "src/sim/{stun,hit_boxes,player,player_hits,collide,combat,runes}.rs"
  - "src/sim/attacks/ranged.rs"
  - "src/sim/attacks/runs.rs"
  - "src/sim/state/foes.rs"
  - "src/sim/state/player_state.rs"
  - "src/{threat_colours,rules}.rs"
  - "src/sim/{breaker,counter,gauge,magnet,perfect_evade,rally,repeat,spirit,tempo,trigger}_tests.rs"
  - "tests/{poise,stagger,stun,knockdown,counter_window,evade,rally,repeat,threat_colours}.rs"
  - "tests/ranged_*.rs"
  - "tests/expertise_*.rs"
  - "tests/common/mod.rs"
---

# Combat rules: how one is built and touched

Sources: ADR-SIM-3, ADR-SIM-9, ADR-SAVE-4, ADR-REPL-2. The weapon's sets and styles:
`weapon-tree.md`, the style kinds: `weapon-styles.md`; the bytes: `state-hash.md`. This file holds how a rule is built, not its
numbers.

## What a combat rule is

- A `rules.json` key read into `Tuning` (`src/rules.rs`; `COMBAT_RULE_KEYS` in `tests/common/mod.rs` lists them: poise damage,
  flinch, the hard-control list, `vulnerable`, the stagger numbers, knockdown, the counter stun, `ranged_grace_ticks`, the perfect
  evade, `orb_clear_radius`); a field of an enemy, boss or attack phase (`poise`, `stagger_max`, `counterable`, `poise_damage`,
  `advance`: `behaviour.rs`, `attack.rs`); or a style kind (`weapon-styles.md`).
- An absent key or a kind no style names is off: one bool test (`World::has_stagger`, `has_down`, `has_gauge`, ...), no state
  moved. `sim_version` 2 only (ADR-REPL-2).
- The bundled content ships them on: the combat keys, poise and `stagger_max` on every enemy and boss, the ranged grace, the
  perfect evade, the orb clear and the shipped styles' kinds; the `rally_*` keys are not shipped. A test that assumed a rule off
  starts from `dormant_rules_value()` and sets its own keys; a test that reads the styles uses `crate::before` (`weapon-tree.md`).
  A change that turns a rule on moves the corpus, seeds, scan constants, pins and generated sets in the same PR (ADR-SIM-3).

## State

- A rule's state is `PlayerCombat`, `FoeControl`, `Stagger` and `State.repeats` (`MAX_REPEATS_QUEUED`), and the foe status
  `repeat_bleed` that the repeat bleed puts on a foe (`StatusKind::RepeatBleed`), and nothing else; a view or an event is derived
  (`state-hash.md`). A new field is a hash move and its own change: format bump, pins, corpus, bench (ADR-SAVE-4).
- Snapshot fields and events for the gauge, spirit, counter, tempo, trigger, rally, perfect evade and the stagger burst are
  derived: the app draws them, and a change to one is a schema change (`schema-change` skill).

## One path per effect

- Every stagger fill is `stun::fill` (the counter bonus's factor, then the breaker's, then the refill percent; a full gauge
  staggers, for the ticks the breaker's `stagger_ticks_pct` scales once (`player::breaker_stagger_ticks`); `Stagger::total` stores
  that length and the status bar reads it); the breaker's burst, `player::stagger_burst`, runs right after with the filling hit's
  damage.
- A taken counter pays once, at its first landed combo hit (`player::counter_first_hit`, then `counter_pay`): `refund_evade`
  returns the last spent evade charge; `gauge_fill_pct` adds that share of the gauge's `max`, never past it, ignoring the step's
  `step_cap`; `PlayerCombat::counter_payout` is the owed flag. With no gauge kind the fill is nothing: a style that names a
  `gauge` with a `full_smash` slam (the Foresight capstone) gives the fill a bar; the next B press at a full bar plays the slam.
- Every combo end seen from outside is `player::combo_reset(.., Reset::{Window, Continue})`: it ends the tempo's stacks
  (`tempo_lose`) unless `tempo_carries` (the combo ended on a smash pressed at full stacks, the grant names `carry_at_full`, and
  the reset is a `Continue`: any but the window running out, which always ends them). A carried state is timed by the combo
  window at the root (`window_left`: set at the carry, kept by a root reset, `tempo_lose` at 0, dropped once the stacks are gone).
  The giant trigger's `end_ring` is `player::trigger_ring`, started where the count ends, only if the player is up (a down: none).
- A repeat (`combat::deal_repeats`) has no crit, echo, smash effect or flinch; its side effects are the on-hit runes at its own
  factor; the orb break of the owner's `repeat_breaks_orbs`, read at the deal (`World::repeat_break_of`); and the bleed of its
  stored row (`Repeat.bleed`, fixed when queued: `runes::apply_repeat_bleed`, the status `repeat_bleed`, a damage-over-time status
  of its own that the runes' bleed, `reap` and the spirit's stacks do not see). Every damage-over-time tick is a `Shot.dot` hit
  and never crits or echoes, whatever rune or none applied it.
- A hard control is `stun::land_hard_control` (the n-th is shorter, then refused; knockdown reuses it). Flinch is
  `hit_boxes::flinch_by`.
- Every cancel (stun, flinch, knockdown, stagger, counter) ends runs through `cancel_runs_of` / `RunEnd::Cancel`;
  `counterable_run` reads a pending `Cancel` or `All` in `ends_pending`, so a new `RunEnd` variant keeps that read. A new
  super-armour condition goes in `player::super_armour`.
- A smash step spends a full gauge in `player::spend_gauge` (spirit and the giant trigger start there; the orb clear,
  `hit_boxes::clear_orbs`, follows).
- `statuses_and_aggro` runs `stagger_tick` and `down_tick`; `perfect_slow` computes the slowed set from the tick's start positions
  (`Scratch::slowed`, recomputed, not state); a slowed boss or enemy holds (`player::slowed`).
- Choices the code fixes, so a change is deliberate: a counter needs a smash step on a `counterable` wind-up
  (`attacks::counterable_run`); any hit on a player sets the ranged grace (`player_hits.rs`).
- The enemy threat colour table (`threat_colours.rs`, `Engine::threat_colours`) decides nothing in the simulation.

## Tests

- By mechanic: `tests/poise.rs`, `stagger.rs`, `knockdown.rs`, `counter_window.rs`, `evade.rs`, `rally.rs`, `repeat.rs`,
  `ranged_*.rs`, `expertise_*.rs`, and the `src/sim/*_tests.rs` units. The building bot (`tests/party_bot.rs`) spends a full gauge
  and picks styles by seed (`party-play.md`).
