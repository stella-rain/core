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
mod kills;
mod player_hits;
mod select;

use crate::attack::{DamageType, MatchUp};
use crate::behaviour::BossCall;
use crate::difficulties::scale_pct;
use crate::engine::{Load, Outcome};
use crate::event::{DomainEvent, HitKind};
use crate::expertise::Expertise;
use crate::fixed::{Fx, Point};
use crate::hash::{StableHash, StateHasher};
use crate::id::EntityId;
use crate::input::Input;
use crate::kinds::KindTable;
use crate::rewards::{EquipmentRules, Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::rune::Rune;
use crate::sim::state::{
    AgentKey, AgentSource, BossState, Career, Dir, Offers, PartState, PlayerState, Progress, Smash,
    StarField, StatRule, State, Streams,
};
use crate::sim::state::{MAX_PLAYERS, PlayerIdx, Players};
use crate::stage::{Side, ViewEdge};
use crate::trig;
use crate::validate::limits::DAMAGE_SCALE;

pub(crate) use drops::DropTable;
use entity::{Who, scale_damage, status_pct};
pub(crate) use expertise::styles_view as expertise_styles_view;
pub(crate) use expertise::{check_tree, tree_ready_view, weapon_stats_view};
use grid::Grid;
use kills::{
    buffer_of, damageable, death_event, died_event, hitter, killer_of, kind_of, knock_enemy,
    profile, pull_enemy, record_kill,
};
pub(crate) use levels::{set_policy, xp_next};
pub(crate) use loadout::{Loadout, apply as apply_loadout};
use player_hits::{aided, hit_companions_side, hit_players_side, hurt_player, near_downed};
pub(crate) use player_hits::{aura_percent, aura_sources, tick_down_and_revive};
pub(crate) use scratch::Scratch;
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

/// The radius the enemy grid is built for (`Grid::rebuild`'s probe radius); it sets the cell side
/// with the largest enemy, so it stays what the friendly orbs' radius was.
const GRID_PROBE_RADIUS: i32 = Fx::px(2).0;

/// The most aggro anyone holds (ADR-SIM-4; an overflow bound, also the validator's bound of
/// `AggroAbove`). What raises it and how fast it fades are `rules.json`'s `aggro_*` fields.
const MAX_AGGRO: u32 = 1_000_000 * DAMAGE_SCALE;

/// What a run brings to its offers: the rewards of its pool, by their place in the rewards file
/// (ADR-REPL-5).
pub(crate) struct RunPool {
    pub rewards: Vec<u16>,
    pub difficulty: RunDifficulty,
}

/// `hp` scaled by `pct`: never to 0 from a positive value, which would be dead at
/// its spawn.
pub(crate) fn scale_hp(hp: u32, pct: u32) -> u32 {
    match hp {
        0 => 0,
        _ => scale_pct(hp, pct).max(1),
    }
}

/// What a run takes from its difficulty: the percents its enemies' hit points and
/// attacks' damage are scaled by, and the range of rarities its stars and offers may have.
/// Fixed for the run, part of the setup, and hashed with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunDifficulty {
    pub hp_pct: u32,
    pub damage_pct: u32,
    pub rarity: (Rarity, Rarity),
}

impl RunDifficulty {
    /// A stage run alone, without a setup: nothing scaled and no rarity left out.
    pub const NONE: RunDifficulty = RunDifficulty {
        hp_pct: 100,
        damage_pct: 100,
        rarity: (Rarity::Common, Rarity::Supernova),
    };
}

/// The state before the first tick, its generator seeded with `seed`: the stage's own for a
/// stage, the setup's for a run (ADR-REPL-5).
pub(crate) fn initial_state(
    stage: &World,
    content: &RunContent,
    pool: RunPool,
    seed: u64,
    kinds: KindTable,
) -> State {
    let mut state = unreserved_state(stage, content, pool, seed, kinds);
    // The weapon's base set at level 1, for slot 0; a party member or a joiner copies the career.
    expertise::start_set(stage, content.expertise(), &mut state, PlayerIdx::SOLO);
    reserve(stage, &mut state);
    state
}

/// Makes room, once, for the most the run's per-enemy lists can hold (DIR1, ADR-SIM-5 point 4): the cap on enemies alive, and the other caps, so that no tick grows a buffer.
/// Called when the run is built and again for a clone of its state, because a cloned `Vec` has
/// only its length. The scratch buffers are `Scratch::reserve`'s.
pub(crate) fn reserve(stage: &World, s: &mut State) {
    let cap = stage.enemy_cap as usize;
    let map = stage.map();
    s.scratch.reserve(cap, map.width.0, map.height.0);
    scratch::reserve_to(&mut s.enemies, cap);
    s.move_index
        .reserve(cap + crate::validate::limits::MAX_COMPANIONS + 1);
    scratch::reserve_to(&mut s.tasks, MAX_ATTACK_TASKS as usize);
    scratch::reserve_to(
        &mut s.runs,
        cap + crate::validate::limits::MAX_COMPANIONS + 1,
    );
    scratch::reserve_to(&mut s.ranged_cooling, MAX_RANGED_ATTACKERS as usize);
    scratch::reserve_to(&mut s.orbs, MAX_ORBS_ALIVE as usize);
    scratch::reserve_to(&mut s.zones, MAX_ZONES_ALIVE as usize);
    if let (Some(cfg), Some(f)) = (&stage.stars, s.stars.as_mut()) {
        let stars = usize::from(cfg.cap.min(crate::validate::limits::MAX_STARS_ON_MAP));
        scratch::reserve_to(&mut f.on_map, stars);
    }
    if let (Some(cfg), Some(pickups)) = (&stage.heal, s.heal_pickups.as_mut()) {
        scratch::reserve_to(pickups, usize::from(cfg.cap));
    }
    // Each present slot's inventory has room for its bound, so a drop allocates nothing
    // (ADR-SIM-5 points 7 and 8).
    for me in s.players.indices() {
        let inventory = &mut s.players.career_mut(me).equipment.inventory;
        scratch::reserve_to(inventory, crate::limits::MAX_INVENTORY);
    }
}

