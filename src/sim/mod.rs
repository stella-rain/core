//! The live rules (ADR-REPL-2): the module that runs the newest `sim_version`, with the
//! earlier versions frozen beside it (`sim_v1`); the hash check across CPUs is never relaxed (ADR-SIM-3).
//!
//! The state holds its players as `players`, one now, addressed by a `PlayerIdx` (ADR-SIM-7); a
//! system that acts on or for a player takes `me` right after the state. A place that names no
//! player yet reads `PlayerIdx::SOLO` and says which later change settles it (`// N=1: ...`).
//!
//! What a tick does, in this order (`schedule::TICK` is the order in code, and a test keeps this
//! table equal to it):
//!
//! <!-- tick -->
//! | step | what it does |
//! |---|---|
//! | `spawn_boss` | the boss of a `time_limit` run whose time the tick reaches comes, and so does a stage boss with an arrival tick, before the counter advances (`horde`, `boss::arrive`) |
//! | `advance_tick` | the tick counter advances |
//! | `regen` | the main character regenerates the hit points of a perk (`loadout`) |
//! | `spawn_waves` | the waves that are due spawn enemies; a horde run holds them at the crowd target (DIR1) |
//! | `spawn_elite` | a `time_limit` run's elite table brings an elite when its time comes (`horde`) |
//! | `leash` | a regular, unanchored enemy outside the leash's reach rectangle (`leash_reach_x`, `leash_reach_y` of `rules.json`) around the view's centre (`view_centre`) moves to the centre's rectangle of `leash_land_x`, `leash_land_y` on the opposite side (or on the enemy's own side when that one is off the map), at most `MAX_LEASH_MOVES_PER_TICK` a tick (`leash`) |
//! | `statuses_and_aggro` | statuses run out and aggro fades, but damage-over-time statuses wait for `collide` (`entity`) |
//! | `perfect_slow` | the perfect evade's slowed set (enemies in the reach of a player whose slow runs) is worked out from the tick's start positions, then every player's window, cooldown and slow run down one (`player`) |
//! | `runes_tick` | each on-hit rune's cooldown counts down (`runes`) |
//! | `advance_runs` | every running phased attack advances a tick (`attacks`) |
//! | `tether` | with two or more up players, when the farthest two are more than `TETHER_RANGE` apart, each up player farthest from the bounding box's centre that is not dashing is pulled toward it by an eighth of the overshoot, so a dash is never blocked; downed and out players are neither measured nor pulled (ADR-SIM-7, `player`) |
//! | `player_update` | the main character turns to its stick, picks a target while aim assist or auto mode is on (`select`), recovers evade charges, evades, uses its stage skill (`skill`), walks its combo, then dashes or moves (`player`) |
//! | `runes_fly` | thrown runes' hit boxes (boomerang) fly a step, after the main character has moved (`runes`) |
//! | `orbits` | each orbit starts a round of its orbs' attacks when its cooldown and the round before are over, and turns them (`orbit`) |
//! | `boss_act` | the boss: phases, timeline, rules (`boss`) |
//! | `call_elites` | a boss that calls elites does so when its time comes (`boss`) |
//! | `agents_act` | companions, then enemies, in entity-ID order: move, then the first rule that can fire, fires (ADR-SIM-4); an elite holds where it spawned, and its rules fire only while the main character is in range (`agents`) |
//! | `dash_attackers` | an enemy or the boss in an active phase with a dash moves its share of the distance along its attack's locked facing, held to the map; a boss's parts follow (`dash`) |
//! | `push_apart` | the enemies push apart, all at once, and one found crowded does not step to its place on the next tick, and one that is dashing is not moved (ADR-SIM-5, `push`) |
//! | `skill_pull` | the stage skill's black hole pulls (`skill`) |
//! | `runes_steer` | homing missiles' hit boxes steer toward where their targets are now (`runes`) |
//! | `zones_advance` | the zones age and run out (`zones`) |
//! | `fire_hooks` | orbs whose lifetime is over start their bursts (`attacks`) |
//! | `run_attacks` | every running attack is carried out up to its next wait, making orbs and zones; the tick's `Load` is read here (`attacks`) |
//! | `move_orbs` | orbs move and leave the map |
//! | `collide` | damage-over-time statuses tick, then the hit boxes of active phases (`hit_boxes`), then orbs land on the main character and the companions; the shield takes from the main character's damage; experience, expertise points, runes' effects and drops follow the hits (`combat`) |
//! | `level_up` | the expertise gauge's levels are reached (`expertise`) |
//! | `decide_outcome` | death or running out of ticks fails, an empty stage clears |
//! <!-- /tick -->
//!
//! In a stage whose enemies or boss give expertise points, each hit and kill by a combo step in
//! `collide` adds them to the one expertise gauge, and its levels are reached in `level_up`
//! (`expertise`). Each level reached is due an offer of the kind the stage's level table gives
//!: common options, the weapon's three styles, or the chosen style's upgrades or
//! capstone.
//!
//! In a stage with levels, defeated enemies give their experience as they are removed in
//! `collide`, and each level-up raises a stat or opens a stat offer (`levels`). The progress is
//! per player slot (`Players.careers`), and so are the expertise gauge, the Stardust and the
//! regeneration (`Career`). Experience and Stardust are shared: every player that is up earns
//! each kill's, XP raised by that player's own `XpPct`. A kill's burst, kill-heal and expertise go to
//! the killer, the first hit of the tick (in `collide`'s fixed order) that takes the target to 0
//! hit points, found through the scratch `killers`; a damage-over-time tick is the hit of the
//! player who applied the stack last. Every such hit, and an echo, lowers the foe's hit points in
//! `damage::land`. The live rules open no offer, so the world never stops for one.
//! The hash and format 1
//! write slot 0's where the run's single ones were, and the tail holds the other slots' (progress,
//! offer generator, offers, expertise, Stardust, regen), a layout the hash-layout work settles.
//! In a stage with stars, defeated enemies then drop falling stars, and the main character takes
//! those it reaches (`stars`); the live rules open no star offer.
//!
//! The perfect evade (`player::near_miss`, the step `perfect_slow`) is off until the pack names it
//! (`Tuning.perfect_evade` is `None` without the `perfect_evade_*` keys; the bundled pack names
//! them) and attack magnetism (`player::magnetism`) is off until a phase names an `advance` (no
//! bundled phase does), so a run without them moves no state, hash or replay. The slowed set is
//! `Scratch::slowed`, recomputed each tick from the state.
//!
//! The weapon gauge a style grants (`World::gauge_of`, the `gauge` style kind) is off until a style
//! names it (the bundled pack does): it reads and writes only `PlayerCombat::weapon_gauge` and `gauge_step_fill`, fills in
//! `hit_boxes::land` (`player::gauge_from_hits`) and `player_hits::hurt_player`
//! (`player::gauge_from_damage`), is spent by a B step at a full gauge (`player::spend_gauge`),
//! and the orb clear (`hit_boxes::clear_orbs`) follows that attack. A run whose pack names no
//! gauge keeps every one of those at zero, one bool test each.
//!
//! The spirit level a style grants (`World::spirit_of`, the `spirit` style kind) is off the same
//! way (the bundled pack names it): it reads and writes only `PlayerCombat::spirit_level` and `spirit_decay`, is raised by the
//! full-gauge smash (`player::spend_gauge`), counted down in `player::update`, emptied by a pick
//! that changes the style (`expertise::tree_pick`), and adds its level's damage, range and bleed
//! stacks to a combo step (`player::strike`, `collide::damage`). `PlayerView.spirit` and the events
//! `SpiritRaised` and `SpiritFell` show it; none of them is state. A run whose pack names no
//! spirit keeps both fields at zero, one bool test each.
//!
//! The counter bonus a style grants (`World::counter_of`, the `counter_bonus` style kind) is
//! off the same way (the bundled pack names it): it reads and writes only `PlayerCombat::counter_window` and
//! `counter_bonus_step`, opens on a near miss of the evade and is taken by the first dash-branch
//! step started inside it (`player::near_miss`, `counter_take`; damage, stagger factor, stun and
//! slide in `strike`, `stun::fill`, `stun::counter_bonus_stun` and `magnetism`).
//! `PlayerView.counter` and the two counter events are not state. A run whose styles name no
//! counter bonus keeps both fields at zero, one bool test each.
//!
//! The combo tempo a style grants (`World::tempo_of`, the `combo_tempo_pct` style kind) is
//! off the same way (the bundled pack names it): it reads and writes only `PlayerCombat::combo_tempo`. A swing of a combo
//! step that hits adds one stack (`player::tempo_from_hits`, from the swing pass of
//! `hit_boxes::land`), `strike` adds the stacks' percent to the attack speed under the run's cap
//! (`player::tempo_speed_pct`), and the stacks end on a combo reset, a smash press, a hit taken,
//! a down or another style (`player::tempo_lose`, `combo_reset`). `PlayerView.tempo` and the two
//! tempo events are not state. A run whose styles name no combo tempo keeps the field at zero,
//! one bool test each. With `super_armour_at_full`, full stacks in a held hold are super armour
//! (`player::tempo_armoured`), the one armour that shelters the stacks from a hit.
//!
//! The giant trigger a style grants (`World::trigger_of`, the `giant_trigger` style kind) reads
//! and writes only `PlayerCombat::giant_trigger`, the ticks the timed state has left. A full-gauge
//! smash starts it after its own attack began (`player::spend_gauge`, `trigger_start`), `update`
//! counts it down, `strike` adds its range and smash percents when an attack starts, and
//! `super_armour` gains its condition; a down or another style ends it. The run rules' rally
//! (`Tuning.rally`) reads and writes `PlayerCombat::rally` and `rally_ticks`: a hit taken leaves
//! part of its hit points as rally (`player::rally_from_hit`), a player's own combo hit on a foe
//! wins part of its damage back (`rally_win_back`, from `collide::damage`), and the ticks or a
//! down lose the rest. `PlayerView.trigger`, `.rally` and the five events are not state. A run
//! that names neither keeps the three fields at zero, one bool test each.
//!
//! The breaker a style grants (`World::breaker_of`, the `breaker` style kind, named at the style's
//! own level and at later levels, the effects stacking in path order) reads and writes no state:
//! its factors scale a stagger fill (`stun::fill`), a combo hit's poise damage (`flinch_by`) and a
//! smash step's damage on a boss part, and its burst hurts and controls the enemies round a
//! target the player's fill staggers, in the call that staggers it (`player::stagger_burst`). The
//! style views and the `StaggerBurst` event are not state; a run whose styles name no breaker pays
//! one bool test at each place.
//!
//! Both are visible without being state: `PlayerView.gauge` and `PlayerView.perfect_evade`
//! read the timers and the gauge, and the events `GaugeFilled`, `GaugeSpent`, `PerfectEvade`
//! and `OrbBroken.player` say what moved. Events are not hashed, saved or replayed.
//!
//! The stage skill (`skill`) acts in `player_update` on a press, after the evade and
//! before the combo; its black hole pulls in `skill_pull`, and its shield takes from the main
//! character's damage in `collide`.
//!
//! Orbits (`orbit`) act in `orbits`: their hit boxes land in `collide`. A run holds at
//! most `MAX_ORBITS` orbits of `MAX_ORBS` orbs.
//!
//! Range rewards widen the hit boxes of the combo steps started after they are taken,
//! adding up over the picks (`player`).
//!
//! Type bonus rewards add up for each damage type, a group's to each type of its
//! group (`stars`). An attack the main character starts fixes those sums, each held to the
//! match-up file's `max_type_bonus_pct`; each of its phases of a type raises its damage by
//! that type's, after atk-up (`attacks::phase_damage`), and the match-up multiplies it at the
//! hit.
//!
//! On-hit runes (`runes`): each cooldown counts down in `runes_tick`; a combo step's
//! hit in `collide` fires each one held and ready, an attack in its own slot whose hit boxes
//! stay on the point hit and land from the next tick, and sets its cooldown. Its hits are no
//! combo step's. A thrown rune's hit boxes (boomerang) fly in `runes_fly`: out to the point hit,
//! then back to the main character. Homing missiles' hit boxes steer in `runes_steer`, after
//! the push-apart and the pull; one that hits anything in `collide` is gone once the hit boxes
//! have landed.
//!
//! Damage-over-time runes (`runes`): a combo step's hit in `collide` that leaves its
//! target standing puts each one's status on it, bleed's `bleed`, poison's `poison`, burn's
//! `burn`, or a stack more up to its maximum, its duration starting again. Statuses of that
//! kind do not run out in `statuses_and_aggro`: at the start of `collide`, before any hit
//! lands, each counts down to its next tick and deals its damage on it, then its duration runs
//! down (`entity::tick_dots`); a tick without one on anyone skips that pass.
//!
//! Not yet in this version: summons by companions.
//! Rule conditions, selectors and actions beyond the ones in `agents` and `select` never fire.
//!
//! Dynamic budgets (ADR-SVC-1): at most `World::enemy_cap` enemies (the crowd target and the
//! pack's headroom, 260 for the bundled pack, derived when the run is built and at most
//! `limits::MAX_ENEMY_CAP`, DS103), the stage's elite cap of
//! them elites (at most `MAX_ELITES_ALIVE`), `MAX_ZONES_ALIVE` zones, `MAX_ORBS_ALIVE` orbs,
//! `MAX_ATTACK_TASKS` attacks at once and
//! `MAX_EMITTER_STEPS_PER_TICK` steps of them per tick. A spawn past a cap is dropped, in
//! spawn order, and a wave spawn that is dropped still counts as spawned; attacks past the
//! step budget wait for the next tick.

