//! The rules of `sim_version` 0 (ADR-035): a development version that changes in place until
//! the freeze at P3, with the hash check across CPUs never relaxed (ADR-019).
//!
//! What a tick does, in this order:
//! 1. the tick counter advances; waves spawn enemies; statuses run out and aggro fades;
//! 2. the main character turns to its stick and, while aim assist or auto mode is on, picks a
//!    target (ADR-072, `select`); its spent evade charges recover, and a press with a charge
//!    ready starts an evade, during a dash too (ADR-074; cancelling its combo step and breaking
//!    a charge), walks its combo: a buffered press, or else auto mode's
//!    (ADR-073), takes the next step, a charge rises or releases (ADR-070, ADR-076; see
//!    `player`), then it dashes or moves (speed cap, slowing, playfield);
//! 3. the boss (phases, timeline, rules: `boss`), then companions, then enemies, in entity-ID
//!    order: move, then the first rule that can fire, fires (ADR-036), which for an attack or
//!    a shot skill starts an attack task; then the enemies push apart, all at once (`push`,
//!    ADR-058), and one found crowded does not step to its place on the next tick. An elite
//!    (ADR-068) holds where it spawned instead of its movement, and its rules fire only while
//!    the main character is within its range;
//! 4. zones go on by a tick (`zones`); bullets whose `on_bullet` wait is over start their
//!    attacks, then every running attack is carried out up to its next wait (`attacks`), making
//!    bullets and zones;
//! 5. bullets move and leave the map;
//! 6. collisions: first the hit boxes of active phases and then the active zones
//!    (`hit_boxes`), then bullets; those of
//!    allies hurt enemies and the boss, those of enemies the main character and the companions;
//! 7. the outcome is decided: death or running out of ticks fails, an empty stage clears;
//! 8. if the run goes on or is cleared, the first offer due opens, and the world stops until it
//!    is answered: the boss offer, then the expertise levels' in level order, then the stars' in
//!    the order taken; a fail opens none.
//!
//! In a stage whose enemies or boss give expertise points, each hit and kill by a combo step in
//! step 6 adds them to the one expertise gauge, and its levels are reached at the end of step 6
//! (`expertise`). Each level reached is due an offer of the kind the stage's level table gives
//! (ADR-106 §4): common options, the weapon's three styles, or the chosen style's upgrades or
//! capstone.
//!
//! In a stage with levels, defeated enemies give their experience as they are removed in
//! step 6, and each level-up gives a stat point (`levels`); points are spent between ticks. In
//! a stage with stars, defeated enemies then drop falling stars, and the main character takes
//! those it reaches; each opens an offer in step 8, answered between ticks (`stars`).
//!
//! The stage skill (ADR-075, `skill`) acts in step 2 on a press, after the evade and before the
//! combo; its black hole pulls after the push-apart of step 3, and its shield takes from the
//! main character's damage in step 6.
//!
//! Orbits (ADR-069, `orbit`) act at the end of step 2: each starts a round of its orbs'
//! attacks when its cooldown has run out and the round before has ended, and turns them; their
//! hit boxes land in step 6. A run holds at most `MAX_ORBITS` orbits of `MAX_ORBS` orbs.
//!
//! Range rewards (ADR-099) widen the hit boxes of the combo steps started after they are taken,
//! adding up over the picks (`player`).
//!
//! Type bonus rewards (ADR-100) add up for each damage type, a group's to each type of its
//! group (`stars`). An attack the main character starts fixes those sums, each held to the
//! match-up file's `max_type_bonus_pct`; each of its phases of a type raises its damage by
//! that type's, after atk-up (`attacks::phase_damage`), and the match-up multiplies it at the
//! hit.
//!
//! On-hit runes (ADR-099, `runes`): each cooldown counts down in step 1; a combo step's hit in
//! step 6 fires each one held and ready, an attack in its own slot whose hit boxes stay on the
//! point hit and land from the next tick, and sets its cooldown. Its hits are no combo step's.
//! A thrown rune's hit boxes (boomerang) fly a step at the end of step 2, after the main
//! character has moved: out to the point hit, then back to the main character. Homing
//! missiles' hit boxes steer a step at the end of step 3, after the push-apart and the pull,
//! toward where their targets are now; one that hits anything in step 6 is gone once the hit
//! boxes have landed.
//!
//! Damage-over-time runes (ADR-099, `runes`): a combo step's hit in step 6 that leaves its
//! target standing puts each one's status on it, bleed's `bleed`, poison's `poison`, burn's `burn`, or a stack
//! more up to its
//! maximum, its duration starting again. Statuses of that kind do not run out in step 1: at
//! the start of step 6, before any hit lands, each counts down to its next tick and deals its
//! damage on it, then its duration runs down (`entity::tick_dots`); a tick without one on
//! anyone skips that pass.
//!
//! Not yet in this version: summons by companions.
//! Rule conditions, selectors and actions beyond the ones in `agents` and `select` never fire.
//!
//! Dynamic budgets (ADR-020): at most `MAX_ENEMIES_ALIVE` enemies, the stage's elite cap of
//! them elites (at most `MAX_ELITES_ALIVE`), `MAX_BULLETS_ALIVE` bullets,
//! `MAX_BULLET_SPAWNS_PER_TICK` new bullets per tick, `MAX_ATTACK_TASKS` attacks at once and
//! `MAX_EMITTER_STEPS_PER_TICK` steps of them per tick. A spawn past a cap is dropped, in
//! spawn order, and a wave spawn that is dropped still counts as spawned; attacks past the
//! step budget wait for the next tick.

mod act;
mod agents;
mod attack_table;
mod attacks;
mod boss;
mod world;
pub(crate) use attacks::phase;
pub(crate) use world::{AttackIndex, BossIndex, DefIndex, HordeSpec, ProgramIndex, World};
mod combat;
mod entity;
mod expertise;
mod grid;
mod heal;
mod hit_boxes;
mod horde;
mod levels;
mod loadout;
mod orbit;
mod player;
mod push;
mod runes;
mod schedule;
mod scratch;
mod skill;
mod stardust;
mod stars;
pub(crate) mod state;
mod stun;
mod zones;

pub(crate) use player::{auto_charge_view, charge_view, combo_view, hold_view};
mod select;

use crate::attack::{DamageType, MatchUp, Profile, ZoneShape};
use crate::behaviour::BossCall;
use crate::difficulties::scale_pct;
use crate::engine::{
    AgentKey, AgentSource, BossState, BulletState, DashState, Dir, Load, MoveOrder, Outcome,
    PartState, PlayerState, Progress, Pull, Smash, StarField, State, UNLIMITED,
};
use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::fixed::{Fx, Point};
use crate::hash::StateHasher;
use crate::id::EntityId;
use crate::input::Input;
use crate::kinds::{KindIndex, KindTable};
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::rune::Rune;
use crate::snapshot::OfferOption;
use crate::trig;
use crate::validate::limits::DAMAGE_SCALE;

use entity::{Who, scale_damage, status_pct};
pub(crate) use expertise::offer_view as expertise_offer_view;
pub(crate) use expertise::styles_view as expertise_styles_view;
use grid::Grid;
pub(crate) use horde::spawn_boss;
use horde::spawn_elite;
pub(crate) use levels::{spend, xp_next};
pub(crate) use loadout::{Loadout, apply as apply_loadout};
pub(crate) use scratch::Scratch;
pub(crate) use stars::{constellation_views, offer_view};

pub(crate) const MAX_ENEMIES_ALIVE: u32 = 500;
pub(crate) const MAX_BULLETS_ALIVE: u32 = 4000;
pub(crate) const MAX_BULLET_SPAWNS_PER_TICK: u32 = 200;
/// Zones alive at once (core Z1, a placeholder cap pending ADR-058); a zone placed over it is
/// dropped, as a bullet over `MAX_BULLETS_ALIVE` is.
pub(crate) const MAX_ZONES_ALIVE: u32 = 128;
/// Attacks being carried out at once (ADR-036 tier 3), and the node steps they may take in a
/// tick between them. An attack that would take more waits for the next tick.
pub(crate) const MAX_ATTACK_TASKS: u32 = 256;
pub(crate) const MAX_EMITTER_STEPS_PER_TICK: u32 = 4096;

