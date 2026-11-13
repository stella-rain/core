---
paths:
  - "src/sim/expertise.rs"
  - "src/sim/levels.rs"
  - "src/sim/levels_tests.rs"
  - "src/sim/tree_stats_tests.rs"
  - "src/expertise.rs"
  - "src/run_stats.rs"
  - "tests/weapon_tree.rs"
  - "tests/level_table.rs"
  - "tests/*style*.rs"
  - "tests/before.rs"
  - "src/sim/before.rs"
  - "src/sim/try_styles_tests.rs"
---

# The weapon in equipment mode: levels, sets and the tree

Sources: ADR-SIM-3, ADR-SIM-7, ADR-SIM-9, ADR-SAVE-4, ADR-CONT-3. Equipment mode: `equipment.md`;
the savepoint's checks: `savepoint.md`.

## Levels and tree points

- The gauge is the weapon's one expertise gauge. A level opens no offer and stops nothing: `expertise::level_up` only
  raises the weapon level and rewrites the set (`write_set`); no stat, no event, no draw.
- The levels of the stage's level table are tree points: the style, the two upgrades and the
  capstone, in that path order. `Stage::level_table` defaults to `LevelTable::NORMAL`, to
  `V1_NORMAL` at `sim_version` 1 (the frozen rules keep their table). Points are derived, earned (`tree_earned`) minus taken, and held
  until the player takes one by `TreePick` (`tree_pick`). `check_tree_pick` refuses with
  `NoTreePoint` (none held, no gauge, no expertise content or no style offered) or `NoTreeOption` (the option is
  past the next pick's choices); the engine maps them to `ActionError`.
- A style pick sets `style` and adds no stat of its own: the tree's set is all it gives. An
  upgrade or capstone applies its effect and records its place in `path`.
- Style kinds (ADR-SIM-9): the base tree is plain; a play difference comes only from a closed
  `StyleKind` list (`src/expertise.rs`) whose numbers sit in `expertise.json`, bounded by the
  validator and `pack/check.rs`, at most `MAX_LEVEL_KINDS` per level. A style path names each kind
  once except `breaker`, which stacks across levels in path order (a later same effect replaces the
  earlier). Grants are read from the recorded style and path (`World::grant_slot`, `weapon_of`,
  `gauge_of`, `breaker_of`, ...): one bool test when no style names the kind, no state or input
  added; a style change empties the old style's gauge, spirit, counter, tempo and trigger state
  (`tree_pick`). Mechanics: `combat.md`.
- `RunSetup.unlocked_styles` becomes `World::styles_offered`: the offer lists only those styles;
  `check_tree` does not refuse a gauge of a style the setup did not unlock. `Stage.try_styles`
  (tutorial mode only) makes every table level a style pick.

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
  ids, and `kinds`, `StyleKindView`) is none at `sim_version` 1 and while no point is held. `PlayerView.weapon_stats`
  (`WeaponStatsView`: the weapon level, the active style, the base set and every tree's, each
  effect now and at the next level) is none at `sim_version` 1, without an expertise gauge and for an inline weapon.
  Both read, write and hash nothing.

## Tests

- `tests/weapon_tree.rs` (picks, refusals, the sets), `tree_stats_tests.rs` (the set reader),
  `tests/equipment_pack.rs` (the pack's sets), `tests/level_table.rs`, the `*style*` tests. A hand
  test that reads the bundled styles uses the content from before the weapons' content moved: `crate::before` (`tests/before.rs`, `src/sim/before.rs`), as `tests/common/equip.rs` does.
