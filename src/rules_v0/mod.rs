//! The rules of `sim_version` 0 (ADR-035): a development version that changes in place until
//! the freeze at P3, with the hash check across CPUs never relaxed (ADR-019).
//!
//! What a tick does, in this order:
//! 1. the tick counter advances; waves spawn enemies; statuses run out and aggro fades;
//! 2. the main character turns to its stick and, while aim assist or auto mode is on, picks a
//!    target (ADR-072, `select`); it starts a ready evade on a press (cancelling its combo
//!    step and breaking a charge), walks its combo: a buffered press, or else auto mode's
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
//! 8. if the run goes on, the first offer due opens, and the world stops until it is answered;
//!    a clear opens only the boss offer of the boss kill that made it.
//!
//! In a stage with levels, defeated enemies give their experience as they are removed in
//! step 6, and each level-up gives a stat point (`levels`); points are spent between ticks. In
//! a stage with stars, defeated enemies then drop falling stars, and the main character takes
//! those it reaches; each opens an offer in step 8, answered between ticks (`stars`).
//!
//! Not yet in this version: summons by companions; the stage skill (ADR-075), whose input is
//! recorded and does nothing yet.
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
mod grid;
mod hit_boxes;
mod levels;
mod player;
mod push;
mod stars;

pub(crate) use player::{charge_view, combo_view};
mod select;

use crate::engine::{
    AgentKey, AgentSource, BossState, BulletState, DashState, Dir, Load, MoveOrder, Outcome,
    PartState, PlayerState, Progress, Pull, StarField, State,
};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::hash::StateHasher;
use crate::id::EntityId;
use crate::input::Input;
use crate::rewards::Rewards;
use crate::rng::SplitMix64;
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::stage::Stage;
use crate::trig;

pub(crate) use agents::agent_def;
use entity::{Who, scale_damage, status_pct};
use grid::Grid;
pub(crate) use levels::{spend, xp_next};
pub(crate) use stars::{choose, offer_view, options as offer_options};

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

/// Aggro (ADR-036): raised by damage dealt, by buffs and by heals; fading every tick.
const AGGRO_PER_DAMAGE: u32 = 1;
const AGGRO_PER_BUFF: u32 = 40;
const AGGRO_PER_HEAL_POINT: u32 = 2;
const AGGRO_DECAY: u32 = 1;
const MAX_AGGRO: u32 = 1_000_000;

pub(crate) fn bullet_radius(friendly: bool) -> i32 {
    if friendly {
        PLAYER_BULLET_RADIUS
    } else {
        ENEMY_BULLET_RADIUS
    }
}

/// What a run brings to its offers: the rewards of its pool, by their place in the rewards file,
/// and the rerolls its setup adds to the stage's (ADR-066 §1).
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
    let boss = stage.boss.as_ref().zip(boss_id).map(|(b, id)| BossState {
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
        // The first phase begins on the first tick (see `boss::act`).
        rules: Vec::new(),
        elites_called: b.call_elites.map(|_| false),
        parts: b
            .parts
            .iter()
            .map(|p| {
                let part_id = EntityId(next_id);
                next_id += 1;
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
    });
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
    }
}

pub(crate) fn step(
    stage: &Stage,
    rewards: &Rewards,
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
    entity::tick_statuses_and_aggro(s);
    attacks::advance_runs(s, events);
    player::update(stage, s, input, events);
    boss::act(stage, s, events);
    boss::call_elites(stage, s, events);
    agents::act(stage, s, events);
    push::apply(stage, s);
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
    collide(stage, s, events);
    decide_outcome(stage, s, events);
    match s.outcome {
        Outcome::Running => stars::open_due(rewards, s, events),
        Outcome::Cleared => {
            stars::run_over(s);
            stars::open_due(rewards, s, events);
        }
        Outcome::Failed => {}
    }
    debug_assert!(
        entity::enemies_in_id_order(s),
        "enemies out of entity-ID order"
    );
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
        spawn_enemy(stage, s, key, &wave.enemy, wave.spawn);
    }
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
        el.called = s.boss.as_ref().and_then(|b| b.elites_called) == Some(true);
    }
    s.enemies.push(enemy);
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
    breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hit (ADR-070).
    pull: Option<Pull>,
}

fn collide(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
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
            breaks_charge: b.breaks_charge,
            pull: b.pull,
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

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, reduced by its
/// defence, reported, and credited to the shooter's aggro. Death is reported too.
fn damage(s: &mut State, b: Shot, target: Who, events: &mut Vec<DomainEvent>) {
    let vulnerable = status_pct(s.statuses(target), crate::engine::StatusKind::Vulnerable);
    let dealt = scale_damage(b.damage, 0, vulnerable, s.defence(target));
    let id = s.id_of(target);
    let hp = s.hp_mut(target);
    *hp = hp.saturating_sub(dealt);
    let dead = *hp == 0;
    events.push(DomainEvent::Hit {
        source: b.owner,
        target: id,
        damage: dealt,
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
    if let Some(i) = grid.first_hit(b.at, PLAYER_BULLET_RADIUS, |i| s.enemies[i].hp > 0) {
        damage(s, b, Who::Enemy(i), events);
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
                damage(s, b, Who::BossPart(i), events);
            }
            return true;
        }
    }
    if overlaps(b.at, PLAYER_BULLET_RADIUS, state.at, def.radius.0) {
        if !absorbed {
            damage(s, b, Who::Boss, events);
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
    let dealt = scale_damage(b.damage, 0, vulnerable, s.player.defence);
    let p = &mut s.player;
    p.hp = p.hp.saturating_sub(dealt);
    p.invulnerable = HIT_INVULNERABLE_TICKS;
    events.push(DomainEvent::PlayerHit { hp_left: p.hp });
    // A powerful attack breaks a charge (core#83).
    if b.breaks_charge {
        player::break_charge(s, events);
    }
}

/// An enemy's shot against the main character (unless it is invulnerable), then the
/// companions in ID order.
fn hit_ally_side(stage: &Stage, s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) -> bool {
    let p = &s.player;
    if p.invulnerable == 0 && p.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, p.at, PLAYER_RADIUS) {
        hurt_player(s, b, events);
        return true;
    }
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let radius = agent_def(stage, c.key).radius.0;
        if c.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, c.at, radius) {
            damage(s, b, Who::Companion(i), events);
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
    h.finish()
}