/// The main character is always entity 0; the boss, if any, is 1, then the companions.
pub(crate) const PLAYER_ID: EntityId = EntityId(0);
/// The speed cap at full stick (ADR-050).
const PLAYER_SPEED: i32 = Fx::px(3).0;
/// The hurt circle, a tiny one (ADR-050).
pub(crate) const PLAYER_RADIUS: i32 = Fx::px(2).0;
const PLAYER_BULLET_RADIUS: i32 = Fx::px(2).0;
const ENEMY_BULLET_RADIUS: i32 = Fx::px(3).0;
/// Ticks of invulnerability after the main character is hit.
const HIT_INVULNERABLE_TICKS: u32 = 60;

/// Aggro (ADR-036): raised by damage dealt, by buffs and by heals; fading every tick. A point of
/// damage or healing is worth the same aggro as before the scale of ADR-103 (so the two
/// per-point values stay), and the flat amounts scale with it: a buff, the decay and the cap.
const AGGRO_PER_DAMAGE: u32 = 1;
const AGGRO_PER_BUFF: u32 = 40 * DAMAGE_SCALE;
const AGGRO_PER_HEAL_POINT: u32 = 2;
const AGGRO_DECAY: u32 = DAMAGE_SCALE;
const MAX_AGGRO: u32 = 1_000_000 * DAMAGE_SCALE;
pub(crate) fn bullet_radius(friendly: bool) -> i32 {
    if friendly {
        PLAYER_BULLET_RADIUS
    } else {
        ENEMY_BULLET_RADIUS
    }
}

/// What a run brings to its offers: the rewards of its pool, by their place in the rewards file,
/// and the rerolls its setup adds to the stage's (ADR-079 §1).
pub(crate) struct RunPool {
    pub rewards: Vec<u16>,
    pub extra_rerolls: u8,
    pub difficulty: RunDifficulty,
}

/// `hp` scaled by `pct` (ADR-110): never to 0 from a positive value, which would be dead at
/// its spawn.
pub(crate) fn scale_hp(hp: u32, pct: u32) -> u32 {
    match hp {
        0 => 0,
        _ => scale_pct(hp, pct).max(1),
    }
}

/// What a run takes from its difficulty (ADR-110): the percents its enemies' hit points and
/// attacks' damage are scaled by, and the range of rarities its stars and offers may have.
/// Fixed for the run, part of the setup, so the state hash writes nothing for it.
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
/// stage, the setup's for a run (ADR-051).
pub(crate) fn initial_state(
    stage: &World,
    content: &RunContent,
    pool: RunPool,
    seed: u64,
    kinds: KindTable,
) -> State {
    let run_stats = content.run_stats();
    let rewards = content.rewards().rewards().len();
    let mut next_id = 1;
    let boss_id = stage.boss.as_ref().map(|_| {
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
            let angle = i32::try_from(i).unwrap_or(0) * 90 * trig::PER_DEGREE;
            agents::new_agent(id, index, stage.defs.get(index), key, start, angle)
        })
        .collect();
    // The boss's parts come after the companions in the order of entity IDs.
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
    State {
        scratch: Scratch::default(),
        tick: 0,
        rng: SplitMix64::new(seed),
        outcome: Outcome::Running,
        player: PlayerState {
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
        },
        boss,
        companions,
        enemies: Vec::new(),
        tasks: Vec::new(),
        runs: Vec::new(),
        move_index: crate::engine::MoveIndex::default(),
        bullets: Vec::new(),
        zones: Vec::new(),
        spawned: vec![0; stage.waves.len()],
        next_id,
        progress: stage.levels.as_ref().map(|_| Progress {
            level: 1,
            xp: 0,
            points: 0,
            spent: vec![0; run_stats.stats().len()],
            totals: [0; StatEffect::ALL.len()],
        }),
        stars: stage.stars.as_ref().map(|st| StarField {
            misses: 0,
            on_map: Vec::new(),
            due: Vec::new(),
            offer: None,
            rarity: pool.difficulty.rarity,
            rerolls_left: u32::from(st.rerolls) + u32::from(pool.extra_rerolls),
            banishes_left: u32::from(st.banishes),
            extra_candidates: 0,
            rune_pool: runes::FULL_POOL,
            pool: pool.rewards,
            taken: vec![0; rewards],
            banished: vec![false; rewards],
            set_counts: vec![0; content.constellations().sets().len()],
        }),
        heal_pickups: stage.heal.as_ref().map(|_| Vec::new()),
        expertise: expertise::initial(stage, content.expertise()),
        match_ups: *content.match_ups(),
        runes: content.runes().clone(),
        stun: run_stats.stun(),
        max_attack_speed_pct: run_stats.max_attack_speed_pct(),
        enemy_attack_damage: run_stats.enemy_attack_damage(),
        max_rune_haste_pct: content.rewards().max_rune_haste_pct(),
        horde: None,
        stardust: stage.stardust.as_ref().map(|_| 0),
        regen: 0,
        hp_pct: pool.difficulty.hp_pct,
        damage_pct: pool.difficulty.damage_pct,
        crit_seed: seed,
        echoes: Vec::new(),
        kinds,
    }
}

/// The big patterns the boss has announced (ADR-103), for the snapshot; an enemy's are not.
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
/// The boss's state at its spawn, its hit points and its parts' scaled by `hp_pct` (ADR-110).
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

pub(crate) fn step(
    stage: &World,
    rewards: &Rewards,
    expertise: &Expertise,
    s: &mut State,
    input: Input,
    events: &mut Vec<DomainEvent>,
    load: &mut Load,
) {
    if s.outcome != Outcome::Running {
        return;
    }
    s.tick += 1;
    loadout::regen(s);
    let mut spawns_left = MAX_BULLET_SPAWNS_PER_TICK;

    spawn_waves(stage, s);
    spawn_elite(stage, s, events);
    let dots = entity::tick_statuses_and_aggro(s, events);
    runes::tick(s);
    attacks::advance_runs(stage, s, events);
    player::update(stage, rewards, s, input, events);
    if !s.player.runes.is_empty() {
        runes::fly(stage, s);
    }
    orbit::act(stage, rewards, s, events);
    boss::act(stage, s, events);
    boss::call_elites(stage, s, events);
    agents::act(stage, s, events);
    push::apply(stage, s);
    skill::pull(stage, s);
    if !s.player.runes.is_empty() {
        runes::steer(stage, s);
    }
    zones::advance(s);
    attacks::fire_hooks(s);
    let (tasks, emitter_steps) = attacks::run(stage, s, &mut spawns_left);
    *load = Load {
        enemies: count(&s.enemies),
        tasks,
        bullets: count(&s.bullets),
        zones: count(&s.zones),
        bullet_spawns: MAX_BULLET_SPAWNS_PER_TICK - spawns_left,
        emitter_steps,
    };
    move_bullets(stage, s);
    collide(stage, s, dots, events);
    expertise::level_up(expertise, s);
    decide_outcome(stage, s, events);
    match s.outcome {
        Outcome::Running | Outcome::Cleared => open_offers(stage, rewards, expertise, s, events),
        Outcome::Failed => {}
    }
    debug_assert!(
        entity::enemies_in_id_order(s),
        "enemies out of entity-ID order"
    );
}

/// An offer is open, a star's, a boss's or an expertise level's: the world is stopped.
pub(crate) fn offer_open(s: &State) -> bool {
    s.stars.as_ref().is_some_and(|f| f.offer.is_some())
        || s.expertise.as_ref().is_some_and(|g| g.offer.is_some())
}

