---
globs:
  - "src/sim/expertise.rs"
  - "src/sim/levels.rs"
  - "src/sim/levels_tests.rs"
  - "src/sim/tree_stats_tests.rs"
  - "src/expertise.rs"
  - "src/run_stats.rs"
  - "tests/weapon_tree.rs"
---

# The weapon in equipment mode: levels, sets and the tree

Sources: ADR-SIM-3, ADR-SIM-7, ADR-SAVE-4, ADR-CONT-3. Equipment mode: `equipment.md`; the
savepoint's checks: `savepoint.md`. Offers mode keeps the expertise offers (`expertise.rs`).

## Levels and tree points

- The gauge is the weapon's one expertise gauge. In equipment mode a level opens no offer and
  stops nothing: `equipment_pass` only raises the weapon level (`offered`); no stat, no event, no
  draw.
- The levels of the stage's level table are tree points: the style, the two upgrades and the
  capstone, in that path order. Points are derived, earned (`tree_earned`) minus taken, and held
  until the player takes one by `TreePick` (`tree_pick`). `check_tree_pick` refuses with
  `NoTreePoint` (none held, no gauge or no expertise content) or `NoTreeOption` (the option is
  past the next pick's choices); the engine maps them to `ActionError`.
- A style pick sets `style` and adds no stat of its own: the tree's set is all it gives. An
  upgrade or capstone applies its effect and records its place in `path`.

## The sets

- Two kinds of set live in the pack's `expertise.json`: the base set (`WeaponExpertise.base`,
  active before a style is picked) and one tree set per style (`Style.stats`). A `SetStat` is an
  effect with `by_level`: exactly ten entries for weapon Lv1 to Lv10, never falling; at most
  `MAX_SET_STATS` effects per set, each once. The values are tenths of a percent; `max_hp` is whole
  hit points, bounded by `MAX_HP_AMOUNT`.
- `tree_stats` and `set_for` read the set at the weapon level (0 reads as 1, past 10 it stops).
  A weapon with no expertise content, an inline weapon or a missing style gives the empty set.
- `Progress.set_tenths` holds the set's tenths, one per effect of `StatEffect::ALL`. It is
  written by `write_set` and recomputed, never stacked: the set of the level and style before is
  replaced by the set after, at the start of a run (`start_set`, base set at Lv1), at a weapon
  level-up (`level_up`) and at a style pick. `max_hp` goes through `add_effect` and
  `remove_effect` by the difference, so it lives in the totals and its `set_tenths` entry stays 0.
- Every other source stays in `Progress.totals`; the damage and speed rules read both.

## Stat effects

- `StatEffect::ALL` has 13 entries and fixes the order of `totals` and `set_tenths`
  (`run_stats.rs`). `StatEffect::RELEASED` is the first six, what `sim_version` 1 released; the
  frozen rules keep it, so a new effect never changes them (`sim_v1/state.rs`).

## What the frontend sees

- `PlayerView.tree_ready` (`TreeReadyView`: the pick's kind, level, points held and its option
  ids) is none in offers mode and while no point is held. `PlayerView.weapon_stats`
  (`WeaponStatsView`: the weapon level, the active style, the base set and every tree's, each
  effect now and at the next level) is none outside equipment mode. Both read, write and hash
  nothing.

## Tests

- `tests/weapon_tree.rs` (picks, refusals, the sets across levels and styles);
  `tree_stats_tests.rs` (the set reader); `tests/equipment_pack.rs` (the pack's sets).
