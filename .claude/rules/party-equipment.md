---
paths:
  - "src/engine/run.rs"
  - "src/sim/drops.rs"
  - "src/sim/starlight.rs"
  - "src/sim/state.rs"
  - "src/sim/state/**"
  - "src/sim/party_equip_tests.rs"
  - "tests/equipment_party.rs"
---

# Equipment mode in a party

Sources: ADR-SIM-7, ADR-SIM-8, ADR-SAVE-7. The mode: `equipment.md`; down, revive, out:
`party-play.md`; joining and leaving: `party.md`.

## Rewards

- No offer opens: the live rules open none (`stars.rs`). A taken
  falling star gives every member that is not out a roll of its own, from its own offer stream,
  the downed included (`drops::pick_up`); a downed member cannot take a star itself.
- A boss kill gives each member that is up the pack's `boss_drops` rolls of its own, at one
  rarity for the party (`drops::boss_killed`), and the pack's `boss_bonus` Starlight
  (`starlight::boss_bonus`). A downed or out member gets neither.

## The window lock

- `Engine::check_action` refuses every kind of equipment action (equip, replace, extract, craft,
  tree pick) with `ActionError::Downed` (hp 0, not out) or `ActionError::Out`, after `NoPlayer`
  and before the rule's own refusals (`Run::window_lock`). The window opens again on the revive;
  out is for good.
- Only a run of the live rules (`Run::V2`, `sim_version` 2) locks; a `sim_version` 1 run has no window and keeps
  `NotBuilt` for all.
- A recorded action of a locked slot is refused on replay, so the run does not verify
  (`ActionRefused`).

## Members, joiners, leavers

- A member at the start and a joiner are a `Career::member` (`state.rs`): slot 0's start career
  (progress, offers, expertise gauge, Stardust and `regen` as at the start), the slots and the
  Lumine slot as they are, an empty inventory, no Starlight and no tree points
  (`Equipment::for_new_slot`). A rejoin starts again from the start career, not from the leaver.
- A leaver's equipment leaves the hash with its block; nothing of it carries to the next
  occupant (`state-hash.md`).

## Tests

- `src/sim/party_equip_tests.rs` and `tests/equipment_party.rs` pin the party rules above. The
  corpus `equipment_party_1` to `_3` and `equipment_two` pin party runs (`replay.md`).