/// Opens the first offer due, unless one is open: the boss offer, then the expertise levels'
/// in level order, then the stars' in the order taken (ADR-079 §2).
fn open_offers(
    stage: &World,
    rewards: &Rewards,
    expertise: &Expertise,
    s: &mut State,
    events: &mut Vec<DomainEvent>,
) {
    stars::open_due(stage, rewards, s, events, true);
    expertise::open_due(stage, expertise, s, events);
    stars::open_due(stage, rewards, s, events, false);
}

/// The options of the open offer, in index order; empty without one.
pub(crate) fn offer_options(
    stage: &World,
    rewards: &Rewards,
    expertise: &Expertise,
    s: &State,
) -> Vec<OfferOption> {
    if s.expertise.as_ref().is_some_and(|g| g.offer.is_some()) {
        expertise::options(stage, expertise, s)
    } else {
        stars::options(rewards, s)
    }
}

/// Carries out `option`, one of `offer_options`, on the open offer; when that closes it, the
/// next offer due opens.
pub(crate) fn choose(
    stage: &World,
    rewards: &Rewards,
    expertise: &Expertise,
    s: &mut State,
    option: OfferOption,
    events: &mut Vec<DomainEvent>,
) {
    if s.expertise.as_ref().is_some_and(|g| g.offer.is_some()) {
        expertise::choose(stage, expertise, s, option, events);
    } else {
        stars::choose(stage, rewards, s, option, events);
    }
    open_offers(stage, rewards, expertise, s, events);
}