/// Events a tick reserves room for per enemy alive (hits, kills, spawns, bursts).
const EVENTS_PER_ENEMY: usize = 4;
/// Events a tick reserves room for beyond those.
const EVENTS_SPARE: usize = 64;

/// How far apart, in degrees, the orbiting companions start.
const COMPANION_SPACING_DEGREES: i32 = 90;

/// The room for a tick's events (DIR1): a few per enemy alive (hits, kills, spawns, bursts) and
/// a few more. A placeholder bound: a tick with more grows the list once, and `tests/alloc.rs`
/// checks the runs it measures stay under it.
pub(crate) fn events_room(stage: &World) -> usize {
    stage.enemy_cap as usize * EVENTS_PER_ENEMY + EVENTS_SPARE
}

fn unreserved_state(
    stage: &World,
    content: &RunContent,
    pool: RunPool,
    seed: u64,
    kinds: KindTable,
) -> State {
    let run_stats = content.run_stats();
    // Slots 0 to 2 are reserved for the players whether or not a slot is in the party
    // (ADR-SIM-7); the boss and the companions come after them.
    let mut next_id = u32::from(MAX_PLAYERS);
    // A boss with an arrival tick after the first is built on that tick (`boss::arrive`).
    let boss_id = stage
        .boss
        .as_ref()
        .filter(|b| boss::on_field_at_start(b))
        .map(|_| {
            let id = EntityId(next_id);
            next_id += 1;
            id
        });
    let start = stage.map().start();
    let companions = stage
        .companions
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let id = EntityId(next_id);
            next_id += 1;
            let key = AgentKey {
                source: AgentSource::Companion,
                index: u32::try_from(i).unwrap_or(u32::MAX),
                rule: 0,
            };
            let index = stage.defs.of_key(key);
            // Orbiting companions start a quarter turn apart.
            let angle =
                i32::try_from(i).unwrap_or(0) * COMPANION_SPACING_DEGREES * trig::PER_DEGREE;
            agents::new_agent(id, index, stage.defs.get(index), key, start, angle)
        })
        .collect();
    // The boss's parts come after the companions in the order of entity IDs. Of these, the crit roll
    // and the push-apart angle read a number: they read `State::roll_key`, the number without
    // the empty player slots (ADR-SIM-7).
    let boss = boss_id.map(|id| {
        let arrival = Arrival {
            at: stage.boss(BossIndex(0)).boss.spawn,
            origin: Point::default(),
            no_call: false,
            mid_pct: None,
        };
        boss_state(
            stage,
            BossIndex(0),
            id,
            &mut next_id,
            pool.difficulty.hp_pct,
            arrival,
            &kinds,
        )
    });
    let progress = stage.levels.as_ref().map(|_| Progress {
        level: 1,
        xp: 0,
        stats: run_stats
            .stats()
            .iter()
            .map(|s| StatRule {
                id: s.id.clone(),
                effect: s.effect,
                step: s.step,
                limit: s.limit,
            })
            .collect(),
        priority: (0..run_stats.stats().len())
            .map(|i| u8::try_from(i).unwrap_or(u8::MAX))
            .collect(),
        spent: vec![0; run_stats.stats().len()],
        totals: [0; StatEffect::ALL.len()],
        set_tenths: [0; StatEffect::ALL.len()],
    });
    let offers = stage.stars.as_ref().map(|_| Offers {
        set_counts: vec![0; content.constellations().sets().len()],
    });
    State {
        scratch: Scratch::default(),
        tick: 0,
        rng: Streams::new(seed),
        outcome: Outcome::Running,
        players: Players::solo(
            PlayerState {
                at: start,
                hp: stage.player.hp,
                max_hp: stage.player.hp,
                defence: stage.player.defence,
                invulnerable: 0,
                // Up the map unless the map's facing says otherwise (core#147).
                facing: stage
                    .map()
                    .facing
                    .map_or(Dir::UP, |angle| attacks::rotate(Dir::DOWN, angle)),
                target: None,
                // The validator holds it to 1..=3; the clamp keeps an unvalidated stage there too.
                evade_charges: stage
                    .player
                    .evade
                    .charges
                    .clamp(1, crate::validate::limits::MAX_EVADE_CHARGES),
                evade_recovering: Vec::new(),
                held: 0,
                combo: player::initial_combo(stage),
                dash: None,
                skill: None,
                pull: None,
                shield: None,
                orbits: Vec::new(),
                range_pct: 0,
                rune_haste_pct: 0,
                pickup_radius_pct: 0,
                skill_cooldown_pct: 0,
                kill_heal: 0,
                kill_heal_every: 0,
                kills_counted: 0,
                crit_chance_pct: 0,
                crit_damage_pct: 0,
                echo_hit_pct: 0,
                burst_pct: 0,
                burst_radius: 0,
                burst_depth: 0,
                burst_depth_extra: 0,
                type_bonus_pct: [0; DamageType::ALL.len()],
                runes: Vec::new(),
                aggro: 0,
                statuses: Default::default(),
                down_ticks: 0,
                revive_hold: 0,
                out: false,
            },
            Career::new(
                seed,
                // The run's first player; the other slots are built by the party builder (a test
                // here, recording parties later).
                PlayerIdx::SOLO,
                progress,
                offers,
                expertise::initial(stage, content.expertise()),
                stage.stardust.as_ref().map(|_| 0),
                equipment::Equipment::default(),
            ),
        ),
        boss,
        companions,
        enemies: Vec::new(),
        tasks: Vec::new(),
        runs: Vec::new(),
        move_index: crate::sim::state::MoveIndex::default(),
        ranged_cooling: Vec::new(),
        orbs: Vec::new(),
        zones: Vec::new(),
        spawned: vec![0; stage.waves.len()],
        next_id,
        next_orb_group: 0,
        stars: stage.stars.as_ref().map(|_| StarField {
            misses: 0,
            on_map: Vec::new(),
            rarity: pool.difficulty.rarity,
            rune_pool: runes::FULL_POOL,
            pool: pool.rewards,
        }),
        heal_pickups: stage.heal.as_ref().map(|_| Vec::new()),
        match_ups: *content.match_ups(),
        runes: std::sync::Arc::new(content.runes().clone()),
        stun: run_stats.stun(),
        max_attack_speed_pct: run_stats.max_attack_speed_pct(),
        enemy_attack_damage: run_stats.enemy_attack_damage(),
        max_rune_haste_pct: content.rewards().max_rune_haste_pct(),
        horde: None,
        hp_pct: pool.difficulty.hp_pct,
        damage_pct: pool.difficulty.damage_pct,
        crit_seed: seed,
        echoes: Vec::new(),
        kinds: std::sync::Arc::new(kinds),
    }
}