mod act;
mod agents;
mod attack_table;
mod attacks;
mod boss;
#[cfg(test)]
mod breaker_tests;
mod build;
#[cfg(test)]
mod build_pin_tests;
mod drops;
mod world;
pub(crate) use attacks::phase;
pub(crate) use world::{
    AttackIndex, BossIndex, DefIndex, HordeSpec, ProgramIndex, RangedPool, World,
};
#[cfg(test)]
pub(crate) mod before;
#[cfg(test)]
mod codec_fields_tests;
mod combat;
#[cfg(test)]
mod counter_tests;
mod damage;
mod dash;
mod entity;
pub(crate) mod equipment;
mod expertise;
#[cfg(test)]
mod frozen_tests;
#[cfg(test)]
mod gauge_tests;
mod grid;
#[cfg(test)]
mod hash_fields_tests;
#[cfg(test)]
mod hash_layout_tests;
mod heal;
mod hit_boxes;
mod horde;
mod leash;
#[cfg(test)]
mod leash_tests;
mod levels;
#[cfg(test)]
mod levels_tests;
mod loadout;
mod offer_rating;
#[cfg(test)]
mod rally_tests;
#[cfg(test)]
mod repeat_tests;
mod slots;
#[cfg(test)]
mod spirit_tests;
#[cfg(test)]
mod tempo_tests;
#[cfg(test)]
mod trigger_tests;
#[cfg(test)]
mod try_styles_tests;
pub(crate) use expertise::TreeError;
pub(crate) use offer_rating::swap_ratings;
pub(crate) use slots::{
    Misfit, SlotError, holds_right_kinds, misfit, runes_match_slots, same_layout,
};
pub(crate) use starlight::StarlightError;
#[cfg(test)]
mod damage_tests;
#[cfg(test)]
mod drops_tests;
#[cfg(test)]
mod equip_support;
#[cfg(test)]
mod magnet_tests;
mod orbit;
#[cfg(test)]
mod party_act_tests;
#[cfg(test)]
mod party_equip_tests;
#[cfg(test)]
mod party_fix_tests;
#[cfg(test)]
mod perfect_evade_tests;
mod player;
#[cfg(test)]
mod players_pin_tests;
#[cfg(test)]
mod players_tests;
mod push;
#[cfg(test)]
mod rejoin_tests;
#[cfg(test)]
mod route_tests;
mod runes;
mod schedule;
mod scratch;
mod skill;
#[cfg(test)]
mod slots_tests;
pub(crate) mod snapshot;
#[cfg(test)]
mod spawn_edge_tests;
mod stardust;
pub(crate) mod starlight;
#[cfg(test)]
mod starlight_tests;
mod stars;
pub(crate) mod state;
mod state_codec;
mod state_hash;
mod stun;
#[cfg(test)]
mod tether_tests;
#[cfg(test)]
mod tree_stats_tests;
mod zones;