fn count<T>(items: &[T]) -> u32 {
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

fn spawn_waves(stage: &World, s: &mut State) {
    for (i, wave) in stage.waves.iter().enumerate() {
        let spawned = s.spawned[i];
        if spawned >= wave.count {
            continue;
        }
        let due = u64::from(wave.at_tick) + u64::from(spawned) * u64::from(wave.every_ticks);
        if u64::from(s.tick) < due {
            continue;
        }
        s.spawned[i] = spawned + 1;
        let key = AgentKey {
            source: AgentSource::Wave,
            index: u32::try_from(i).unwrap_or(u32::MAX),
            rule: 0,
        };
        let at = match wave.around {
            Some(ring) => around_player(stage, s, ring),
            None => wave.spawn,
        };
        spawn_enemy(stage, s, key, at);
    }
}

/// A point `ring` from the main character, at an angle drawn from the run's generator, moved
/// onto the map.
fn around_player(stage: &World, s: &mut State, ring: Fx) -> Point {
    let angle = i32::try_from(s.rng.below(360)).unwrap_or(0) * trig::PER_DEGREE;
    let (sin, cos) = trig::sin_cos(angle);
    let offset = |unit: i32| {
        i32::try_from(i64::from(ring.0) * i64::from(unit) / i64::from(trig::SCALE)).unwrap_or(0)
    };
    stage.map().clamp(Point {
        x: s.player.at.x.saturating_add(Fx(offset(cos))),
        y: s.player.at.y.saturating_add(Fx(offset(sin))),
    })
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
    let room = count(&s.enemies) < MAX_ENEMIES_ALIVE
        && (def.elite.is_none()
            || s.enemies.iter().filter(|e| e.elite.is_some()).count()
                < usize::from(stage.elite_cap()));
    if !room {
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
    // A pack leader's escort comes with it, on a ring around it (ADR-090). Only a wave's enemy
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

fn spawn_bullet(s: &mut State, bullet: BulletState, spawns_left: &mut u32) {
    if *spawns_left == 0 || count(&s.bullets) >= MAX_BULLETS_ALIVE {
        return;
    }
    *spawns_left -= 1;
    s.bullets.push(bullet);
}

/// Moves every bullet a tick, and drops those that leave: one whose lifetime is up leaves
/// before it moves, so a bullet with `n` ticks is on the map for `n` ticks (core#147).
fn move_bullets(stage: &World, s: &mut State) {
    let holds = stage.map().holds();
    s.bullets.retain_mut(|b| {
        if b.life == 0 {
            return false;
        }
        b.at.x = b.at.x.saturating_add(b.vx);
        b.at.y = b.at.y.saturating_add(b.vy);
        if b.life != UNLIMITED {
            b.life -= 1;
        }
        holds(b.at)
    });
}

/// Two circles overlap (touching counts), in 64-bit fixed point.
fn overlaps(a: Point, ra: i32, b: Point, rb: i32) -> bool {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    let r = i64::from(ra) + i64::from(rb);
    dx * dx + dy * dy <= r * r
}

/// A pickup circle of `radius` widened by `pct` percent, the percent held to
/// `MAX_PICKUP_RADIUS_TOTAL_PCT` (the pickup radius rewards).
fn widened(radius: i32, pct: u32) -> i32 {
    let pct = i64::from(pct.min(crate::rewards::MAX_PICKUP_RADIUS_TOTAL_PCT));
    i32::try_from(i64::from(radius) * (100 + pct) / 100).unwrap_or(i32::MAX)
}

/// What a hit needs to know about the bullet that made it.
#[derive(Clone, Copy)]
struct Shot {
    at: Point,
    owner: EntityId,
    friendly: bool,
    damage: u32,
    /// Its type (ADR-100); `None` is neutral.
    damage_type: Option<DamageType>,
    breaks_charge: bool,
    /// What a smash does to the enemy it hits (ADR-084 point 8), for a combo part's hit.
    smash: Option<Smash>,
    /// A combo step's hit, which adds expertise points (ADR-078 §2).
    combo: bool,
    /// The rune that dealt it (ADR-099), which the `Hit` event reports.
    rune: Option<Rune>,
    /// A hit the combat rewards make (a kill burst), which neither crits nor echoes.
    chain: bool,
}

fn collide(stage: &World, s: &mut State, dots: bool, events: &mut Vec<DomainEvent>) {
    if dots {
        entity::tick_dots(stage, s, events);
    }
    // The enemies do not move while hit boxes and bullets land, so one grid, built for the first
    // friendly hit box or bullet, serves all of them; a tick without either never builds it.
    let mut grid: Option<Grid> = None;
    let boss_alive = s.boss.as_ref().is_some_and(|b| b.hp > 0);
    combat::deal_echoes(s, events);
    hit_boxes::resolve(stage, s, &mut grid, events);
    let mut consumed = std::mem::take(&mut s.scratch.consumed);
    consumed.clear();
    for i in 0..s.bullets.len() {
        let b = &s.bullets[i];
        let shot = Shot {
            at: b.at,
            owner: b.owner,
            friendly: b.friendly,
            damage: b.damage,
            damage_type: b.damage_type,
            breaks_charge: b.breaks_charge,
            smash: b.smash,
            combo: b.combo,
            rune: None,
            chain: false,
        };
        consumed.push(if shot.friendly {
            let grid = grid.get_or_insert_with(|| enemy_grid(stage, s));
            hit_enemy_side(stage, s, grid, shot, events)
        } else {
            hit_ally_side(stage, s, shot, events)
        });
    }
    let mut flags = consumed.iter();
    s.bullets
        .retain(|_| !flags.next().copied().unwrap_or(false));
    s.scratch.consumed = consumed;
    if let Some(grid) = grid {
        s.scratch.grid = grid;
    }
    s.companions.retain(|c| c.hp > 0);
    combat::bursts(stage, s, events);
    levels::credit_defeated(stage, s, events);
    stardust::credit_defeated(stage, s);
    stars::drop_from_defeated(stage, s, events);
    heal::drop_from_defeated(stage, s, events);
    heal::kills_heal(s, events);
    s.enemies.retain(|e| e.hp > 0);
    if boss_alive && s.boss.as_ref().is_some_and(|b| b.hp == 0) {
        stars::boss_killed(stage, s);
        heal::boss_killed(stage, s, events);
        stardust::boss_killed(stage, s);
    }
    stars::pick_up(stage, s, events);
    heal::pick_up(stage, s, events);
}

/// The enemies as circles, in index order, for friendly bullets to land on.
/// It is built in the state's scratch grid, which the caller returns when it is done.
fn enemy_grid(stage: &World, s: &mut State) -> Grid {
    let mut grid = std::mem::take(&mut s.scratch.grid);
    let circles = s
        .enemies
        .iter()
        .map(|e| (e.at, stage.defs.agent(e.def).radius.0));
    grid.rebuild(circles, PLAYER_BULLET_RADIUS);
    grid
}

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, changed by its
/// match-up, reduced by its defence, reported, and credited to the shooter's aggro. Death is
/// reported too. A combo step's hit adds expertise points (`expertise`), puts the
/// damage-over-time runes' statuses on a target it leaves standing, and fires the on-hit runes
/// that are ready (`runes`).
fn damage(stage: &World, s: &mut State, b: Shot, target: Who, events: &mut Vec<DomainEvent>) {
    // A shrine, or a mimic in disguise, is an object: nothing damages it (ADR-090).
    if !damageable(s, target) {
        return;
    }
    let vulnerable = status_pct(s.statuses(target), crate::engine::StatusKind::Vulnerable);
    let match_up = profile(stage, s, target).map_or(MatchUp::None, |p| p.match_up(b.damage_type));
    let id = s.id_of(target);
    // The common case, with no combat reward held, pays for one comparison (ADR-058).
    let crit = s.player.crit_chance_pct != 0
        && combat::crits(s, &b, id, s.hp(target), !State::is_ally(target));
    let base = if crit {
        combat::crit_damage(b.damage, s.player.crit_damage_pct)
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
    );
    let hp = s.hp_mut(target);
    *hp = hp.saturating_sub(dealt);
    let dead = *hp == 0;
    events.push(DomainEvent::Hit {
        source: b.owner,
        target: id,
        target_kind: kind_of(s, target),
        damage: dealt,
        damage_type: b.damage_type,
        match_up,
        rune: b.rune,
        crit,
        echo: false,
    });
    if dead {
        events.push(death_event(s, target, id));
    } else if s.player.echo_hit_pct != 0 {
        combat::queue_echo(s, &b, id, dealt, !State::is_ally(target));
    }
    s.raise_aggro(b.owner, dealt.saturating_mul(AGGRO_PER_DAMAGE));
    if b.combo {
        expertise::credit(stage, s, target, dead);
        if !dead && b.smash == Some(Smash::Bleed) {
            runes::apply_smash_bleed(s, target, events);
        }
        if let Some(Smash::Stun { ticks }) = b.smash.filter(|_| !dead) {
            stun::apply(stage, s, target, b.owner, ticks, events);
        }
        if !s.player.runes.is_empty() {
            if !dead {
                runes::apply_dots(s, target, events);
            }
            let at = s.at(target);
            runes::on_hit(stage, s, id, at, events);
        }
    }
}

/// Whether anything can damage `target`: a shrine and a mimic in disguise are objects (ADR-090).
fn damageable(s: &State, target: Who) -> bool {
    !matches!(target, Who::Enemy(i) if s.enemies[i].shrine.is_some() || s.enemies[i].hidden())
}

/// What a death is reported as: a part breaks, everything else dies.
fn death_event(s: &State, target: Who, id: EntityId) -> DomainEvent {
    match (target, s.boss.as_ref()) {
        (Who::BossPart(i), Some(boss)) => DomainEvent::PartBroken {
            part: boss.parts[i].name.clone(),
        },
        _ => DomainEvent::Died {
            entity: id,
            kind: kind_of(s, target),
        },
    }
}

/// The kind of a target (`EntityView::kind_index`): the index of the content ID the Bestiary keys
/// on. An
/// enemy's, an elite's or a companion's is its sprite ID, the boss's is its own, a boss part's
/// is the part's ID. The main character has none, and a mimic still in disguise shows none.
pub(super) fn kind_of(s: &State, who: Who) -> Option<KindIndex> {
    match who {
        Who::Player => None,
        Who::Boss => Some(s.boss_state().kind),
        Who::BossPart(i) => Some(s.boss_state().parts[i].kind),
        Who::Companion(i) => Some(s.companions[i].kind),
        Who::Enemy(i) => {
            let e = &s.enemies[i];
            (!e.hidden()).then_some(e.kind)
        }
    }
}

/// The profile of a target (ADR-100): an enemy's or an elite's own, the boss's, or a part's own.
/// The main character and the companions have none.
fn profile<'a>(stage: &'a World, s: &'a State, target: Who) -> Option<&'a Profile> {
    match target {
        Who::Enemy(i) => stage.defs.agent(s.enemies[i].def).profile.as_ref(),
        Who::Boss => s.boss_def(stage)?.profile.as_ref(),
        Who::BossPart(i) => s.boss_def(stage)?.parts.get(i)?.profile.as_ref(),
        Who::Player | Who::Companion(_) => None,
    }
}

/// The lowest live enemy a shot at `at` lands on past the mimics in disguise from `first` on:
/// the rare shot that lands on one.
#[cold]
fn past_hidden(s: &State, grid: &Grid, at: Point, mut first: Option<usize>) -> Option<usize> {
    while let Some(i) = first.filter(|&i| s.enemies[i].hidden()) {
        first = grid.first_hit(at, PLAYER_BULLET_RADIUS, |j| {
            j > i && s.enemies[j].hp > 0 && s.enemies[j].shrine.is_none()
        });
    }
    first
}

/// An ally's shot against the enemies in ID order, then the boss. Returns whether it was used up.
/// `grid` holds the enemies of this tick, so only those near the shot are checked.
fn hit_enemy_side(
    stage: &World,
    s: &mut State,
    grid: &Grid,
    b: Shot,
    events: &mut Vec<DomainEvent>,
) -> bool {
    // A shot passes through a shrine.
    let hittable = |i: usize| s.enemies[i].hp > 0 && s.enemies[i].shrine.is_none();
    let mut first = grid.first_hit(b.at, PLAYER_BULLET_RADIUS, hittable);
    // And through a mimic in disguise. Checked on the one found rather than in `hittable`, which
    // runs for every enemy near every bullet (ADR-058).
    if first.is_some_and(|i| s.enemies[i].hidden()) {
        first = past_hidden(s, grid, b.at, first);
    }
    if let Some(i) = first {
        damage(stage, s, b, Who::Enemy(i), events);
        match b.smash {
            Some(Smash::Pull(pull)) => pull_enemy(stage, s, i, pull),
            Some(Smash::Knockback(push)) => knock_enemy(stage, s, i, push),
            _ => {}
        }
        return true;
    }
    let Some((def, state)) = s.boss_pair(stage) else {
        return false;
    };
    if state.hp == 0 {
        return false;
    }
    // The parts are in front of the body. While the boss is invulnerable a shot that lands is
    // used up and does no harm.
    let absorbed = state.invulnerable > 0;
    for (i, part) in state.parts.iter().enumerate() {
        if part.hp > 0
            && overlaps(
                b.at,
                PLAYER_BULLET_RADIUS,
                s.at(Who::BossPart(i)),
                part.radius,
            )
        {
            if !absorbed {
                damage(stage, s, b, Who::BossPart(i), events);
            }
            return true;
        }
    }
    if overlaps(b.at, PLAYER_BULLET_RADIUS, state.at, def.radius.0) {
        if !absorbed {
            damage(stage, s, b, Who::Boss, events);
        }
        return true;
    }
    false
}

/// The Sub weapon's pull on enemy `i`, hit by a combo part (ADR-070): it is drawn toward the
/// main character by at most `distance`, over `ticks`, as a move order that replaces its
/// movement, and stops where their circles touch. A dead enemy and an elite are not pulled.
pub(super) fn pull_enemy(stage: &World, s: &mut State, i: usize, pull: Pull) {
    let e = &s.enemies[i];
    if e.hp == 0 || e.elite.is_some() {
        return;
    }
    let to = s.player.at;
    let (dx, dy) = (
        i64::from(e.at.x.0) - i64::from(to.x.0),
        i64::from(e.at.y.0) - i64::from(to.y.0),
    );
    let len = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    let contact = i64::from(PLAYER_RADIUS) + i64::from(stage.defs.agent(e.def).radius.0);
    let left = (len - i64::from(pull.distance)).max(contact);
    if left >= len {
        return;
    }
    let at =
        |from: Fx, d: i64| Fx(i32::try_from(i64::from(from.0) + d * left / len).unwrap_or(from.0));
    let goal = stage.map().clamp(Point {
        x: at(to.x, dx),
        y: at(to.y, dy),
    });
    s.enemies[i].order = Some(MoveOrder {
        to: goal,
        ticks_left: pull.ticks.max(1),
    });
}

/// A smash's knockback on enemy `i` (ADR-084 point 8): the pull's mirror. It is pushed away from
/// the main character by at most `distance`, over `ticks`, as a move order that replaces its
/// movement, and ends inside the map. A dead enemy and an elite are not pushed, nor one on the
/// main character's very centre (no direction); a boss and its parts are never enemies here.
pub(super) fn knock_enemy(stage: &World, s: &mut State, i: usize, push: Pull) {
    let e = &s.enemies[i];
    if e.hp == 0 || e.elite.is_some() {
        return;
    }
    let from = s.player.at;
    let (dx, dy) = (
        i64::from(e.at.x.0) - i64::from(from.x.0),
        i64::from(e.at.y.0) - i64::from(from.y.0),
    );
    let len = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    if len == 0 {
        return;
    }
    let step = i64::from(push.distance);
    let at =
        |from: Fx, d: i64| Fx(i32::try_from(i64::from(from.0) + d * step / len).unwrap_or(from.0));
    let goal = stage.map().clamp(Point {
        x: at(e.at.x, dx),
        y: at(e.at.y, dy),
    });
    s.enemies[i].order = Some(MoveOrder {
        to: goal,
        ticks_left: push.ticks.max(1),
    });
}

/// Damage to the main character, raised by its vulnerability and reduced by its defence; it is
/// then invulnerable for a while.
fn hurt_player(stage: &World, s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) {
    let vulnerable = status_pct(&s.player.statuses, crate::engine::StatusKind::Vulnerable);
    // The main character has no profile (ADR-100): every hit on it is neutral.
    let dealt = scale_damage(b.damage, 0, 0, vulnerable, 0, s.player.defence);
    let dealt = skill::absorb(s, dealt);
    let p = &mut s.player;
    p.hp = p.hp.saturating_sub(dealt);
    p.invulnerable = HIT_INVULNERABLE_TICKS;
    events.push(DomainEvent::PlayerHit { hp_left: p.hp });
    // A powerful attack breaks a charge (core#83).
    if b.breaks_charge {
        player::break_charge(stage, s, events);
    }
}

/// `b` with the damage a pack leader's aura adds (ADR-090), for a hit of an enemy's on an ally:
/// if the enemy that dealt it is still on the map and not an elite, the highest `damage_percent`
/// among the auras whose leader's centre is within `radius` of its centre (the edge counts)
/// raises the damage to `damage * (100 + percent) / 100`, rounded down. Every leader still in
/// the enemies counts, in index order, so a leader killed in this tick's collisions still gives
/// its aura through the rest of the tick, and none from the next, when the dead are removed.
/// An enemy that has since left the map or died earlier gets none. No random number is drawn.
fn aided(stage: &World, s: &State, b: Shot) -> Shot {
    let Ok(at) = s.enemies.binary_search_by_key(&b.owner, |e| e.id) else {
        return b;
    };
    let owner = &s.enemies[at];
    if owner.elite.is_some() {
        return b;
    }
    let percent = aura_percent(aura_sources(stage, s), owner.at);
    Shot {
        damage: u32::try_from(u64::from(b.damage) * (100 + percent) / 100).unwrap_or(u32::MAX),
        ..b
    }
}

/// The auras of the elites on the map, each with its leader's centre, in index order (ADR-090).
pub(crate) fn aura_sources<'a>(
    stage: &'a World,
    s: &'a State,
) -> impl Iterator<Item = (Point, crate::behaviour::Aura)> + 'a {
    s.enemies
        .iter()
        .filter(|l| l.elite.is_some())
        .filter_map(|l| {
            stage
                .defs
                .agent(l.def)
                .elite
                .as_ref()?
                .aura
                .map(|aura| (l.at, aura))
        })
}

/// The highest `damage_percent` among the auras whose leader's centre is within its radius of
/// `at` (the edge counts); 0 when none reaches it.
pub(crate) fn aura_percent(
    auras: impl Iterator<Item = (Point, crate::behaviour::Aura)>,
    at: Point,
) -> u64 {
    auras
        .filter(|(from, aura)| {
            let dx = i64::from(from.x.0) - i64::from(at.x.0);
            let dy = i64::from(from.y.0) - i64::from(at.y.0);
            let reach = i64::from(aura.radius.0);
            dx * dx + dy * dy <= reach * reach
        })
        .map(|(_, aura)| u64::from(aura.damage_percent))
        .max()
        .unwrap_or(0)
}

/// An enemy's shot against the main character (unless it is invulnerable), then the
/// companions in ID order.
fn hit_ally_side(stage: &World, s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) -> bool {
    let p = &s.player;
    if p.invulnerable == 0 && p.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, p.at, PLAYER_RADIUS) {
        hurt_player(stage, s, aided(stage, s, b), events);
        return true;
    }
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let radius = stage.defs.agent(c.def).radius.0;
        if c.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, c.at, radius) {
            damage(stage, s, aided(stage, s, b), Who::Companion(i), events);
            return true;
        }
    }
    false
}

