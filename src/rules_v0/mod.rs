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
//! 4. bullets whose `on_bullet` wait is over start their attacks, then every running attack
//!    is carried out up to its next wait (`attacks`), making bullets;
//! 5. bullets move and leave the map;
//! 6. collisions: first the hit boxes of active phases (`hit_boxes`), then bullets; those of
//!    allies hurt enemies and the boss, those of enemies the main character and the companions;
//! 7. the outcome is decided: death or running out of ticks fails, an empty stage clears;
//! 8. if the run goes on or is cleared, the first offer due opens, and the world stops until it
//!    is answered: the boss offer, then the expertise levels' in level order, then the stars' in
//!    the order taken; a fail opens none.
//!
//! In a stage whose enemies or boss give expertise points, each hit and kill by a combo step in
//! step 6 adds them to the one expertise gauge, and its levels are reached at the end of step 6
//! (`expertise`). Each level reached is due an offer for the weapon ADR-078's table gives: its
//! common options, its three styles, or the chosen Main style's upgrades or capstones.
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
mod attacks;
mod boss;
mod entity;
mod expertise;
mod grid;
mod hit_boxes;
mod horde;
mod levels;
mod orbit;
mod player;
mod push;
mod runes;
mod skill;
mod stars;

pub(crate) use player::{charge_view, combo_view};
mod select;

use crate::attack::{DamageType, MatchUp, Profile};
use crate::behaviour::BossCall;
use crate::engine::{
    AgentKey, AgentSource, BossState, BulletState, DashState, Dir, Load, MoveOrder, Outcome,
    PartState, PlayerState, Progress, Pull, StarField, State,
};
use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::fixed::{Fx, Point};
use crate::hash::StateHasher;
use crate::id::EntityId;
use crate::input::Input;
use crate::rewards::Rewards;
use crate::rng::SplitMix64;
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::rune::Rune;
use crate::snapshot::OfferOption;
use crate::stage::Stage;
use crate::trig;
use crate::validate::limits::DAMAGE_SCALE;

pub(crate) use agents::agent_def;
use entity::{Who, scale_damage, status_pct};
pub(crate) use expertise::offer_view as expertise_offer_view;
pub(crate) use expertise::styles_view as expertise_styles_view;
use grid::Grid;
pub(crate) use horde::spawn_boss;
use horde::spawn_elite;
pub(crate) use levels::{spend, xp_next};
pub(crate) use stars::offer_view;

pub(crate) const MAX_ENEMIES_ALIVE: u32 = 500;
pub(crate) const MAX_BULLETS_ALIVE: u32 = 4000;
pub(crate) const MAX_BULLET_SPAWNS_PER_TICK: u32 = 200;
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
/// The damage an attack does when an enemy or the boss makes it and the attack names none: 1 at
/// the design values, scaled (ADR-103).
const ENEMY_ATTACK_DAMAGE: u32 = DAMAGE_SCALE;

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
}

/// The state before the first tick, its generator seeded with `seed`: the stage's own for a
/// stage, the setup's for a run (ADR-051).
pub(crate) fn initial_state(
    stage: &Stage,
    content: &RunContent,
    pool: RunPool,
    seed: u64,
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
        .map(|(i, def)| {
            let id = EntityId(next_id);
            next_id += 1;
            let key = AgentKey {
                source: AgentSource::Companion,
                index: u32::try_from(i).unwrap_or(u32::MAX),
                rule: 0,
            };
            // Orbiting companions start a quarter turn apart.
            let angle = i32::try_from(i).unwrap_or(0) * 90 * trig::PER_DEGREE;
            agents::new_agent(id, key, def, start, angle)
        })
        .collect();
    // The boss's parts come after the companions in the order of entity IDs.
    let boss = stage
        .boss
        .as_ref()
        .zip(boss_id)
        .map(|(b, id)| boss_state(b, id, &mut next_id));
    State {
        tick: 0,
        rng: SplitMix64::new(seed),
        outcome: Outcome::Running,
        player: PlayerState {
            at: start,
            hp: stage.player.hp,
            max_hp: stage.player.hp,
            defence: stage.player.defence,
            invulnerable: 0,
            // Up the map on every map, landscape ones included; a stage cannot set it yet.
            facing: Dir::UP,
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
            type_bonus_pct: [0; DamageType::ALL.len()],
            runes: Vec::new(),
            aggro: 0,
            statuses: Vec::new(),
        },
        boss,
        companions,
        enemies: Vec::new(),
        tasks: Vec::new(),
        runs: Vec::new(),
        bullets: Vec::new(),
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
            rerolls_left: u32::from(st.rerolls) + u32::from(pool.extra_rerolls),
            banishes_left: u32::from(st.banishes),
            pool: pool.rewards,
            taken: vec![0; rewards],
            banished: vec![false; rewards],
        }),
        expertise: expertise::initial(stage, content.expertise()),
        match_ups: *content.match_ups(),
        runes: content.runes().clone(),
        horde: None,
    }
}

