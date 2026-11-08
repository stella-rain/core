---
paths:
  - "src/sim/build.rs"
  - "src/sim/slots.rs"
  - "src/sim/slots_tests.rs"
  - "src/sim/starlight.rs"
  - "src/sim/starlight_tests.rs"
  - "src/sim/drops.rs"
  - "src/sim/drops_tests.rs"
  - "src/sim/equipment.rs"
  - "src/sim/loadout.rs"
  - "tests/equipment_*.rs"
---

# Equipment mode: the build, slots, Starlight and drops

Sources: ADR-SIM-3, ADR-SIM-5, ADR-SIM-7, ADR-SAVE-4, ADR-SAVE-7, ADR-CONT-3. The weapon:
`weapon-tree.md`; a party: `party-equipment.md`; the file: `savepoint.md`; rewards: `content.md`.

## The build

- Equipment mode is `RunSetup.build_mode: Equipment`, read as `World.equipment_mode` in a run of `sim_version` 2;
  offers mode runs none of this (`stars.rs` branches before its own body). A setup in equipment mode on a pack with no
  `equipment` object is refused by `run_setup::validate_in`.
- The build is what the stars equipped add up to (`build.rs`: `Build`, `Grants`), not the inventory. It is re-derived,
  never stacked: `slots::refresh` writes the difference between the build of the stars before and after
  (`Build::write`, `Grants::write`) and recounts the sets.
- Rules are pack data: `EquipmentRules` (`rewards.json`'s `equipment` object, held in `World`) has the category slots,
  the rune slots, the kills each level takes, the percentage each level adds, the Starlight values, the boss bonus and
  drops and the rune share. No const in code holds one; a doc names the field, not the figure
  (`tests/equipment_pack.rs` pins the pack).

## Slots

- The layout is the categories' slots in list order, then the rune slots (`fresh_slots`); the Lumine slot follows when
  the setup names a Lumine star. A star fits the slot of its reward's `category`; the Lumine slot takes any star, a
  rune slot only a rune. No two equipped stars share an option, the Lumine slot counted (`slots::check`, `SlotError`:
  the item, the slot, what fits, then the option or the rune).
- The Lumine star is in the build at Lv1 from tick 0 (`loadout::apply`; nothing drawn). It levels like any slot and is
  replaceable. At tick 0 `slots::refresh` runs with `Arrival::Start`.
- Expertise (`slots::credit_kills`): each kill adds one point to every slot holding a star; a level rises at the
  pack's kills, to `MAX_LEVEL`, and adds the pack's percentage of the star's base (`level_scaled`); a level-up
  rewrites the build from the levels before and after.
- A replace by a star sharing no option halves the level (at least 1) and zeroes the kills; a same-option switch keeps
  both. A rune keeps level 1.

## A swap in play (`Arrival::Swap`)

- Hit points keep their fraction when the most hit points change (`keep_fraction`: floored, at least 1 for a living
  player, 0 stays 0), so a swap never heals. The XP point, a perk, an option and a tree set still heal by the amount
  (`add_effect`).
- A stage skill star that enters a slot (equip, replace, a level-up that changes it) starts the full cooldown a press
  sets (`skill::cooldown_ticks`); a grade switch keeps its own. Tick 0 and a savepoint load do not cool.
- The snapshot's `pickup_radius_pct` and the offer rating are the total: the stars' share plus the stat, held to
  `MAX_PICKUP_RADIUS_TOTAL_PCT` (`pickup_pct_of`).

## Starlight and drops

- Starlight (`starlight.rs`, `Equipment.starlight`): extract gives an inventory item's value by rarity (a rune the
  flat `rune_extract`); craft takes an index of the setup's star pool, costs the rarity's price, draws nothing and
  ignores the difficulty's rarity range; a named star already held is refused. A downed or out player does neither:
  the window lock refuses every action first (`party-equipment.md`). Nothing carries Starlight out of the run.
- Drops (`drops.rs`, `roll`): a taken falling star gives every member that is not out a roll from its own offer
  stream: the kind `below(1000)` (a rune under `rune_share_permille`), then the rune pool or the weighted star pool.
  The weight is the star's own, scaled by the setup's `drop_weight` percent for the star's category
  (`World.drop_weight_pct`). Past the inventory bound the draws are made and the item is dropped. The pool is
  `Rewards::default_pool(mode)` unless the setup names one.
- The draw order is fixed per `sim_version`; a change to it is a new version (`determinism.md`).

## Tests on a hand-built stage

- Equip the star instead of playing to an offer: `common::equip` (`tests/common/equip.rs`, ADR-SIM-7). `equipped` and
  `try_equipped` (the refusal as an `ActionError`) take the rewards file, `(reward ID, slot)` pairs and a level;
  `equipped_in` and `try_equipped_in` also take `Parts` (skills, sets, runes, expertise). Stacks, clamps and recorded
  runs stay on offers.
- They rest on the seam, under the `bench` feature and hidden from docs: `Engine::new_equipped` (a stage as given),
  `script_equipment`, `script_runes` and `script_expertise` (each before the first tick).