/// The big patterns the boss has announced, for the snapshot; an enemy's are not.
pub(crate) fn announced_patterns(stage: &World, s: &State) -> Vec<crate::snapshot::PatternView> {
    boss::announced(stage, s)
}

/// What differs between the arrivals of one boss definition: where it stands, how far its
/// `move_to` targets are moved, whether it calls elites, and the percent its hit points and its
/// parts' are scaled by first (a `time_limit` mid-boss's, core#336).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Arrival {
    pub at: Point,
    pub origin: Point,
    pub no_call: bool,
    pub mid_pct: Option<u32>,
}

/// The boss before its first tick, its parts taking the entity IDs from `next_id` on. The
/// first phase begins on the first tick (see `boss::act`).
/// The boss's state at its spawn, its hit points and its parts' scaled by `hp_pct`.
pub(crate) fn boss_state(
    stage: &World,
    boss: BossIndex,
    id: EntityId,
    next_id: &mut u32,
    hp_pct: u32,
    arrival: Arrival,
    kinds: &KindTable,
) -> BossState {
    let def = stage.boss(boss);
    let b = &def.boss;
    let hp = |hp: u32| {
        let hp = arrival.mid_pct.map_or(hp, |pct| scale_hp(hp, pct));
        scale_hp(hp, hp_pct)
    };
    BossState {
        id,
        boss,
        origin: arrival.origin,
        no_call: arrival.no_call,
        kind: def.kind,
        at: arrival.at,
        hp: hp(b.hp),
        max_hp: hp(b.hp),
        defence: b.defence,
        statuses: Default::default(),
        phase: 0,
        phase_age: 0,
        invulnerable: 0,
        step: 0,
        wait_left: 0,
        order: None,
        rules: Default::default(),
        // A boss that calls no elites hashes nothing for it, a no-call boss too.
        elites_called: b.call_elites.filter(|_| !arrival.no_call).map(|_| false),
        age: 0,
        parts: b
            .parts
            .iter()
            .map(|p| {
                let part_id = EntityId(*next_id);
                *next_id += 1;
                PartState {
                    id: part_id,
                    name: p.id.clone(),
                    kind: kinds.of(&p.id),
                    hp: hp(p.hp),
                    max_hp: hp(p.hp),
                    defence: p.defence,
                    offset: p.offset,
                    radius: p.radius.0,
                    statuses: Default::default(),
                }
            })
            .collect(),
    }
}

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

/// Whether the inventory item `item` could go into `slot` of `me`, as an equip or a replace
/// (`slots::check`). Read-only.
pub(crate) fn check_equip(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
    item: u32,
    slot: u32,
    replace: bool,
) -> Result<(), SlotError> {
    slots::check(
        rules,
        rewards,
        &s.players.career(me).equipment,
        item,
        slot,
        replace,
    )
}