pub(crate) use player::{auto_charge_view, charge_view, combo_view, hold_view};
mod collide;
mod item_actions;
mod kills;
mod player_hits;
mod select;
mod setup;
mod spawn;
mod view;

use crate::attack::DamageType;
use crate::engine::{Load, Outcome};
use crate::event::{DomainEvent, HitKind};
use crate::expertise::Expertise;
use crate::fixed::Point;
use crate::hash::{StableHash, StateHasher};
use crate::id::EntityId;
use crate::input::Input;
use crate::rewards::Rewards;
use crate::rune::Rune;
use crate::sim::state::{MAX_PLAYERS, PlayerIdx, Players};
use crate::sim::state::{Smash, State};
use crate::validate::limits::DAMAGE_SCALE;

pub(crate) use collide::{beam_to, pickup_pct_of};
use collide::{collide, damage, enemy_grid, move_orbs, overlaps, pickup_pct, widened};
pub(crate) use drops::DropTable;
pub(crate) use expertise::styles_view as expertise_styles_view;
pub(crate) use expertise::{check_tree, tree_ready_view, weapon_stats_view};
pub(crate) use item_actions::{
    check_craft, check_equip, check_extract, check_tree_pick, craft_action, equip_action,
    extract_action, tree_pick_action,
};
use kills::{
    buffer_of, damageable, death_event, died_event, hitter, killer_of, kind_of, knock_enemy,
    profile, pull_enemy, record_kill,
};
pub(crate) use levels::{set_policy, xp_next};
pub(crate) use loadout::{Loadout, apply as apply_loadout};
use player_hits::{aided, hit_companions_side, hit_players_side, hurt_player, near_downed};
pub(crate) use player_hits::{aura_percent, aura_sources, tick_down_and_revive};
pub(crate) use scratch::Scratch;
pub(crate) use setup::{
    Arrival, RunDifficulty, RunPool, announced_patterns, boss_state, events_room, initial_state,
    reserve, scale_hp,
};
#[cfg(test)]
use spawn::{any_point, any_total};
use spawn::{edge_point, rect_sides, spawn_enemy, spawn_waves, usable_side};
pub(crate) use stars::constellation_views;
#[cfg(test)]
use view::view_centre;
pub(crate) use view::{Area, VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH};

