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
---

# Parties in play: acting, down, tether, rewards

Sources: ADR-SIM-7, ADR-SIM-8, ADR-SAVE-3, ADR-SAVE-6. Joining, leaving and the corpus:
`party.md`; the tick order: the `sim` module doc.

## Acting

- Every present slot acts in slot order, its whole `player::update` before the next's. A downed or out player's timers (invulnerability, evade recovery, dash-strike,
  skill cooldown) and held buttons still follow the input, but it does not act; `regen` and
  `orbits` skip it. Going down ends its dash, combo run, charge and hold (`stand_down`). The
  revive runs before the timers: invulnerability after a revive is 59 on the next tick.
- A rooted reviver: an up player holding `interact` within `REVIVE_RANGE` of a downed teammate
  (`near_downed`) stands still and does not attack, evade or use a skill that tick; a charge or
  hold breaks once; `assist` stays. `skill` and `auto` are also ignored (a placeholder). Every
  such player is rooted, not only the one whose hold counts.

## Down, revive, out

- Hp 0 downs a player in a party only (`PlayerDowned`); a solo player at 0 hp fails, and
  `DownView` shows only in a party. The run fails when no player is up (`decide_outcome`).
- `tick_down_and_revive`: `down_ticks` counts up; `OUT_TIMER_TICKS` (1200) makes the player out
  (`PlayerOut`, its offers dropped). The reviver is the first up player in slot order holding
  `interact` within `REVIVE_RANGE` (48 px); the search stops there, so one reviver counts per
  tick. `REVIVE_HOLD_TICKS` (180) in a row revive at `REVIVE_HP_PCT` (30) of `max_hp`, at least
  1, with hit invulnerability (`PlayerRevived`). With no reviver in range `revive_hold` resets.
- Offers: a downed player takes option 0 of every offer on the tick it opens, a whole queue on
  one tick, at most `MAX_AUTO_PICKS` (256) per slot (`take_downed`). An out player opens nothing
  and its queue is forgotten (`drop_offers`). In a `timed` run a player that goes down with an
  offer open has its deadline set to that tick. `levels::add_effect` raises `max_hp` for MaxHp
  but heals only a player with hp above 0.

## Soft tether

- The `tether` step runs between `advance_runs` and `player_update`. With two or more up players,
  when the farthest pair is more than `TETHER_RANGE` (840 px, two view half widths, ADR-SIM-5)
  apart, each up player at the greatest distance from the bounding box's centre that is not
  dashing moves toward it by 1/`TETHER_PULL_DIV` (8, a placeholder) of the overshoot. A dash is
  never blocked; downed and out players are neither measured nor pulled. Integer math, no
  allocation. A party of one returns at once; the frozen rules have none.

## Rewards
- Experience (`levels::credit_defeated`) and Stardust (`stardust::credit_defeated`,
  `boss_killed`) go only to a player that is up (`is_up`), in every run, solo included.
- The shared star drop's tier reads the highest level among up players, among the present ones
  when none is up (`top_up_level`, `top_level` in `stars.rs`).
- `stars::boss_killed` gives every up player a Boss offer due first, at one rarity for the party
  (the top of the tier for the highest level among the up players, held to the difficulty's
  range). A downed or out player gets none. The boss heal skips hp 0.

## Per-player result
- `Engine::stardust_of(player)` and `RunResult::from_run_for(setup, engine, player)` give one
  player's Stardust and result (`None` for an absent slot, or a result while running); a downed
  or out player's result is the run's outcome with its own Stardust. `stardust` and `from_run`
  are slot 0's. Settling each save is the app's (ADR-SAVE-6 point 2).

## Tests

- `src/sim/`: `party_act_tests.rs` (acting, rooted reviver), `party_fix_tests.rs` (auto-picks,
  out, rewards, boss offers), `tether_tests.rs`. `tests/party_down.rs`: down, out timer, revive,
  wipe, events once each, no solo down view. `tests/party_tether.rs`: a walker held within
  856 px over 600 ticks, a three-player party deterministic.