/// Equips (`replace` false) or replaces: the item `item` into `slot` of `me`, or a refusal that
/// changes nothing. Made between ticks, like a pick; no event, no draw.
pub(crate) fn equip_action(
    stage: &World,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    item: u32,
    slot: u32,
    replace: bool,
) -> Result<(), SlotError> {
    let (player, career) = s.players.both_mut(me);
    match replace {
        true => slots::replace(stage, rewards, player, career, item, slot),
        false => slots::equip(stage, rewards, player, career, item, slot),
    }
}

/// Whether `me` could take choice `option` of the next tree pick (`expertise::check_tree_pick`).
/// Read-only.
pub(crate) fn check_tree_pick(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
    option: u32,
) -> Result<(), TreeError> {
    expertise::check_tree_pick(stage, expertise, s, me, option)
}

/// Takes choice `option` of the next tree pick for `me`, or a refusal that changes nothing. Made
/// between ticks; no event, no draw.
pub(crate) fn tree_pick_action(
    stage: &World,
    expertise: &Expertise,
    s: &mut State,
    me: PlayerIdx,
    option: u32,
) -> Result<(), TreeError> {
    expertise::tree_pick(stage, expertise, s, me, option)
}

/// The setup's star pool of `s`, empty when the run has no star field.
fn star_pool(s: &State) -> &[u16] {
    s.stars.as_ref().map_or(&[], |f| &f.pool)
}

/// Whether `me` could extract inventory item `item` (`starlight::check_extract`). Read-only.
pub(crate) fn check_extract(s: &State, me: PlayerIdx, item: u32) -> Result<(), StarlightError> {
    starlight::check_extract(&s.players.career(me).equipment, item)
}

/// Extracts inventory item `item` of `me` into Starlight, or a refusal that changes nothing.
/// Made between ticks; no event, no draw.
pub(crate) fn extract_action(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    item: u32,
) -> Result<(), StarlightError> {
    starlight::extract(
        rules,
        rewards,
        &mut s.players.career_mut(me).equipment,
        item,
    )
}

/// Whether `me` could craft the star at index `star` of the star pool. Read-only.
pub(crate) fn check_craft(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
    star: u32,
) -> Result<(), StarlightError> {
    starlight::check_craft(
        rules,
        rewards,
        star_pool(s),
        &s.players.career(me).equipment,
        star,
    )
}

/// Crafts the star at index `star` of the star pool for `me`, or a refusal that changes nothing.
/// Made between ticks; no event, no draw.
pub(crate) fn craft_action(
    rules: &EquipmentRules,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    star: u32,
) -> Result<(), StarlightError> {
    let State { stars, players, .. } = s;
    let pool = stars.as_ref().map_or(&[][..], |f| &f.pool);
    starlight::craft(
        rules,
        rewards,
        pool,
        &mut players.career_mut(me).equipment,
        star,
    )
}

