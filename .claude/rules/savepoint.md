---
paths:
  - "src/savepoint/**"
  - "tests/savepoint.rs"
  - "tests/savepoint_proof.rs"
  - "tests/fixtures/savepoint/**"
---

# The savepoint file and its pins

Sources: ADR-SAVE-4, ADR-REPL-5. Also `replay.md`, `state-hash.md`.

## Pins

A pinned format or hash is never re-pinned: a change is a new version.

- Savepoint: `tests/fixtures/savepoint/v<N>.sha256` per `SAVEPOINT_FORMAT` (now 10,
  `savepoint/file.rs`; `v1` to `v10` are pinned and `tests/savepoint.rs` wants one per older
  format and refuses each); the v2 savepoint bytes are pinned by `sim::players_pin_tests` in
  `src/`. A file of format 1 to 9 is refused as `Format`.
- The format history: 4 added the party and each player's equipment; 5 the stat totals; 6 13
  totals (`StatEffect::ALL`) and, after them, the 13 weapon-set entries `Progress.set_tenths`
  (`weapon-tree.md`); 7 drops the stale offer fields of `Progress` and `ExpertiseGauge`; 8 drops
  the star-offer fields of `Offers`; 9 adds the combat fields (per agent `FoeControl`, per elite
  and boss `Stagger`, per player `PlayerCombat`, the run's `repeats`; `state-hash.md`); 10 adds
  `PlayerCombat::counter_bonus_step`. A new combat field is the next format (ADR-SAVE-4).
- The payload starts with a fixed 60-byte `PlayerStats` record (per slot `damage_dealt` u64,
  `kills` u32, `support` u64; `STATS_RECORD_LEN`, `ledger.rs`), unhashed and trusted like the
  replay prefix, and a slot outside the party may hold a leaver's totals (`decode_for` ignores
  the mask). Then the state bytes (`state-hash.md`: the presence byte, the generations, a block
  per present player, `Progress` with its totals and `set_tenths`, then the player's equipment). `restore` builds its
  fresh engine as the party the prefix log ends
  on, after its last join or leave (`presence_at_end`); a size mismatch is `Damaged` or
  `PrefixMismatch`.

## What the load refuses

`decode_over` and `check_indices` (`src/savepoint/state.rs`) refuse, besides the party checks of `state-hash.md`:

- an equipment star (inventory, slot or Lumine slot) out of range of the rewards file, or the
  lone random rune;
- a star in a slot of another category or with none, or two equipped stars sharing an option (`slots::misfit`, run at
  every `sim_version` 2 load). The same rune in two rune slots is not checked here;
- a weapon-tree path taken past the level's points, a place past its choices, or a style the
  weapon lacks (`check_tree`);
- a `set_tenths` entry above its bound, or a non-zero `MaxHp` entry (the set's hit points live
  in the totals);
- combat state out of its bound: every `PlayerCombat`, `FoeControl` and `Stagger` field is held
  to its limit constant (`within`, "<field> past its bound"); `rally` above the player's
  `max_hp`; a queued repeat whose `attack` or `phase` is out of range. The checks are per
  field, not across fields: a `rally_ticks` with `rally` 0 is not refused.

A load does not start a skill star's cooldown: only a swap in play does (`equipment.md`).