/// The main character's death wins over a clear on the same tick.
fn decide_outcome(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    if s.player.hp == 0 {
        s.outcome = Outcome::Failed;
        events.push(DomainEvent::StageFailed);
        return;
    }
    let cleared = match &s.boss {
        // A run with a horde ends in a clear only by the final boss's death: a mid-boss's is
        // not the end, and neither is an empty field.
        _ if s.horde.is_some() => horde::final_boss_dead(s),
        Some(b) => b.hp == 0,
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

fn hash_statuses(h: &mut StateHasher, statuses: &[crate::engine::StatusState]) {
    h.write_u32(count(statuses));
    for st in statuses {
        h.write_u32(st.kind as u32);
        h.write_u32(st.source.0);
        h.write_u32(st.remaining);
        h.write_u32(u32::from(st.stacks));
        h.write_u32(u32::from(st.value_pct));
        h.write_u32(st.total);
        // Only for a damage-over-time status, which its kind already tells apart, so other
        // statuses hash as before.
        if let Some(d) = st.dot {
            h.write_u32(d.damage);
            h.write_u32(type_code(d.damage_type));
            h.write_u32(d.atk_up_pct);
            h.write_u32(d.type_bonus_pct);
            h.write_u32(d.interval);
            h.write_u32(d.next_in);
        }
    }
}

fn hash_agent(h: &mut StateHasher, a: &crate::engine::AgentState) {
    h.write_u32(a.id.0);
    h.write_u32(a.key.source as u32);
    h.write_u32(a.key.index);
    h.write_u32(a.key.rule);
    h.write_i32(a.at.x.0);
    h.write_i32(a.at.y.0);
    h.write_u32(a.hp);
    h.write_u32(a.max_hp);
    h.write_u32(a.defence);
    h.write_u32(a.age);
    h.write_u32(a.aggro);
    h.write_i32(a.orbit_angle);
    h.write_bool(a.crowded);
    // Only for an elite, which its key already tells apart, so other agents hash as before.
    if let Some(el) = a.elite {
        h.write_i32(el.home.x.0);
        h.write_i32(el.home.y.0);
        h.write_bool(el.called);
        // Only while a mimic is disguised, so every other elite hashes as before.
        if el.disguised {
            h.write_bool(true);
        }
    }
    // Only while immune to a stun, so every other agent hashes as before.
    if a.stun_immune > 0 {
        h.write_u32(a.stun_immune);
    }
    // Only for a shrine, which its key already tells apart.
    if let Some(sh) = &a.shrine {
        h.write_bool(sh.fight.is_some());
        h.write_u32(sh.fight.unwrap_or(0));
        h.write_u32(count(&sh.guardians));
        for g in &sh.guardians {
            h.write_u32(g.0);
        }
    }
    // Only for an agent that leaves, which its key already tells apart.
    if let Some(l) = a.leave {
        h.write_bool(l.disturbed.is_some());
        h.write_u32(l.disturbed.unwrap_or(0));
        h.write_bool(l.go);
    }
    match a.order {
        Some(o) => {
            h.write_bool(true);
            h.write_i32(o.to.x.0);
            h.write_i32(o.to.y.0);
            h.write_u32(o.ticks_left);
        }
        None => h.write_bool(false),
    }
    h.write_u32(count(&a.rules));
    for r in &a.rules {
        h.write_u32(r.cooldown_left);
        h.write_u32(r.charges_left);
        h.write_bool(r.fired);
    }
    hash_statuses(h, &a.statuses);
}

fn hash_path(h: &mut StateHasher, path: &[crate::engine::PathStep]) {
    h.write_u32(count(path));
    for step in path {
        h.write_u32(u32::from(step.node));
        h.write_bool(step.then);
    }
}

fn hash_aim(h: &mut StateHasher, aim: crate::engine::Aim) {
    match aim {
        crate::engine::Aim::Entity(id) => {
            h.write_u32(0);
            h.write_u32(id.0);
        }
        crate::engine::Aim::Fixed(d) => {
            h.write_u32(1);
            h.write_i32(d.x);
            h.write_i32(d.y);
        }
    }
}

/// What identifies an attack task for the hash is where it has got to, not which program it
/// runs: a preset and its inline copy are the same task and hash the same.
fn hash_task(h: &mut StateHasher, t: &crate::engine::AttackTask) {
    h.write_u32(t.owner.0);
    hash_path(h, &t.base);
    match t.origin {
        crate::engine::Origin::Owner => h.write_bool(false),
        crate::engine::Origin::At(p) => {
            h.write_bool(true);
            h.write_i32(p.x.0);
            h.write_i32(p.y.0);
        }
    }
    hash_aim(h, t.aim);
    h.write_bool(t.friendly);
    h.write_u32(t.damage);
    h.write_bool(t.breaks_charge);
    hash_smash(h, t.smash);
    h.write_u32(t.age);
    h.write_u32(t.wait_left);
    h.write_i32(t.parent_dir.x);
    h.write_i32(t.parent_dir.y);
    h.write_i32(t.last_dir.x);
    h.write_i32(t.last_dir.y);
    h.write_u32(count(&t.frames));
    for f in &t.frames {
        h.write_u32(u32::from(f.idx));
        h.write_u32(f.loop_i);
        h.write_u32(f.loop_n);
        h.write_i32(f.param);
        h.write_i32(f.offset);
        h.write_u32(f.hook_after);
    }
}

/// A phased attack in progress (ADR-055). Like a task's program, its phases are not written:
/// they come from the stage, and where the run has got to is.
fn hash_run(h: &mut StateHasher, world: &World, r: &crate::engine::AttackRun) {
    h.write_u32(r.owner.0);
    h.write_u32(u32::from(r.slot));
    // A combo step's phases may be shortened by attack speed (ADR-070), so their lengths are
    // written.
    h.write_u32(u32::try_from(attacks::phase_count(world, r)).unwrap_or(u32::MAX));
    for i in 0..attacks::phase_count(world, r) {
        h.write_u32(attacks::phase(world, r, i).ticks);
    }
    h.write_u32(r.phase);
    h.write_u32(r.tick);
    h.write_i32(r.facing.x);
    h.write_i32(r.facing.y);
    hash_aim(h, r.aim);
    h.write_bool(r.friendly);
    h.write_u32(r.base);
    h.write_u32(r.atk_up_pct);
    h.write_u32(r.age);
    h.write_u32(count(&r.hits));
    for (id, tick) in &r.hits {
        h.write_u32(id.0);
        h.write_u32(*tick);
    }
    hash_smash(h, r.smash);
}

fn hash_smash(h: &mut StateHasher, smash: Option<Smash>) {
    match smash {
        Some(Smash::Pull(Pull { distance, ticks })) => {
            h.write_bool(true);
            h.write_i32(distance);
            h.write_u32(ticks);
        }
        Some(Smash::Knockback(Pull { distance, ticks })) => {
            h.write_bytes(&[4]);
            h.write_i32(distance);
            h.write_u32(ticks);
        }
        // A tag that is neither the presence flag's 0 nor 1: a bleed has no numbers of its own.
        Some(Smash::Bleed) => h.write_bytes(&[2]),
        Some(Smash::Stun { ticks }) => {
            h.write_bytes(&[3]);
            h.write_u32(ticks);
        }
        None => h.write_bool(false),
    }
}

/// The state hash: FNV-1a 64 over the whole state at fixed widths, field by field in the order
/// of the `State` types, with each list preceded by its length (ADR-019).
pub(crate) fn state_hash(world: &World, s: &State) -> u64 {
    let mut h = StateHasher::new();
    h.write_u32(s.tick);
    h.write_u64(s.rng.state());
    h.write_u32(match s.outcome {
        Outcome::Running => 0,
        Outcome::Cleared => 1,
        Outcome::Failed => 2,
    });
    let p = &s.player;
    h.write_i32(p.at.x.0);
    h.write_i32(p.at.y.0);
    h.write_u32(p.hp);
    h.write_u32(p.max_hp);
    // A reward raises it in the run, as well as a perk fixes it at the start.
    h.write_u32(s.regen);
    h.write_u32(p.defence);
    h.write_u32(p.invulnerable);
    h.write_i32(p.facing.x);
    h.write_i32(p.facing.y);
    match p.target {
        Some(id) => {
            h.write_bool(true);
            h.write_u32(id.0);
        }
        None => h.write_bool(false),
    }
    h.write_u32(u32::from(p.evade_charges));
    // At most the charges held, so at most 3.
    h.write_u32(p.evade_recovering.len() as u32);
    for &t in &p.evade_recovering {
        h.write_u32(t);
    }
    h.write_u32(u32::from(p.held));
    match p.skill {
        Some(slot) => {
            h.write_bool(true);
            h.write_u32(u32::from(slot.index));
            h.write_u32(u32::from(slot.level));
            h.write_u32(slot.cooldown_left);
        }
        None => h.write_bool(false),
    }
    match p.pull {
        Some(b) => {
            h.write_bool(true);
            h.write_i32(b.at.x.0);
            h.write_i32(b.at.y.0);
            h.write_i32(b.radius);
            h.write_i32(b.strength);
            h.write_u32(b.ticks_left);
        }
        None => h.write_bool(false),
    }
    match p.shield {
        Some(sh) => {
            h.write_bool(true);
            h.write_u32(sh.amount);
            h.write_u32(sh.ticks_left);
        }
        None => h.write_bool(false),
    }
    h.write_u32(count(&p.orbits));
    for o in &p.orbits {
        h.write_u32(u32::from(o.reward));
        h.write_u32(u32::from(o.orbs));
        h.write_u32(u32::from(o.round));
        h.write_u32(o.cooldown_left);
        h.write_i32(o.angle);
    }
    h.write_u32(p.range_pct);
    let c = &p.combo;
    h.write_u32(c.path);
    h.write_u32(u32::from(c.depth));
    h.write_bool(c.dash);
    h.write_u32(match c.buffered {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    });
    h.write_bool(c.running);
    h.write_u32(c.window_left);
    h.write_u32(c.dash_strike_left);
    match c.charge {
        Some(held) => {
            h.write_bool(true);
            h.write_u32(held);
        }
        None => h.write_bool(false),
    }
    h.write_bool(c.hand);
    h.write_u32(c.auto_path);
    h.write_u32(u32::from(c.auto_depth));
    // The charge levels (ADR-114), only once a step was charged, so that the hashes of other
    // runs stay as they were.
    if c.levels != 0 || c.auto_levels != 0 {
        h.write_u32(c.levels);
        h.write_u32(c.auto_levels);
        h.write_bool(c.charge_auto);
    }
    // The hold, only while one is held (ADR-084 point 6), so that the hashes of other runs stay
    // as they were.
    if c.hold.is_some() || c.released {
        h.write_bool(c.released);
        h.write_u32(c.hold.unwrap_or(0));
    }
    match p.dash {
        Some(DashState { dir, done, left }) => {
            h.write_bool(true);
            h.write_i32(dir.x);
            h.write_i32(dir.y);
            h.write_u32(done);
            h.write_u32(left);
        }
        None => h.write_bool(false),
    }
    h.write_u32(p.aggro);
    hash_statuses(&mut h, &p.statuses);
    match &s.boss {
        Some(b) => {
            h.write_bool(true);
            h.write_u32(b.id.0);
            h.write_i32(b.at.x.0);
            h.write_i32(b.at.y.0);
            h.write_u32(b.hp);
            h.write_u32(b.max_hp);
            h.write_u32(b.defence);
            hash_statuses(&mut h, &b.statuses);
            h.write_u32(b.phase);
            h.write_u32(b.phase_age);
            h.write_u32(b.invulnerable);
            h.write_u32(b.step);
            h.write_u32(b.wait_left);
            match b.order {
                Some(o) => {
                    h.write_bool(true);
                    h.write_i32(o.to.x.0);
                    h.write_i32(o.to.y.0);
                    h.write_u32(o.ticks_left);
                }
                None => h.write_bool(false),
            }
            h.write_u32(count(&b.rules));
            for r in &b.rules {
                h.write_u32(r.cooldown_left);
                h.write_u32(r.charges_left);
                h.write_bool(r.fired);
            }
            h.write_u32(count(&b.parts));
            for part in &b.parts {
                h.write_u32(part.id.0);
                h.write_u32(part.hp);
                h.write_u32(part.max_hp);
                h.write_u32(part.defence);
                hash_statuses(&mut h, &part.statuses);
            }
            // Only for a boss that calls its elites, so other bosses hash as before.
            if let Some(called) = b.elites_called {
                h.write_bool(called);
                h.write_u32(b.age);
            }
        }
        None => h.write_bool(false),
    }
    h.write_u32(s.next_id);
    h.write_u32(count(&s.spawned));
    for n in &s.spawned {
        h.write_u32(u32::from(*n));
    }
    h.write_u32(count(&s.companions));
    for c in &s.companions {
        hash_agent(&mut h, c);
    }
    h.write_u32(count(&s.enemies));
    for e in &s.enemies {
        hash_agent(&mut h, e);
    }
    h.write_u32(count(&s.tasks));
    for t in &s.tasks {
        hash_task(&mut h, t);
    }
    h.write_u32(count(&s.runs));
    for r in &s.runs {
        hash_run(&mut h, world, r);
    }
    h.write_u32(count(&s.bullets));
    for b in &s.bullets {
        h.write_i32(b.at.x.0);
        h.write_i32(b.at.y.0);
        h.write_i32(b.vx.0);
        h.write_i32(b.vy.0);
        h.write_i32(b.dir.x);
        h.write_i32(b.dir.y);
        h.write_u32(u32::from(b.style));
        h.write_u32(b.owner.0);
        h.write_bool(b.friendly);
        h.write_u32(b.damage);
        h.write_bool(b.breaks_charge);
        hash_smash(&mut h, b.smash);
        match &b.hook {
            Some(hook) => {
                h.write_bool(true);
                hash_path(&mut h, &hook.path);
                h.write_u32(hook.left);
                h.write_u32(hook.owner.0);
                hash_aim(&mut h, hook.aim);
                h.write_bool(hook.friendly);
                h.write_u32(hook.damage);
                h.write_bool(hook.breaks_charge);
                hash_smash(&mut h, hook.smash);
            }
            None => h.write_bool(false),
        }
    }
    // Last, and only in a stage with levels, so that the hashes of stages without stay as they
    // were.
    if let Some(p) = &s.progress {
        h.write_u32(p.level);
        h.write_u32(p.xp);
        h.write_u32(p.points);
        h.write_u32(count(&p.spent));
        for n in &p.spent {
            h.write_u32(u32::from(*n));
        }
        for t in p.totals {
            h.write_u32(t);
        }
    }
    if let Some(stardust) = s.stardust {
        h.write_u32(stardust);
    }
    // After it, and only in a stage with stars, for the same reason.
    if let Some(f) = &s.stars {
        h.write_u32(f.misses);
        h.write_u32(count(&f.on_map));
        for star in &f.on_map {
            h.write_u32(star.id.0);
            h.write_i32(star.at.x.0);
            h.write_i32(star.at.y.0);
            h.write_u32(u32::from(star.rarity.index()));
        }
        h.write_u32(count(&f.due));
        for d in &f.due {
            h.write_u32(d.kind as u32);
            h.write_u32(u32::from(d.rarity.index()));
        }
        match &f.offer {
            Some(o) => {
                h.write_bool(true);
                h.write_u32(o.kind as u32);
                h.write_u32(u32::from(o.rarity.index()));
                h.write_u32(count(&o.candidates));
                for c in &o.candidates {
                    h.write_u32(u32::from(*c));
                }
            }
            None => h.write_bool(false),
        }
        h.write_u32(f.rerolls_left);
        h.write_u32(f.banishes_left);
        h.write_u32(count(&f.pool));
        for i in &f.pool {
            h.write_u32(u32::from(*i));
        }
        h.write_u32(count(&f.taken));
        for n in &f.taken {
            h.write_u32(u32::from(*n));
        }
        h.write_u32(count(&f.banished));
        for b in &f.banished {
            h.write_bool(*b);
        }
        // Then the sets with a member held, and nothing while none has, so that a run without
        // sets hashes as it did before them.
        let held = f.set_counts.iter().filter(|n| **n > 0).count();
        if held > 0 {
            h.write_u32(count(&f.set_counts));
            for (i, n) in f.set_counts.iter().enumerate() {
                if *n > 0 {
                    h.write_u32(u32::try_from(i).unwrap_or(u32::MAX));
                    h.write_u32(u32::from(*n));
                }
            }
        }
    }
    // After it, and only in a stage with healing, for the same reason: the heal pickups on the
    // map, in the order they dropped.
    if let Some(heals) = &s.heal_pickups {
        h.write_u32(count(heals));
        for pickup in heals {
            h.write_u32(pickup.id.0);
            h.write_i32(pickup.at.x.0);
            h.write_i32(pickup.at.y.0);
        }
    }
    // After it, and only in a stage that gives expertise points, for the same reason: the
    // gauge, then which attacks and bullets are a combo step's, which matters only here.
    if let Some(g) = &s.expertise {
        h.write_u32(g.level);
        h.write_u32(g.points);
        h.write_u32(g.next);
        h.write_u32(g.offered);
        match &g.offer {
            Some(o) => {
                h.write_bool(true);
                h.write_u32(o.level);
                h.write_u32(o.kind as u32);
                h.write_u32(count(&o.candidates));
                for c in &o.candidates {
                    h.write_u32(u32::from(*c));
                }
            }
            None => h.write_bool(false),
        }
        h.write_u32(count(&g.taken));
        for n in &g.taken {
            h.write_u32(u32::from(*n));
        }
        h.write_u32(g.range_pct);
        h.write_bool(g.style.is_some());
        h.write_u32(g.style.map_or(0, u32::from));
        h.write_u32(count(&g.path));
        for i in &g.path {
            h.write_u32(u32::from(*i));
        }
        h.write_u32(count(&s.tasks));
        for t in &s.tasks {
            h.write_bool(t.combo);
        }
        h.write_u32(count(&s.bullets));
        for b in &s.bullets {
            h.write_bool(b.combo);
            h.write_bool(b.hook.as_ref().is_some_and(|hook| hook.combo));
        }
    }
    // Last, and only while an attack or a bullet carries a damage type (ADR-100), so that the
    // hashes of runs without types stay as they were: each one's type, 0 for neutral. A run's
    // phases are content, which the content hash covers.
    let typed = s.tasks.iter().any(|t| t.damage_type.is_some())
        || s.bullets.iter().any(|b| {
            b.damage_type.is_some() || b.hook.as_ref().is_some_and(|k| k.damage_type.is_some())
        });
    if typed {
        h.write_u32(count(&s.tasks));
        for t in &s.tasks {
            h.write_u32(type_code(t.damage_type));
        }
        h.write_u32(count(&s.bullets));
        for b in &s.bullets {
            h.write_u32(type_code(b.damage_type));
            h.write_u32(b.hook.as_ref().map_or(0, |k| type_code(k.damage_type)));
        }
    }
    // Each bullet's ticks left, and only while one has a lifetime (core#147), so that the hashes
    // of runs without one stay as they were.
    if s.bullets.iter().any(|b| b.life != UNLIMITED) {
        h.write_u32(count(&s.bullets));
        for b in &s.bullets {
            h.write_u32(b.life);
        }
    }
    // Last, and only while the main character holds a type bonus (ADR-100), so that the hashes
    // of runs without one stay as they were: its bonus for each type, then each run's, fixed at
    // its start. A run can hold one only once the main character does: bonuses never go.
    if s.player.type_bonus_pct.iter().any(|pct| *pct > 0) {
        for pct in s.player.type_bonus_pct {
            h.write_u32(pct);
        }
        h.write_u32(count(&s.runs));
        for r in &s.runs {
            for pct in r
                .type_bonus_pct
                .as_deref()
                .unwrap_or(&[0; DamageType::ALL.len()])
            {
                h.write_u32(*pct);
            }
        }
    }
    // Last, and only while the Main weapon holds a rune (ADR-099), so that the hashes of runs
    // without one stay as they were: each rune held and its cooldown left, then where each
    // on-hit rune's attack in progress is centred, by its place in `runs`, where each thrown
    // one turns and each homing missile's target. Runes never go.
    if !s.player.runes.is_empty() {
        h.write_u32(count(&s.player.runes));
        for r in &s.player.runes {
            h.write_u32(u32::from(r.rune.index()));
            h.write_u32(r.cooldown_left);
        }
        let centred = || {
            s.runs
                .iter()
                .enumerate()
                .filter_map(|(i, r)| Some((i, r.at?)))
        };
        h.write_u32(u32::try_from(centred().count()).unwrap_or(u32::MAX));
        for (i, at) in centred() {
            h.write_u32(u32::try_from(i).unwrap_or(u32::MAX));
            h.write_i32(at.x.0);
            h.write_i32(at.y.0);
        }
        // Then each thrown rune's turn point, by its place in `runs`.
        let thrown = || {
            s.runs
                .iter()
                .enumerate()
                .filter_map(|(i, r)| Some((i, r.throw?)))
        };
        h.write_u32(u32::try_from(thrown().count()).unwrap_or(u32::MAX));
        for (i, to) in thrown() {
            h.write_u32(u32::try_from(i).unwrap_or(u32::MAX));
            h.write_i32(to.x.0);
            h.write_i32(to.y.0);
        }
        // Then each homing missile's target, by its place in `runs`.
        let homing = || {
            s.runs
                .iter()
                .enumerate()
                .filter_map(|(i, r)| Some((i, r.homing?)))
        };
        h.write_u32(u32::try_from(homing().count()).unwrap_or(u32::MAX));
        for (i, target) in homing() {
            h.write_u32(u32::try_from(i).unwrap_or(u32::MAX));
            h.write_u32(target.0);
        }
    }
    // Last, and only while the main character holds Rune Haste (ADR-099), so that the hashes of
    // runs without it stay as they were: what the picks add up to. Haste never goes.
    if s.player.rune_haste_pct > 0 {
        h.write_u32(s.player.rune_haste_pct);
    }
    // The same for the pickup radius and the skill cooldown rewards (core NS2a).
    if s.player.pickup_radius_pct > 0 {
        h.write_u32(s.player.pickup_radius_pct);
    }
    if s.player.skill_cooldown_pct > 0 {
        h.write_u32(s.player.skill_cooldown_pct);
    }
    if combat::held(&s.player) {
        h.write_u32(s.player.crit_chance_pct);
        h.write_u32(s.player.crit_damage_pct);
        h.write_u32(s.player.echo_hit_pct);
        h.write_u32(s.player.burst_pct);
        h.write_u32(s.player.burst_radius);
        h.write_u32(s.player.burst_depth);
        // Only a run that holds some, so every other hash is what it was.
        if s.player.burst_depth_extra != 0 {
            h.write_u32(s.player.burst_depth_extra);
        }
        h.write_u32(u32::try_from(s.echoes.len()).unwrap_or(u32::MAX));
        for echo in &s.echoes {
            h.write_u32(echo.source.0);
            h.write_u32(echo.target.0);
            h.write_u32(echo.damage);
        }
    }
    if let Some(f) = s.stars.as_ref().filter(|f| f.extra_candidates > 0) {
        h.write_u32(u32::from(f.extra_candidates));
    }
    if s.player.kill_heal_every > 0 {
        h.write_u32(s.player.kill_heal);
        h.write_u32(s.player.kill_heal_every);
        h.write_u32(s.player.kills_counted);
    }
    // Last of all, and only in a run with a horde: the bosses come so far, and which pool
    // entries are drawn.
    if let Some(horde) = &s.horde {
        h.write_u32(horde.spawned);
        h.write_u32(count(&horde.used));
        for used in &horde.used {
            h.write_bool(*used);
        }
    }
    // After it, and only while a zone is alive or an attack holds the spot where its target
    // stood (core Z1), so that the hashes of runs without either stay as they were.
    if !s.zones.is_empty() {
        h.write_u32(count(&s.zones));
        for z in &s.zones {
            hash_zone(&mut h, z);
        }
    }
    if s.runs.iter().any(|r| r.spot.is_some()) || s.tasks.iter().any(|t| t.spot.is_some()) {
        h.write_u32(count(&s.runs));
        for r in &s.runs {
            hash_spot(&mut h, r.spot);
        }
        h.write_u32(count(&s.tasks));
        for t in &s.tasks {
            hash_spot(&mut h, t.spot);
        }
    }
    h.finish()
}

fn hash_spot(h: &mut StateHasher, spot: Option<Point>) {
    h.write_bool(spot.is_some());
    if let Some(at) = spot {
        h.write_i32(at.x.0);
        h.write_i32(at.y.0);
    }
}

fn hash_zone(h: &mut StateHasher, z: &crate::engine::ZoneState) {
    match &z.shape {
        ZoneShape::Circle { radius } => {
            h.write_u32(0);
            h.write_i32(radius.0);
        }
        ZoneShape::Arc { radius, half_angle } => {
            h.write_u32(1);
            h.write_i32(radius.0);
            h.write_i32(*half_angle);
        }
        ZoneShape::Line { length, width } => {
            h.write_u32(2);
            h.write_i32(length.0);
            h.write_i32(width.0);
        }
    }
    h.write_u32(z.owner.0);
    h.write_bool(z.friendly);
    h.write_i32(z.at.x.0);
    h.write_i32(z.at.y.0);
    h.write_i32(z.dir.x);
    h.write_i32(z.dir.y);
    h.write_u32(z.wind_left);
    h.write_u32(z.active_left);
    h.write_u32(z.damage);
    h.write_u32(type_code(z.damage_type));
    h.write_bool(z.breaks_charge);
    h.write_u32(count(&z.hits));
    for (id, tick) in &z.hits {
        h.write_u32(id.0);
        h.write_u32(*tick);
    }
}

/// A damage type in the state hash: its place in `DamageType::ALL` plus 1, or 0 for neutral.
fn type_code(ty: Option<DamageType>) -> u32 {
    ty.and_then(|ty| DamageType::ALL.iter().position(|t| *t == ty))
        .map_or(0, |i| u32::try_from(i + 1).unwrap_or(u32::MAX))
}