fn count<T>(items: &[T]) -> u32 {
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

fn spawn_waves(stage: &World, s: &mut State) {
    if stage.director {
        return spawn_waves_directed(stage, s);
    }
    // The party's box, built on the first due wave of the tick: players do not move meanwhile.
    let mut area = None;
    for (i, wave) in stage.waves.iter().enumerate() {
        let spawned = s.spawned[i];
        if spawned >= wave.count {
            continue;
        }
        let due = u64::from(wave.at_tick) + u64::from(spawned) * u64::from(wave.every_ticks);
        if u64::from(s.tick) < due {
            continue;
        }
        spawn_wave(stage, s, i, &mut area);
    }
}

/// The spawn director (DIR1, DS103): the waves of a horde run, held at the crowd target.
///
/// A wave is due when its tick comes, as it is without the director, plus what it has been held
/// so far (`Horde::held`): a held spawn is not dropped and not queued, its schedule slides. The
/// room is the target less the enemies alive, taken once. If every due wave fits, all spawn in
/// index order, drawing in that order as they do without the director, so a run under the target
/// is unchanged. If not, the waves held longest spawn (ties to the lower index) in index order,
/// and every other due wave is held one more tick and counted in `Load::held_spawns`. A held
/// spawn draws no angle and does not count as spawned. The headroom between the target and the
/// cap stays for the specials, which the cap alone limits.
fn spawn_waves_directed(stage: &World, s: &mut State) {
    use crate::time_limit::MAX_WAVES;
    let mut due = [0usize; MAX_WAVES];
    let mut n = 0;
    if let Some(horde) = s.horde.as_ref() {
        for (i, wave) in stage.waves.iter().enumerate() {
            let spawned = s.spawned[i];
            if spawned >= wave.count || n == MAX_WAVES {
                continue;
            }
            let slid = u64::from(horde.held.get(i).copied().unwrap_or(0));
            let at = u64::from(wave.at_tick) + u64::from(spawned) * u64::from(wave.every_ticks);
            if u64::from(s.tick) >= at + slid {
                due[n] = i;
                n += 1;
            }
        }
    }
    let room = (stage.crowd_target as usize).saturating_sub(s.enemies.len());
    let mut spawn = [true; MAX_WAVES];
    if n > room {
        // The `room` waves held longest, the lower index first among equals.
        let held = |i: usize| s.horde.as_ref().map_or(0, |h| h.held[i]);
        let mut order = [0usize; MAX_WAVES];
        for (p, slot) in order.iter_mut().enumerate().take(n) {
            *slot = p;
        }
        order[..n].sort_unstable_by_key(|&p| (std::cmp::Reverse(held(due[p])), due[p]));
        for &p in &order[room..n] {
            spawn[p] = false;
        }
    }
    let mut area = None;
    for p in 0..n {
        let i = due[p];
        if !spawn[p] {
            if let Some(horde) = s.horde.as_mut() {
                horde.held[i] = horde.held[i].saturating_add(1);
            }
            s.scratch.held_spawns = s.scratch.held_spawns.saturating_add(1);
            continue;
        }
        spawn_wave(stage, s, i, &mut area);
    }
}

/// One spawn of wave `i`: counted as spawned, placed, and refused only by the caps.
fn spawn_wave(stage: &World, s: &mut State, i: usize, area: &mut Option<Area>) {
    let wave = &stage.waves[i];
    s.spawned[i] += 1;
    let key = AgentKey {
        source: AgentSource::Wave,
        index: u32::try_from(i).unwrap_or(u32::MAX),
        rule: 0,
    };
    let at = match (wave.edge, wave.spawn) {
        (Some(edge), _) => {
            let area = *area.get_or_insert_with(|| Area::of_view(&stage.map(), &s.players));
            edge_point(stage, area, &mut s.rng.spawn, edge)
        }
        (None, Some(p)) => p,
        // The validator refuses a wave with neither: the lowest up player's place.
        (None, None) => {
            s.players[Players::slots(s.players.targets())
                .next()
                .unwrap_or(PlayerIdx::SOLO)]
            .at
        }
    };
    spawn_enemy(stage, s, key, at);
}

/// One side of the spawn rectangle: `fixed` is its coordinate on the axis it stands on, and it
/// runs along the other axis from `lo` to `hi`. The sides come in the order top, right, bottom,
/// left; the opposite of side `i` is `(i + 2) % 4`.
#[derive(Clone, Copy)]
struct RectSide {
    vertical: bool,
    fixed: i64,
    lo: i64,
    hi: i64,
}

impl RectSide {
    /// The part of the side on the map `w` by `h`, as `(lo, hi)`, if it has a length.
    fn on_map(self, w: i64, h: i64) -> Option<(i64, i64)> {
        let (across, along) = if self.vertical { (w, h) } else { (h, w) };
        if !(0..=across).contains(&self.fixed) {
            return None;
        }
        let (lo, hi) = (self.lo.max(0), self.hi.min(along));
        (hi > lo).then_some((lo, hi))
    }

    /// The point `t` along the side from `lo`.
    fn at(self, lo: i64, t: i64) -> (i64, i64) {
        if self.vertical {
            (self.fixed, lo + t)
        } else {
            (lo + t, self.fixed)
        }
    }
}

/// The sides the `any` draw walks, in the order top, right, bottom, left, each with its part:
/// top and right run `[lo, hi)` and bottom and left `(lo, hi]`, so every perimeter point has one
/// draw value (the corners go top-left to top, top-right to right, bottom-right to bottom,
/// bottom-left to left). The part is the part on the map `w` by `h`; with none on the map, the whole sides (moved onto it by the
/// caller).
fn any_parts(
    px: i64,
    py: i64,
    ex: i64,
    ey: i64,
    w: i64,
    h: i64,
) -> [(RectSide, Option<(i64, i64)>); 4] {
    let mut sides = rect_sides(px, py, ex, ey);
    // Each corner belongs to one side: top owns top-left, right owns top-right, bottom owns
    // bottom-right and left owns bottom-left, so the four walk the perimeter once.
    for side in &mut sides[2..] {
        side.lo += 1;
        side.hi += 1;
    }
    let parts = sides.map(|side| side.on_map(w, h));
    let on_map = parts.iter().flatten().map(|(lo, hi)| hi - lo).sum::<i64>() > 0;
    let mut out = sides.map(|side| (side, None));
    for (i, side) in sides.iter().enumerate() {
        out[i].1 = if on_map {
            parts[i]
        } else {
            Some((side.lo, side.hi))
        };
    }
    out
}

/// How many values the `any` draw has: the length of the on-map parts of all four sides.
pub(super) fn any_total(px: i64, py: i64, ex: i64, ey: i64, w: i64, h: i64) -> i64 {
    any_parts(px, py, ex, ey, w, h)
        .iter()
        .filter_map(|(_, part)| *part)
        .map(|(lo, hi)| hi - lo)
        .sum()
}

/// The point of the `any` draw's value `t`: the walk along the parts in side order.
pub(super) fn any_point(
    px: i64,
    py: i64,
    ex: i64,
    ey: i64,
    w: i64,
    h: i64,
    mut t: i64,
) -> (i64, i64) {
    for (side, part) in any_parts(px, py, ex, ey, w, h) {
        let Some((lo, hi)) = part else { continue };
        if t < hi - lo {
            return side.at(lo, t);
        }
        t -= hi - lo;
    }
    (px, py)
}

/// The four sides of the spawn rectangle around (`px`, `py`).
fn rect_sides(px: i64, py: i64, ex: i64, ey: i64) -> [RectSide; 4] {
    [
        RectSide {
            vertical: false,
            fixed: py - ey,
            lo: px - ex,
            hi: px + ex,
        },
        RectSide {
            vertical: true,
            fixed: px + ex,
            lo: py - ey,
            hi: py + ey,
        },
        RectSide {
            vertical: false,
            fixed: py + ey,
            lo: px - ex,
            hi: px + ex,
        },
        RectSide {
            vertical: true,
            fixed: px - ex,
            lo: py - ey,
            hi: py + ey,
        },
    ]
}

/// Which side to use when side `i` (0 top, 1 right, 2 bottom, 3 left) of a rectangle around
/// the view centre is wanted: `i` if on the map, otherwise the opposite side `(i + 2) % 4` if
/// on the map; if neither fits, `i` is kept (and clamped to the map border).
fn usable_side(parts: &[Option<(i64, i64)>; 4], i: usize) -> usize {
    if parts[i].is_some() || parts[(i + 2) % 4].is_none() {
        i
    } else {
        (i + 2) % 4
    }
}

/// A point on the spawn rectangle around the view's centre for a main character at `from` (VS1,
/// `view_centre`): ±the spawn edge (the view and `spawn_margin`), drawn
/// from `rng` (the wave's stream for a wave, the horde's for an elite) on the part of `edge`'s
/// side that is on the map. A side wholly off the map gives way to the opposite one; with
/// neither on the map the side itself is used and the point moved onto the map. `any` is one draw
/// over the on-map parts of all four sides, each corner on one side only (`any_parts`); a side with no
/// `along` is one draw along its part; with `along` there is no draw.
pub(super) fn edge_point(stage: &World, area: Area, rng: &mut SplitMix64, edge: ViewEdge) -> Point {
    let map = stage.map();
    let (w, h) = (i64::from(map.width.0), i64::from(map.height.0));
    let centre = area.centre;
    let (px, py) = (i64::from(centre.x.0), i64::from(centre.y.0));
    let (ex, ey) = (
        i64::from(stage.tuning.spawn_edge_x.0) + area.half_x,
        i64::from(stage.tuning.spawn_edge_y.0) + area.half_y,
    );
    let sides = rect_sides(px, py, ex, ey);
    let parts = sides.map(|side| side.on_map(w, h));
    let draw =
        |rng: &mut SplitMix64, n: i64| i64::from(rng.below(u32::try_from(n).unwrap_or(u32::MAX)));
    let (x, y) = match edge.side {
        Side::Any => {
            let t = draw(rng, any_total(px, py, ex, ey, w, h));
            any_point(px, py, ex, ey, w, h, t)
        }
        named => {
            let i = match named {
                Side::Top => 0,
                Side::Right => 1,
                Side::Bottom => 2,
                // `Any` is the arm above.
                Side::Left | Side::Any => 3,
            };
            let j = usable_side(&parts, i);
            let side = sides[j];
            let (lo, hi) = parts[j].unwrap_or((side.lo, side.hi));
            match edge.along {
                Some(along) => {
                    let (half, middle) = if side.vertical { (ey, py) } else { (ex, px) };
                    let t = middle + i64::from(along) * half / 1000;
                    if side.vertical {
                        (side.fixed, t)
                    } else {
                        (t, side.fixed)
                    }
                }
                None => side.at(lo, draw(rng, hi - lo)),
            }
        }
    };
    let fx = |v: i64| Fx(i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX }));
    map.clamp(Point { x: fx(x), y: fx(y) })
}