/// The most enemies the leash moves in a tick.
pub(crate) const MAX_LEASH_MOVES_PER_TICK: u32 = 16;
/// Zones alive at once (core Z1, a placeholder cap pending ADR-SIM-5); a zone placed over it is
/// dropped, as an orb over `MAX_ORBS_ALIVE` is.
pub(crate) const MAX_ZONES_ALIVE: u32 = 128;
/// Regular enemies attacking at range at once (core Z2, DS10): a `ranged` run holds a token while
/// it is in its wind-up or active phase. The tokens are the two pools of `rules.json`
/// (`ranged_zone_tokens`, `ranged_line_tokens`), which sum to at most this (ADR-SIM-5's cap); they go to the nearest ready enemies in view (RA1).
pub(crate) const MAX_RANGED_ATTACKERS: u32 = 4;

/// Orbs alive per attacker and in all (core Z2, DS11, tuning rows); an orb over either cap is
/// dropped in node order.
pub(crate) const MAX_ORBS_PER_OWNER: u32 = 8;
pub(crate) const MAX_ORBS_ALIVE: u32 = 32;
/// Attacks being carried out at once (ADR-SIM-4 tier 3), and the node steps they may take in a
/// tick between them. An attack that would take more waits for the next tick.
pub(crate) const MAX_ATTACK_TASKS: u32 = 256;
pub(crate) const MAX_EMITTER_STEPS_PER_TICK: u32 = 4096;

