---
globs:
  - "src/expertise.rs"
  - "src/pack/check.rs"
  - "src/sim/{expertise,stacking_tests}.rs"
  - "tests/*style*.rs"
---

# Style kinds

Sources: ADR-SIM-9, ADR-SIM-3, ADR-SAVE-4. The weapon, its levels and the tree: `weapon-tree.md`; mechanics: `combat.md`.

- Style kinds (ADR-SIM-9): the base tree is plain; a play difference comes only from a closed `StyleKind` list (`src/expertise.rs`)
  whose numbers sit in `expertise.json`, bounded by the validator and `pack/check.rs`, at most `MAX_LEVEL_KINDS` per level, each
  kind once per level. `breaker`, `spirit`, `counter_bonus`, `combo_tempo_pct` and `giant_trigger` may be named at every level of a
  path and stack in path order, laid over the earlier levels once at world build (`spirit_fold`, `counter_fold`, `tempo_fold`,
  `trigger_fold`, `breaker_folded`):
  - A later level's scalar (`decay_ticks`, `window_ticks`, `max_stacks`, `ticks`) replaces the earlier one; an effect it lists
    replaces the same named effect, one it does not list stays; the flags (`super_armour`, `keep_on_smash`,
    `super_armour_at_full`) stay on once any level names them. Spirit tables merge by index (entry `i` over level `i + 1`), a longer
    one extending it to `MAX_SPIRIT_LEVEL`; an empty entry changes nothing.
  - A level after one naming the kind may list no effects (the tempo need not list `speed_pct`); the first naming lists 1 to 4
    (tempo 1 to 3, with `speed_pct`); a spirit always lists 1 to `MAX_SPIRIT_LEVEL` entries, each may be empty.
  - Tempo: `max_stacks` never falls along the path and the folded `max_stacks` times `speed_pct` stays within
    `MAX_ATTACK_SPEED_PCT` (800) at every place (`check_tempo_fold`); the fold clamps each number again.
  - Grants are read from the recorded style and path (`World::grant_slot`, `weapon_of`, `gauge_of`, `breaker_of`, ...): one bool
    test when no style names the kind, no state or input added; a style change empties the old style's gauge, spirit, counter,
    tempo and trigger state (`tree_pick`). Mechanics: `combat.md`.