/// Spawns an enemy of `def` at `at`, unless the cap on enemies alive is reached, or for an
/// elite the stage's cap on elites; an elite that comes after the boss's call is called at
/// once. Returns whether it spawned.
pub(super) fn spawn_enemy(stage: &World, s: &mut State, key: AgentKey, at: Point) -> bool {
    let index = match key.source {
        AgentSource::BossRule => {
            let boss = s.boss.as_ref().expect("a boss summons its own agents");
            stage.boss_summon(boss.boss, key.index, key.rule)
        }
        _ => stage.defs.of_key(key),
    };
    let d = stage.defs.get(index);
    let def = &d.agent;
    // Every number below `MAX_ENTITY_NUMBER` is an enemy's or a companion's; from there on a
    // number reads as a rejoined player's, so the spawns stop (a run spawns far fewer).
    let room = count(&s.enemies) < stage.enemy_cap
        && s.next_id < crate::id::MAX_ENTITY_NUMBER
        && (def.elite.is_none()
            || s.enemies.iter().filter(|e| e.elite.is_some()).count()
                < usize::from(stage.elite_cap()));
    if !room {
        s.scratch.refused_spawns = s.scratch.refused_spawns.saturating_add(1);
        return false;
    }
    let id = EntityId(s.next_id);
    s.next_id += 1;
    let mut enemy = agents::new_agent(id, index, d, key, at, 0);
    enemy.hp = scale_hp(enemy.hp, s.hp_pct);
    enemy.max_hp = enemy.hp;
    if let Some(el) = enemy.elite.as_mut() {
        let called = s.boss.as_ref().and_then(|b| b.elites_called) == Some(true);
        match (el.boss_call, enemy.leave.as_mut()) {
            (BossCall::Leave, Some(l)) => l.go = called,
            // A shrine, or any elite that stays, is never called.
            (BossCall::Stay, _) => {}
            // A mimic is called when it reveals, not before.
            _ if el.disguised => {}
            _ => el.called = called,
        }
    }
    s.enemies.push(enemy);
    // A pack leader's escort comes with it, on a ring around it. Only a wave's enemy
    // has one: that is the definition the escort is found by.
    if key.source == AgentSource::Wave {
        if let Some(escort) = def.elite.as_ref().and_then(|e| e.escort.as_ref()) {
            let key = AgentKey {
                source: AgentSource::Escort,
                index: key.index,
                rule: 0,
            };
            agents::ring_up(stage, s, key, escort, at);
        }
    }
    true
}