/// The most aggro anyone holds (ADR-SIM-4; an overflow bound, also the validator's bound of
/// `AggroAbove`). What raises it and how fast it fades are `rules.json`'s `aggro_*` fields.
const MAX_AGGRO: u32 = 1_000_000 * DAMAGE_SCALE;

#[allow(clippy::too_many_arguments)]
pub(crate) fn step(
    _version: u32,
    stage: &World,
    rewards: &Rewards,
    expertise: &Expertise,
    s: &mut State,
    inputs: &[Input; MAX_PLAYERS as usize],
    events: &mut Vec<DomainEvent>,
    load: &mut Load,
) {
    if s.outcome != Outcome::Running {
        return;
    }
    let mut ctx = schedule::Ctx::new(stage, rewards, expertise, s, inputs, events, load);
    for (_, run) in schedule::TICK {
        run(&mut ctx);
    }
    load.refused_spawns = std::mem::take(&mut s.scratch.refused_spawns);
    load.held_spawns = std::mem::take(&mut s.scratch.held_spawns);
    debug_assert!(
        entity::enemies_in_id_order(s),
        "enemies out of entity-ID order"
    );
}

fn count<T>(items: &[T]) -> u32 {
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

/// What a hit needs to know about the orb that made it.
#[derive(Clone, Copy)]
struct Shot {
    at: Point,
    owner: EntityId,
    friendly: bool,
    damage: u32,
    /// Its type; `None` is neutral.
    damage_type: Option<DamageType>,
    breaks_charge: bool,
    /// What a smash does to the enemy it hits, for a combo part's hit.
    smash: Option<Smash>,
    /// A combo step's hit, which adds expertise points.
    combo: bool,
    /// The rune that dealt it, which the `Hit` event reports.
    rune: Option<Rune>,
    /// A hit the combat rewards make (a kill burst), which neither crits nor echoes.
    chain: bool,
}

/// Where the attack of a hit is found.
#[derive(Clone, Copy)]
enum Culprit {
    /// A run's hit box: the run knows its attack.
    Run(AttackIndex),
    /// What a program placed (an orb, a zone or a beam): the attack is the one that holds it.
    Program(ProgramIndex),
}

/// What hurt the main character, for the `PlayerHit` event: the kind, and where its attack is.
#[derive(Clone, Copy)]
struct Blame {
    kind: HitKind,
    by: Culprit,
}

impl Blame {
    /// What a program placed: an orb, a zone or a beam.
    fn of_program(kind: HitKind, program: ProgramIndex) -> Blame {
        Blame {
            kind,
            by: Culprit::Program(program),
        }
    }
}

/// The main character's death wins over a clear on the same tick. In a party, the run continues
/// while any player is up (ADR-SIM-7).
fn decide_outcome(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let failed = if s.players.len() == 1 {
        s.players[PlayerIdx::SOLO].hp == 0
    } else {
        !s.players.any_up()
    };
    if failed {
        s.outcome = Outcome::Failed;
        events.push(DomainEvent::StageFailed);
        return;
    }
    let cleared = match &s.boss {
        // A run with a horde ends in a clear only by the final boss's death: a mid-boss's is
        // not the end, and neither is an empty field.
        _ if s.horde.is_some() => horde::final_boss_dead(s),
        Some(b) => b.hp == 0,
        // A stage boss that has not come yet is not a cleared field.
        None if stage.boss.is_some() => false,
        None => {
            s.enemies.is_empty()
                && stage
                    .waves
                    .iter()
                    .zip(&s.spawned)
                    .all(|(w, spawned)| *spawned >= w.count)
        }
    };
    if cleared {
        s.outcome = Outcome::Cleared;
        events.push(DomainEvent::StageCleared);
    } else if s.tick >= stage.length_ticks {
        s.outcome = Outcome::Failed;
        events.push(DomainEvent::StageFailed);
    }
}

/// The state hash (ADR-SIM-3 point 2): FNV-1a 64 over the `StableHash` encoding of the whole
/// state, which `state_hash.rs` writes.
pub(crate) fn state_hash(s: &State) -> u64 {
    let mut h = StateHasher::new();
    s.stable_hash(&mut h);
    h.finish()
}
