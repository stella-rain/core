---
paths:
  - "src/expertise.rs"
  - "src/pack/check.rs"
  - "src/sim/{expertise,stacking_tests}.rs"
  - "tests/*style*.rs"
---

# Style kinds

Sources: ADR-SIM-9, ADR-SIM-3, ADR-SAVE-4. The weapon, its levels and the tree: `weapon-tree.md`; mechanics: `combat.md`.

- Style kinds (ADR-SIM-9): the base tree is plain; a play difference comes only from a closed `StyleKind` list
  (`src/expertise.rs`) whose numbers sit in `expertise.json`, bounded by the validator and `pack/check.rs`, at most
  `MAX_LEVEL_KINDS` per level, the stacking kinds once each per level. `breaker`, `spirit`, `counter_bonus`, `combo_tempo_pct`
  and `giant_trigger` may be named at every level of a path and stack in path order, laid over the earlier levels once at world
  build (`spirit_fold`, `counter_fold`, `tempo_fold`, `trigger_fold`, `breaker_folded`):
  - A later level's scalar (`decay_ticks`, `window_ticks`, `max_stacks`, `ticks`) replaces the earlier one; an effect it lists
    replaces the same named effect, one it does not list stays; the flags (`super_armour`, `keep_on_smash`,
    `super_armour_at_full`) stay on once any level names them. Spirit tables merge by index (entry `i` over level `i + 1`), a longer
    one extending it to `MAX_SPIRIT_LEVEL`; an empty entry changes nothing.
  - A level after one naming the counter, tempo or trigger may list no effects (the tempo need not list `speed_pct`); the first
    naming lists 1 to 6 for the counter bonus, 1 to `MAX_TRIGGER_EFFECTS` (5) for the giant trigger (with `hold_move_pct`, 1 to
    `MAX_HOLD_MOVE_PCT` 100) and 1 to 3 for the tempo (with `speed_pct`); the breaker has no empty level: it always lists 1 to 5
    (`stagger_ticks_pct` is its fifth, beside `stagger_pct`, `poise_pct`, `part_break_pct`, `on_stagger_burst`); a spirit always
    lists 1 to `MAX_SPIRIT_LEVEL` entries, each may be empty.
  - Tempo: `max_stacks` never falls along the path and the folded `max_stacks` times `speed_pct` stays within
    `MAX_ATTACK_SPEED_PCT` (800) at every place (`check_tempo_fold`); the fold clamps each number again.
  - Grants are read from the recorded style and path (`World::grant_slot`, `weapon_of`, `gauge_of`, `breaker_of`, ...): one bool
    test when no style names the kind, no state or input added; a style change empties the old style's gauge, spirit, counter,
    tempo and trigger state (`tree_pick`). Mechanics: `combat.md`.
- Single kinds: `smash_effect`, `gauge`, `repeat_breaks_orbs` and `repeat_bleed` once per style path (the last two need a `repeat`
  at or before their level); `replace_step` once per step per style (distinct steps), `proc_factor` once per step per path,
  `repeat` up to `MAX_STEP_REPEATS` (4) per step per path; `charge_sooner` once per step per path, after the grant that gives the
  step a charge. `charge_sooner` is applied when the world builds the style's weapon (`weapon_with_style`): no state, no hash
  field. `repeat_bleed` is stamped on each queued repeat by the same function and does move state: the repeat's hashed, saved
  `bleed` row (`Repeat.bleed`, savepoint format 12) and the `repeat_bleed` status it puts on foes. `repeat_breaks_orbs` is read
  at the deal (`combat.md`).