/// Moves every orb a tick, and drops those that leave: one whose lifetime is up leaves
/// before it moves, so an orb with `n` ticks is on the map for `n` ticks (core#147).
fn move_orbs(stage: &World, s: &mut State) {
    let holds = stage.map().holds();
    s.orbs.retain_mut(|b| {
        if b.life == 0 {
            false
        } else {
            b.at.x = b.at.x.saturating_add(b.vx);
            b.at.y = b.at.y.saturating_add(b.vy);
            b.life -= 1;
            if let Some(n) = b.stop.as_mut().filter(|n| **n > 0) {
                *n -= 1;
                if *n == 0 {
                    b.vx = Fx(0);
                    b.vy = Fx(0);
                }
            }
            holds(b.at)
        }
    });
}

/// The orb that the beam from orb `i` runs to, if it is live: both are alive, in one group, and
/// neither is still flying toward its halt. A ring of two has one beam, from slot 0.
pub(crate) fn beam_to(orbs: &[crate::sim::state::OrbState], i: usize) -> Option<usize> {
    let from = orbs.get(i)?;
    let link = from.link?;
    if link.of < 2 || (link.of == 2 && link.slot == 1) {
        return None;
    }
    let flying = |b: &crate::sim::state::OrbState| b.stop.is_some_and(|n| n > 0);
    if flying(from) {
        return None;
    }
    let next = (link.slot + 1) % link.of;
    let j = orbs.iter().position(|b| {
        b.link
            .is_some_and(|l| l.group == link.group && l.slot == next)
    })?;
    (!flying(&orbs[j])).then_some(j)
}

/// Two circles overlap (touching counts), in 64-bit fixed point.
fn overlaps(a: Point, ra: i32, b: Point, rb: i32) -> bool {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    let r = i64::from(ra) + i64::from(rb);
    dx * dx + dy * dy <= r * r
}

/// The percent a pickup circle of `me` is widened by: `pickup_pct_of` for its player.
fn pickup_pct(s: &State, me: PlayerIdx) -> u32 {
    pickup_pct_of(s.players.career(me).progress.as_ref(), &s.players[me])
}

/// The percent a pickup circle of `player` is widened by: the stars' share plus the
/// `PickupRadiusPct` total of the run stats in `progress` (0 without levels), saturating, held to
/// `MAX_PICKUP_RADIUS_TOTAL_PCT`. What the snapshot and the offer rating show is this number.
pub(crate) fn pickup_pct_of(
    progress: Option<&state::Progress>,
    player: &state::PlayerState,
) -> u32 {
    let stat = progress.map_or(0, |p| {
        p.totals[crate::run_stats::StatEffect::PickupRadiusPct as usize]
    });
    player
        .pickup_radius_pct
        .saturating_add(stat)
        .min(crate::rewards::MAX_PICKUP_RADIUS_TOTAL_PCT)
}

