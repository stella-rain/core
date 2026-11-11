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
mod codec_fields_tests;
mod combat;
mod damage;
mod dash;
mod entity;
pub(crate) mod equipment;
mod expertise;
#[cfg(test)]
mod frozen_tests;
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
mod slots;
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
mod orbit;
#[cfg(test)]
mod party_act_tests;
#[cfg(test)]
mod party_equip_tests;
#[cfg(test)]
mod party_fix_tests;
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

use crate::attack::DamageType;
use crate::engine::{Load, Outcome};
use crate::event::{DomainEvent, HitKind};
use crate::expertise::Expertise;
use crate::fixed::{Fx, Point};
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

/// The most enemies the leash moves in a tick.
pub(crate) const MAX_LEASH_MOVES_PER_TICK: u32 = 16;
/// Zones alive at once (core Z1, a placeholder cap pending ADR-SIM-5); a zone placed over it is
/// dropped, as an orb over `MAX_ORBS_ALIVE` is.
pub(crate) const MAX_ZONES_ALIVE: u32 = 128;
/// Regular enemies attacking at range at once (core Z2, DS10): a `ranged` run holds a token while
/// it is in its wind-up or active phase. The tokens are the two pools of `rules.json`
/// (`ranged_zone_tokens`, `ranged_line_tokens`), which sum to at most this (ADR-SIM-5's cap); they go to the nearest ready enemies in view (RA1).
pub(crate) const MAX_RANGED_ATTACKERS: u32 = 4;
/// The view rectangle (RA1, DS82): the widest supported aspect, 21:9, of the 360 px design
/// height, 840 by 360 px, never the device's screen. It is the camera's rectangle: centred on the
/// main character but held inside the map (`view_centre`). An enemy starts a ranged attack
/// only from inside it, edges included. Tuning values.
pub(crate) const VIEW_HALF_WIDTH: Fx = Fx::px(crate::limits::VIEW_HALF_WIDTH);
pub(crate) const VIEW_HALF_HEIGHT: Fx = Fx::px(crate::limits::VIEW_HALF_HEIGHT);

/// The centre of the view rectangle for a main character at `at` on `map`: the camera's
/// centre, which follows the main character but stops where the view would leave the map. It
/// mirrors `camera_follow.gd` (`_axis`) on each axis: a map no larger than the view gives
/// `map / 2` (integer division, so an odd number of units loses half a unit, 1/512 px), else
/// the main character's coordinate held between the half view and `map - half view`.
pub(crate) fn view_centre(map: &crate::stage::Map, at: Point) -> Point {
    let axis = |target: Fx, side: Fx, half: Fx| {
        let (target, side, half) = (i64::from(target.0), i64::from(side.0), i64::from(half.0));
        let centre = if side <= 2 * half {
            side / 2
        } else {
            target.clamp(half, side - half)
        };
        // Within `side`, which fits `i32`.
        Fx(i32::try_from(centre).unwrap_or(i32::MAX))
    };
    Point {
        x: axis(at.x, map.width, VIEW_HALF_WIDTH),
        y: axis(at.y, map.height, VIEW_HALF_HEIGHT),
    }
}

/// The bounding box of the view centres of the players the world reads (`Players::targets`):
/// its centre and half extents. One player gives that player's view centre and half extents 0,
/// so every use equals the single-player rule (ADR-SIM-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) centre: Point,
    pub(crate) half_x: i64,
    pub(crate) half_y: i64,
}

impl Area {
    pub(crate) fn of_view(map: &crate::stage::Map, players: &Players) -> Area {
        let mask = players.targets();
        // One player (nearly every run): its view centre, no box.
        if mask & mask.wrapping_sub(1) == 0 {
            let me = Players::slots(mask).next().unwrap_or(PlayerIdx::SOLO);
            return Area::at(view_centre(map, players[me].at));
        }
        let (mut lo, mut hi) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
        for me in Players::slots(mask) {
            let c = view_centre(map, players[me].at);
            lo = (lo.0.min(c.x.0), lo.1.min(c.y.0));
            hi = (hi.0.max(c.x.0), hi.1.max(c.y.0));
        }
        let mid = |lo: i32, hi: i32| lo + (hi - lo) / 2;
        let centre = Point {
            x: Fx(mid(lo.0, hi.0)),
            y: Fx(mid(lo.1, hi.1)),
        };
        Area {
            centre,
            half_x: i64::from(hi.0 - centre.x.0),
            half_y: i64::from(hi.1 - centre.y.0),
        }
    }

    /// The area of a single point: no extent.
    pub(crate) fn at(centre: Point) -> Area {
        Area {
            centre,
            half_x: 0,
            half_y: 0,
        }
    }
}

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
