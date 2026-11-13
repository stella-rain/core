---
globs:
  - "src/sim/player.rs"
  - "src/sim/schedule.rs"
  - "src/sim/mod.rs"
  - "src/sim/stars.rs"
  - "src/sim/levels.rs"
  - "src/sim/stardust.rs"
  - "src/sim/heal.rs"
  - "src/sim/snapshot.rs"
  - "src/progression.rs"
  - "src/sim/tether_tests.rs"
  - "src/sim/party_act_tests.rs"
  - "src/sim/party_fix_tests.rs"
  - "tests/party_down.rs"
  - "tests/party_tether.rs"
  - "tests/party_bot.rs"
  - "tests/run_play.rs"
  - "tests/sync_rig.rs"
---

# Parties in play: acting, down, tether, rewards

Sources: ADR-SIM-7, ADR-SIM-8, ADR-SAVE-3, ADR-SAVE-6, ADR-REPL-5. Joining, leaving and the corpus: `party.md`; the
tick order: the `sim` module doc.

## Acting

- Every present slot acts in slot order, its whole `player::update` before the next's. A downed or out player's timers
  (invulnerability, evade recovery, dash-strike, skill cooldown) and held buttons still follow the input, but it does
  not act; `regen` and `orbits` skip it. Going down ends its dash, combo run, charge and hold (`stand_down`). The
  revive runs before the timers: invulnerability after a revive is 59 on the next tick.
- A rooted reviver: an up player holding `interact` within `REVIVE_RANGE` of a downed teammate (`near_downed`) stands
  still and does not attack, evade or use a skill that tick; a charge or hold breaks once; `assist` stays. `skill` and
  `auto` are also ignored (a placeholder). Every such player is rooted, not only the one whose hold counts.

## Down, revive, out

- Hp 0 downs a player in a party only (`PlayerDowned`); a solo player at 0 hp fails, and `DownView` shows only in a
  party. The run fails when no player is up (`decide_outcome`).
- `tick_down_and_revive`: `down_ticks` counts up; `OUT_TIMER_TICKS` (1200) makes the player out (`PlayerOut`). The
  reviver is the first up player in slot order holding `interact` within `REVIVE_RANGE` (48 px);
  the search stops there, so one reviver counts per tick. `REVIVE_HOLD_TICKS` (180) in a row revive at `REVIVE_HP_PCT`
  (30) of `max_hp`, at least 1, with hit invulnerability (`PlayerRevived`). With no reviver in range `revive_hold`
  resets.
- `levels::add_effect` raises `max_hp` for MaxHp but heals only a player with hp above 0.

## Soft tether

- The `tether` step runs between `advance_runs` and `player_update`. With two or more up players, when the farthest
  pair is more than `TETHER_RANGE` (840 px, two view half widths, ADR-SIM-5) apart, each up player at the greatest
  distance from the bounding box's centre that is not dashing moves toward it by 1/`TETHER_PULL_DIV` (8, a
  placeholder) of the overshoot. A dash is never blocked; downed and out players are neither measured nor pulled.
  Integer math, no allocation. A party of one returns at once; the frozen rules have none.

## Rewards
- The live rules open no offer: a taken star and a boss kill give each member rolls of its own (`party-equipment.md`).
- Experience (`levels::credit_defeated`) and Stardust (`stardust::credit_defeated`, `boss_killed`) go only to a player
  that is up (`is_up`), in every run, solo included.
- The shared star drop's tier reads the highest level among up players, among the present ones when none is up
  (`top_up_level`, `top_level` in `stars.rs`).
- `stars::boss_killed` makes no offer due: it gives the Starlight bonus and `drops::boss_killed` the rolls
  (`party-equipment.md`). The boss heal skips hp 0.

## Per-player result
- `Engine::stardust_of(player)` and `RunResult::from_run_for(setup, engine, player)` give one player's Stardust and
  result (`None` for an absent slot, or a result while running); a downed or out player's result is the run's outcome
  with its own Stardust. `stardust` and `from_run` are slot 0's. Settling each save is the app's (ADR-SAVE-6 point 2).

## Tests

- `src/sim/`: `party_act_tests.rs` (acting, rooted reviver), `party_fix_tests.rs` (out, max hp, experience, Stardust,
  the drop tier), `tether_tests.rs`. `tests/party_down.rs`: down, out timer, revive, wipe, events once each, no solo down
  view. `tests/party_tether.rs`: a walker held within 856 px over 600 ticks, a three-player party deterministic.
- `tests/party_bot.rs`: `PartyBot` (a pure function of seed, engine, slot and tick), `RigParty` (the builder over
  `run_play::PartyCase`), `RigRun` (replays a recorded run onto an engine). With `builds`, in an equipment run a bot
  acts one tick in `ACTION_EVERY` (30) per slot: the tree point (a style by the seed, option 0 for an upgrade or capstone; presses the smash while `Sight.gauge_full`, so the corpus plays the style kinds); best rated equip or replace (never lowers a slot);
  extract the rest; with an empty inventory, craft the best rated star; each action passed `Engine::check_action`.
- Cases `equipment_rig_1` to `_3` (3000 ticks, 1 to 3 players; each must pass `rig_builds_miss`); `tests/sync_rig.rs`
  rolls the rig cases back 1 to 8 ticks at every boundary and compares hashes.