/// A pickup circle of `radius` widened by `pct` percent, the percent held to
/// `MAX_PICKUP_RADIUS_TOTAL_PCT` (the pickup radius rewards).
fn widened(radius: i32, pct: u32) -> i32 {
    let pct = i64::from(pct.min(crate::rewards::MAX_PICKUP_RADIUS_TOTAL_PCT));
    i32::try_from(i64::from(radius) * (100 + pct) / 100).unwrap_or(i32::MAX)
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

fn collide(stage: &World, s: &mut State, dots: bool, events: &mut Vec<DomainEvent>) {
    if dots {
        entity::tick_dots(stage, s, events);
    }
    // The enemies do not move while hit boxes land, so one grid, built for the first friendly hit
    // box, serves all of them; a tick without one never builds it.
    let mut grid: Option<Grid> = None;
    let boss_alive = s.boss.as_ref().is_some_and(|b| b.hp > 0);
    combat::deal_echoes(stage, s, events);
    hit_boxes::resolve(stage, s, &mut grid, events);
    let mut consumed = std::mem::take(&mut s.scratch.consumed);
    consumed.clear();
    for i in 0..s.orbs.len() {
        let b = &s.orbs[i];
        let shot = Shot {
            at: b.at,
            owner: b.owner,
            friendly: false,
            damage: b.damage,
            damage_type: b.damage_type,
            breaks_charge: b.breaks_charge,
            smash: b.smash,
            combo: b.combo,
            rune: None,
            chain: false,
        };
        let by = Blame::of_program(HitKind::Attack, b.program);
        consumed.push(
            hit_players_side(stage, s, shot, by, events)
                || hit_companions_side(stage, s, shot, events),
        );
    }
    let mut flags = consumed.iter();
    s.orbs.retain(|_| !flags.next().copied().unwrap_or(false));
    s.scratch.consumed = consumed;
    if let Some(grid) = grid {
        s.scratch.grid = grid;
    }
    s.companions.retain(|c| c.hp > 0);
    // A kill's burst, kill-heal and expertise go to the killer; the bursts of all slots share the
    // tick's cap.
    let mut gone_off = 0;
    for me in s.players.indices() {
        combat::bursts(stage, s, me, &mut gone_off, events);
    }
    levels::credit_defeated(stage, s, events);
    // Stardust is shared: every player that is up earns every kill's base.
    for me in s.players.indices() {
        stardust::credit_defeated(stage, s, me);
    }
    stars::drop_from_defeated(stage, s, events);
    heal::drop_from_defeated(stage, s, events);
    for me in s.players.indices() {
        heal::kills_heal(s, me, events);
    }
    slots::count_kills(s);
    // The record is by enemy index, which no step between a hit and this removal changes.
    debug_assert!(s.scratch.killers.len() <= s.enemies.len());
    s.enemies.retain(|e| e.hp > 0);
    s.scratch.killers.clear();
    if boss_alive && s.boss.as_ref().is_some_and(|b| b.hp == 0) {
        stars::boss_killed(stage, s);
        for me in s.players.indices() {
            heal::boss_killed(stage, s, me, events);
            stardust::boss_killed(stage, s, me);
        }
    }
    // The stars and pick-ups on the map are shared: the lowest slot takes what its circle
    // covers, and the next finds it gone.
    for me in s.players.indices() {
        stars::pick_up(stage, s, me, events);
        heal::pick_up(stage, s, me, events);
    }
}

/// The enemies as circles, in index order, for friendly hit boxes to land on and for the
/// push-apart. It is built in the state's scratch grid, which the caller returns when it is done.
fn enemy_grid(stage: &World, s: &mut State) -> Grid {
    let mut grid = std::mem::take(&mut s.scratch.grid);
    let circles = s
        .enemies
        .iter()
        .map(|e| (e.at, stage.defs.agent(e.def).radius.0));
    grid.rebuild(circles, GRID_PROBE_RADIUS);
    grid
}

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, changed by its
/// match-up, reduced by its defence, reported, and credited to the shooter's aggro. Death is
/// reported too. A combo step's hit adds expertise points (`expertise`), puts the
/// damage-over-time runes' statuses on a target it leaves standing, and fires the on-hit runes
/// that are ready (`runes`). The hit points, the kill credit, the events and the aggro are
/// `damage::land`.
fn damage(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    b: Shot,
    target: Who,
    events: &mut Vec<DomainEvent>,
) {
    // A shrine, or a mimic in disguise, is an object: nothing damages it.
    if !damageable(s, target) {
        return;
    }
    let vulnerable = status_pct(
        s.statuses(target),
        crate::sim::state::StatusKind::Vulnerable,
    );
    let match_up = profile(stage, s, target).map_or(MatchUp::None, |p| p.match_up(b.damage_type));
    let id = s.id_of(target);
    // The common case, with no combat reward held, pays for one comparison (ADR-SIM-5).
    let crit = s.players[me].crit_chance_pct != 0
        && combat::crits(s, me, &b, id, s.hp(target), !State::is_ally(target));
    let base = if crit {
        combat::crit_damage(
            b.damage,
            s.players[me].crit_damage_pct,
            stage.tuning.crit_damage_base_pct,
        )
    } else {
        b.damage
    };
    let dealt = scale_damage(
        base,
        0,
        0,
        vulnerable,
        s.match_ups.pct(match_up),
        s.defence(target),
        stage.tuning.defence_k,
    );
    // The buffer is read only in a party, and for a player's own shot (ADR-SIM-7).
    let buffed_by =
        if !State::is_ally(target) && s.players.len() > 1 && s.players.of_id(b.owner).is_some() {
            buffer_of(s, me)
        } else {
            None
        };
    let how = damage::Landing {
        id,
        source: b.owner,
        damage_type: b.damage_type,
        match_up,
        rune: b.rune,
        crit,
        echo: false,
        buffed_by,
    };
    let dead = damage::land(stage, s, me, target, dealt, how, events).dead;
    if !dead && s.players[me].echo_hit_pct != 0 {
        combat::queue_echo(s, me, &b, id, dealt, !State::is_ally(target));
    }
    if b.combo {
        expertise::credit(stage, s, me, target, dead);
        if !dead && b.smash == Some(Smash::Bleed) {
            runes::apply_smash_bleed(s, me, target, events);
        }
        if let Some(Smash::Stun { ticks }) = b.smash.filter(|_| !dead) {
            stun::apply(stage, s, target, b.owner, ticks, events);
        }
        if !s.players[me].runes.is_empty() {
            if !dead {
                runes::apply_dots(s, me, target, events);
            }
            let at = s.at(target);
            runes::on_hit(stage, s, me, id, at, events);
        }
    }
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