/// The big patterns the boss has announced (ADR-103), for the snapshot.
pub(crate) fn announced_patterns(stage: &Stage, s: &State) -> Vec<crate::snapshot::PatternView> {
    boss::announced(stage, s)
}

/// The boss before its first tick, its parts taking the entity IDs from `next_id` on. The
/// first phase begins on the first tick (see `boss::act`).
pub(crate) fn boss_state(b: &crate::stage::Boss, id: EntityId, next_id: &mut u32) -> BossState {
    BossState {
        id,
        at: b.spawn,
        hp: b.hp,
        max_hp: b.hp,
        defence: b.defence,
        statuses: Vec::new(),
        phase: 0,
        phase_age: 0,
        invulnerable: 0,
        step: 0,
        wait_left: 0,
        order: None,
        rules: Vec::new(),
        elites_called: b.call_elites.map(|_| false),
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
                    hp: p.hp,
                    max_hp: p.hp,
                    defence: p.defence,
                    offset: p.offset,
                    radius: p.radius.0,
                    statuses: Vec::new(),
                }
            })
            .collect(),
    }
}

pub(crate) fn step(
    stage: &Stage,
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
    let mut spawns_left = MAX_BULLET_SPAWNS_PER_TICK;

    spawn_waves(stage, s);
    spawn_elite(stage, s, events);
    let dots = entity::tick_statuses_and_aggro(s);
    runes::tick(s);
    attacks::advance_runs(s, events);
    player::update(stage, rewards, s, input, events);
    if !s.player.runes.is_empty() {
        runes::fly(s);
    }
    orbit::act(stage, rewards, s, events);
    boss::act(stage, s, events);
    boss::call_elites(stage, s, events);
    agents::act(stage, s, events);
    push::apply(stage, s);
    skill::pull(stage, s);
    if !s.player.runes.is_empty() {
        runes::steer(s);
    }
    attacks::fire_hooks(s);
    let (tasks, emitter_steps) = attacks::run(s, &mut spawns_left);
    *load = Load {
        enemies: count(&s.enemies),
        tasks,
        bullets: count(&s.bullets),
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
    stage: &Stage,
    rewards: &Rewards,
    expertise: &Expertise,
    s: &mut State,
    events: &mut Vec<DomainEvent>,
) {
    stars::open_due(rewards, s, events, true);
    expertise::open_due(stage, expertise, s, events);
    stars::open_due(rewards, s, events, false);
}

/// The options of the open offer, in index order; empty without one.
pub(crate) fn offer_options(
    stage: &Stage,
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
    stage: &Stage,
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

fn spawn_waves(stage: &Stage, s: &mut State) {
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
        spawn_enemy(stage, s, key, &wave.enemy, at);
    }
}

/// A point `ring` from the main character, at an angle drawn from the run's generator, moved
/// onto the map.
fn around_player(stage: &Stage, s: &mut State, ring: Fx) -> Point {
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
pub(super) fn spawn_enemy(
    stage: &Stage,
    s: &mut State,
    key: AgentKey,
    def: &crate::behaviour::Agent,
    at: Point,
) -> bool {
    let room = count(&s.enemies) < MAX_ENEMIES_ALIVE
        && (def.elite.is_none()
            || s.enemies.iter().filter(|e| e.elite.is_some()).count()
                < usize::from(stage.elite_cap()));
    if !room {
        return false;
    }
    let id = EntityId(s.next_id);
    s.next_id += 1;
    let mut enemy = agents::new_agent(id, key, def, at, 0);
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

fn move_bullets(stage: &Stage, s: &mut State) {
    for b in &mut s.bullets {
        b.at.x = b.at.x.saturating_add(b.vx);
        b.at.y = b.at.y.saturating_add(b.vy);
    }
    let holds = stage.map().holds();
    s.bullets.retain(|b| holds(b.at));
}

/// Two circles overlap (touching counts), in 64-bit fixed point.
fn overlaps(a: Point, ra: i32, b: Point, rb: i32) -> bool {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    let r = i64::from(ra) + i64::from(rb);
    dx * dx + dy * dy <= r * r
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
    /// The Sub weapon's pull, for a combo part's hit (ADR-070).
    pull: Option<Pull>,
    /// A combo step's hit, which adds expertise points (ADR-078 §2).
    combo: bool,
    /// The rune that dealt it (ADR-099), which the `Hit` event reports.
    rune: Option<Rune>,
}

fn collide(stage: &Stage, s: &mut State, dots: bool, events: &mut Vec<DomainEvent>) {
    if dots {
        entity::tick_dots(stage, s, events);
    }
    // The enemies do not move while hit boxes and bullets land, so one grid, built for the first
    // friendly hit box or bullet, serves all of them; a tick without either never builds it.
    let mut grid: Option<Grid> = None;
    let boss_alive = s.boss.as_ref().is_some_and(|b| b.hp > 0);
    hit_boxes::resolve(stage, s, &mut grid, events);
    let mut consumed = Vec::with_capacity(s.bullets.len());
    for i in 0..s.bullets.len() {
        let b = &s.bullets[i];
        let shot = Shot {
            at: b.at,
            owner: b.owner,
            friendly: b.friendly,
            damage: b.damage,
            damage_type: b.damage_type,
            breaks_charge: b.breaks_charge,
            pull: b.pull,
            combo: b.combo,
            rune: None,
        };
        consumed.push(if shot.friendly {
            let grid = grid.get_or_insert_with(|| enemy_grid(stage, s));
            hit_enemy_side(stage, s, grid, shot, events)
        } else {
            hit_ally_side(stage, s, shot, events)
        });
    }
    let mut flags = consumed.into_iter();
    s.bullets.retain(|_| !flags.next().unwrap_or(false));
    s.companions.retain(|c| c.hp > 0);
    levels::credit_defeated(stage, s, events);
    stars::drop_from_defeated(stage, s, events);
    s.enemies.retain(|e| e.hp > 0);
    if boss_alive && s.boss.as_ref().is_some_and(|b| b.hp == 0) {
        stars::boss_killed(stage, s);
    }
    stars::pick_up(stage, s, events);
}

/// The enemies as circles, in index order, for friendly bullets to land on.
fn enemy_grid(stage: &Stage, s: &State) -> Grid {
    let circles = s
        .enemies
        .iter()
        .map(|e| (e.at, agent_def(stage, e.key).radius.0))
        .collect();
    Grid::new(circles, PLAYER_BULLET_RADIUS)
}

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, changed by its
/// match-up, reduced by its defence, reported, and credited to the shooter's aggro. Death is
/// reported too. A combo step's hit adds expertise points (`expertise`), puts the
/// damage-over-time runes' statuses on a target it leaves standing, and fires the on-hit runes
/// that are ready (`runes`).
fn damage(stage: &Stage, s: &mut State, b: Shot, target: Who, events: &mut Vec<DomainEvent>) {
    // A shrine, or a mimic in disguise, is an object: nothing damages it (ADR-090).
    if matches!(target, Who::Enemy(i) if s.enemies[i].shrine.is_some() || s.enemies[i].hidden()) {
        return;
    }
    let vulnerable = status_pct(s.statuses(target), crate::engine::StatusKind::Vulnerable);
    let match_up = profile(stage, s, target).map_or(MatchUp::None, |p| p.match_up(b.damage_type));
    let dealt = scale_damage(
        b.damage,
        0,
        0,
        vulnerable,
        s.match_ups.pct(match_up),
        s.defence(target),
    );
    let id = s.id_of(target);
    let hp = s.hp_mut(target);
    *hp = hp.saturating_sub(dealt);
    let dead = *hp == 0;
    events.push(DomainEvent::Hit {
        source: b.owner,
        target: id,
        damage: dealt,
        match_up,
        rune: b.rune,
    });
    if dead {
        // A part breaks; everything else dies.
        events.push(match (target, s.boss.as_ref()) {
            (Who::BossPart(i), Some(boss)) => DomainEvent::PartBroken {
                part: boss.parts[i].name.clone(),
            },
            _ => DomainEvent::Died { entity: id },
        });
    }
    s.raise_aggro(b.owner, dealt.saturating_mul(AGGRO_PER_DAMAGE));
    if b.combo {
        expertise::credit(stage, s, target, dead);
        if !s.player.runes.is_empty() {
            if !dead {
                runes::apply_dots(s, target, events);
            }
            let at = s.at(target);
            runes::on_hit(stage, s, id, at, events);
        }
    }
}

/// The profile of a target (ADR-100): an enemy's or an elite's own, the boss's, or a part's own.
/// The main character and the companions have none.
fn profile<'a>(stage: &'a Stage, s: &State, target: Who) -> Option<&'a Profile> {
    match target {
        Who::Enemy(i) => agent_def(stage, s.enemies[i].key).profile.as_ref(),
        Who::Boss => stage.boss.as_ref()?.profile.as_ref(),
        Who::BossPart(i) => stage.boss.as_ref()?.parts.get(i)?.profile.as_ref(),
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
    stage: &Stage,
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
        if let Some(pull) = b.pull {
            pull_enemy(stage, s, i, pull);
        }
        return true;
    }
    let Some((def, state)) = stage.boss.as_ref().zip(s.boss.as_ref()) else {
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
pub(super) fn pull_enemy(stage: &Stage, s: &mut State, i: usize, pull: Pull) {
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
    let contact = i64::from(PLAYER_RADIUS) + i64::from(agent_def(stage, e.key).radius.0);
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

/// Damage to the main character, raised by its vulnerability and reduced by its defence; it is
/// then invulnerable for a while.
fn hurt_player(s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) {
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
        player::break_charge(s, events);
    }
}

/// `b` with the damage a pack leader's aura adds (ADR-090), for a hit of an enemy's on an ally:
/// if the enemy that dealt it is still on the map and not an elite, the highest `damage_percent`
/// among the auras whose leader's centre is within `radius` of its centre (the edge counts)
/// raises the damage to `damage * (100 + percent) / 100`, rounded down. Every leader still in
/// the enemies counts, in index order, so a leader killed in this tick's collisions still gives
/// its aura through the rest of the tick, and none from the next, when the dead are removed.
/// An enemy that has since left the map or died earlier gets none. No random number is drawn.
fn aided(stage: &Stage, s: &State, b: Shot) -> Shot {
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
    stage: &'a Stage,
    s: &'a State,
) -> impl Iterator<Item = (Point, crate::behaviour::Aura)> + 'a {
    s.enemies
        .iter()
        .filter(|l| l.elite.is_some())
        .filter_map(|l| {
            agent_def(stage, l.key)
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
fn hit_ally_side(stage: &Stage, s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) -> bool {
    let p = &s.player;
    if p.invulnerable == 0 && p.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, p.at, PLAYER_RADIUS) {
        hurt_player(s, aided(stage, s, b), events);
        return true;
    }
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let radius = agent_def(stage, c.key).radius.0;
        if c.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, c.at, radius) {
            damage(stage, s, aided(stage, s, b), Who::Companion(i), events);
            return true;
        }
    }
    false
}

/// The main character's death wins over a clear on the same tick.
fn decide_outcome(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
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
    hash_pull(h, t.pull);
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
fn hash_run(h: &mut StateHasher, r: &crate::engine::AttackRun) {
    h.write_u32(r.owner.0);
    h.write_u32(u32::from(r.slot));
    // A combo step's phases may be shortened by attack speed (ADR-070), so their lengths are
    // written.
    h.write_u32(count(&r.phases));
    for p in r.phases.iter() {
        h.write_u32(p.ticks);
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
    hash_pull(h, r.pull);
}

fn hash_pull(h: &mut StateHasher, pull: Option<Pull>) {
    match pull {
        Some(Pull { distance, ticks }) => {
            h.write_bool(true);
            h.write_i32(distance);
            h.write_u32(ticks);
        }
        None => h.write_bool(false),
    }
}

/// The state hash: FNV-1a 64 over the whole state at fixed widths, field by field in the order
/// of the `State` types, with each list preceded by its length (ADR-019).
pub(crate) fn state_hash(s: &State) -> u64 {
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
        hash_run(&mut h, r);
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
        hash_pull(&mut h, b.pull);
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
                hash_pull(&mut h, hook.pull);
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
                h.write_u32(o.weapon as u32);
                h.write_u32(o.kind as u32);
                h.write_u32(count(&o.candidates));
                for c in &o.candidates {
                    h.write_u32(u32::from(*c));
                }
            }
            None => h.write_bool(false),
        }
        for taken in [&g.main_taken, &g.sub_taken] {
            h.write_u32(count(taken));
            for n in taken.iter() {
                h.write_u32(u32::from(*n));
            }
        }
        h.write_u32(g.range_pct);
        for style in [g.main_style, g.sub_style] {
            h.write_bool(style.is_some());
            h.write_u32(style.map_or(0, u32::from));
        }
        h.write_u32(count(&g.main_path));
        for i in &g.main_path {
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
    // Last of all, and only in a run with a horde: the bosses come so far, and which pool
    // entries are drawn.
    if let Some(horde) = &s.horde {
        h.write_u32(horde.spawned);
        h.write_u32(count(&horde.used));
        for used in &horde.used {
            h.write_bool(*used);
        }
    }
    h.finish()
}

/// A damage type in the state hash: its place in `DamageType::ALL` plus 1, or 0 for neutral.
fn type_code(ty: Option<DamageType>) -> u32 {
    ty.and_then(|ty| DamageType::ALL.iter().position(|t| *t == ty))
        .map_or(0, |i| u32::try_from(i + 1).unwrap_or(u32::MAX))
}
