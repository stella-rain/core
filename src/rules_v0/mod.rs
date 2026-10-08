//! The rules of `sim_version` 0 (ADR-035): a development version that changes in place until
//! the freeze at P3, with the hash check across CPUs never relaxed (ADR-019).
//!
//! What a tick does, in this order:
//! 1. the tick counter advances; waves spawn enemies; statuses run out and aggro fades;
//! 2. the main character moves (speed cap, focus, slowing, playfield), fires on `fire` ticks
//!    only (ADR-038, no auto-fire), and its skills are held and cast (ADR-032);
//! 3. companions, then enemies, in entity-ID order: move, then the first rule that can fire,
//!    fires (ADR-036);
//! 4. bullets move and leave the field;
//! 5. collisions: shots of allies hurt enemies and the boss, shots of enemies hurt the main
//!    character and the companions;
//! 6. the outcome is decided: death or running out of ticks fails, an empty stage clears.
//!
//! Not yet in this version: boss phases, parts and timelines (core#17); attack contents
//! (every `attack` fires one aimed bullet, until core#18); summons by companions. Rule
//! conditions, selectors and actions beyond the ones in `agents` and `select` never fire.
//!
//! Dynamic budgets (ADR-020): at most `MAX_ENEMIES_ALIVE` enemies, `MAX_BULLETS_ALIVE` bullets
//! and `MAX_BULLET_SPAWNS_PER_TICK` new bullets per tick. A spawn past a cap is dropped, in
//! spawn order, and a wave spawn that is dropped still counts as spawned.

mod act;
mod agents;
mod entity;
mod player;
mod select;

use crate::engine::{
    AgentKey, AgentSource, BossState, BulletState, HoldState, Outcome, PlayerState, SkillState,
    State, UNLIMITED,
};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::hash::StateHasher;
use crate::id::EntityId;
use crate::input::Input;
use crate::rng::SplitMix64;
use crate::stage::Stage;
use crate::trig;
use crate::validate::limits::{FIELD_H, FIELD_MARGIN, FIELD_W};

pub(crate) use agents::agent_def;
use entity::{Who, scale_damage, status_pct};

pub(crate) const MAX_ENEMIES_ALIVE: u32 = 300;
pub(crate) const MAX_BULLETS_ALIVE: u32 = 4000;
pub(crate) const MAX_BULLET_SPAWNS_PER_TICK: u32 = 200;

/// The main character is always entity 0; the boss, if any, is 1, then the companions.
const PLAYER_ID: EntityId = EntityId(0);
const PLAYER_SPEED: i32 = Fx::px(3).0;
const FOCUS_SPEED: i32 = Fx::px(1).0;
/// The hurt circle, a tiny one (ADR-038).
const PLAYER_RADIUS: i32 = Fx::px(2).0;
const PLAYER_START_Y_FROM_BOTTOM: i32 = Fx::px(64).0;
/// Ticks between main shots while `fire` is held.
const SHOT_INTERVAL: u32 = 6;
const PLAYER_BULLET_SPEED: i32 = Fx::px(8).0;
const PLAYER_BULLET_RADIUS: i32 = Fx::px(2).0;
const ENEMY_BULLET_SPEED: i32 = Fx::px(3).0;
const ENEMY_BULLET_RADIUS: i32 = Fx::px(3).0;
/// Ticks of invulnerability after the main character is hit.
const HIT_INVULNERABLE_TICKS: u32 = 60;
/// A held skill casts by itself after this many ticks (ADR-032).
const HOLD_LIMIT_TICKS: u32 = 30;

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

pub(crate) fn initial_state(stage: &Stage) -> State {
    let mut next_id = 1;
    let boss = stage.boss.as_ref().map(|b| {
        let id = EntityId(next_id);
        next_id += 1;
        BossState {
            id,
            at: b.spawn,
            hp: b.hp,
            max_hp: b.hp,
            statuses: Vec::new(),
        }
    });
    let start = Point {
        x: Fx(FIELD_W / 2),
        y: Fx(FIELD_H - PLAYER_START_Y_FROM_BOTTOM),
    };
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
    State {
        tick: 0,
        rng: SplitMix64::new(u64::from(stage.seed)),
        outcome: Outcome::Running,
        input: Input::default(),
        player: PlayerState {
            at: start,
            hp: stage.player.hp,
            max_hp: stage.player.hp,
            invulnerable: 0,
            shot_cooldown: 0,
            firing_streak: 0,
            aggro: 0,
            statuses: Vec::new(),
            skills: stage
                .player
                .skills
                .iter()
                .map(|sk| SkillState {
                    cooldown_left: 0,
                    charges_left: sk.charges.map_or(UNLIMITED, u32::from),
                })
                .collect(),
            hold: None,
            held_before: 0,
        },
        boss,
        companions,
        enemies: Vec::new(),
        bullets: Vec::new(),
        spawned: vec![0; stage.waves.len()],
        next_id,
    }
}

pub(crate) fn step(stage: &Stage, s: &mut State, input: Input, events: &mut Vec<DomainEvent>) {
    if s.outcome != Outcome::Running {
        return;
    }
    s.tick += 1;
    s.input = input;
    s.player.firing_streak = if input.fire {
        s.player.firing_streak.saturating_add(1)
    } else {
        0
    };
    let mut spawns_left = MAX_BULLET_SPAWNS_PER_TICK;

    spawn_waves(stage, s);
    entity::tick_statuses_and_aggro(s);
    player::update(stage, s, input, events, &mut spawns_left);
    agents::act(stage, s, events, &mut spawns_left);
    move_bullets(s);
    collide(stage, s, events);
    decide_outcome(stage, s, events);
}

fn count<T>(items: &[T]) -> u32 {
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

fn inside_with_margin(p: Point) -> bool {
    let range = |v: i32, max: i32| (-FIELD_MARGIN..=max + FIELD_MARGIN).contains(&v);
    range(p.x.0, FIELD_W) && range(p.y.0, FIELD_H)
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
        if count(&s.enemies) >= MAX_ENEMIES_ALIVE {
            continue;
        }
        let id = EntityId(s.next_id);
        s.next_id += 1;
        let key = AgentKey {
            source: AgentSource::Wave,
            index: u32::try_from(i).unwrap_or(u32::MAX),
            rule: 0,
        };
        s.enemies
            .push(agents::new_agent(id, key, &wave.enemy, wave.spawn, 0));
    }
}

fn spawn_bullet(s: &mut State, bullet: BulletState, spawns_left: &mut u32) {
    if *spawns_left == 0 || count(&s.bullets) >= MAX_BULLETS_ALIVE {
        return;
    }
    *spawns_left -= 1;
    s.bullets.push(bullet);
}

/// A bullet velocity from `from` toward `to` at `speed`; straight down if they coincide.
fn aimed(from: Point, to: Point, speed: i32) -> (Fx, Fx) {
    let dx = i64::from(to.x.0) - i64::from(from.x.0);
    let dy = i64::from(to.y.0) - i64::from(from.y.0);
    let dist = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    if dist == 0 {
        return (Fx(0), Fx(speed));
    }
    let speed = i64::from(speed);
    (
        Fx((dx * speed / dist) as i32),
        Fx((dy * speed / dist) as i32),
    )
}

fn move_bullets(s: &mut State) {
    for b in &mut s.bullets {
        b.at.x = b.at.x.saturating_add(b.vx);
        b.at.y = b.at.y.saturating_add(b.vy);
    }
    s.bullets.retain(|b| inside_with_margin(b.at));
}

/// Two circles overlap (touching counts), in 64-bit fixed point.
fn overlaps(a: Point, ra: i32, b: Point, rb: i32) -> bool {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    let r = i64::from(ra) + i64::from(rb);
    dx * dx + dy * dy <= r * r
}

fn collide(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let mut consumed = Vec::with_capacity(s.bullets.len());
    for i in 0..s.bullets.len() {
        let b = s.bullets[i];
        consumed.push(if b.friendly {
            hit_enemy_side(stage, s, b, events)
        } else {
            hit_ally_side(stage, s, b, events)
        });
    }
    let mut flags = consumed.into_iter();
    s.bullets.retain(|_| !flags.next().unwrap_or(false));
    s.companions.retain(|c| c.hp > 0);
    s.enemies.retain(|e| e.hp > 0);
}

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, reported, and
/// credited to the shooter's aggro. Death is reported too.
fn damage(s: &mut State, b: BulletState, target: Who, events: &mut Vec<DomainEvent>) {
    let vulnerable = status_pct(s.statuses(target), crate::engine::StatusKind::Vulnerable);
    let dealt = scale_damage(b.damage, 0, vulnerable);
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
        events.push(DomainEvent::Died { entity: id });
    }
    s.raise_aggro(b.owner, dealt.saturating_mul(AGGRO_PER_DAMAGE));
}

/// An ally's shot against the enemies in ID order, then the boss. Returns whether it was used up.
fn hit_enemy_side(
    stage: &Stage,
    s: &mut State,
    b: BulletState,
    events: &mut Vec<DomainEvent>,
) -> bool {
    for i in 0..s.enemies.len() {
        let e = &s.enemies[i];
        let radius = agent_def(stage, e.key).radius.0;
        if e.hp > 0 && overlaps(b.at, PLAYER_BULLET_RADIUS, e.at, radius) {
            damage(s, b, Who::Enemy(i), events);
            return true;
        }
    }
    if let (Some(state), Some(def)) = (&s.boss, &stage.boss) {
        if state.hp > 0 && overlaps(b.at, PLAYER_BULLET_RADIUS, state.at, def.radius.0) {
            damage(s, b, Who::Boss, events);
            return true;
        }
    }
    false
}

/// An enemy's shot against the main character (unless it is invulnerable), then the
/// companions in ID order.
fn hit_ally_side(
    stage: &Stage,
    s: &mut State,
    b: BulletState,
    events: &mut Vec<DomainEvent>,
) -> bool {
    let p = &s.player;
    if p.invulnerable == 0 && p.hp > 0 && overlaps(b.at, ENEMY_BULLET_RADIUS, p.at, PLAYER_RADIUS) {
        let vulnerable = status_pct(&p.statuses, crate::engine::StatusKind::Vulnerable);
        let dealt = scale_damage(b.damage, 0, vulnerable);
        let p = &mut s.player;
        p.hp = p.hp.saturating_sub(dealt);
        p.invulnerable = HIT_INVULNERABLE_TICKS;
        events.push(DomainEvent::PlayerHit { hp_left: p.hp });
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
    h.write_u32(a.age);
    h.write_u32(a.aggro);
    h.write_i32(a.orbit_angle);
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
    h.write_u32(p.invulnerable);
    h.write_u32(p.shot_cooldown);
    h.write_u32(p.firing_streak);
    h.write_u32(p.aggro);
    h.write_u32(u32::from(p.held_before));
    hash_statuses(&mut h, &p.statuses);
    h.write_u32(count(&p.skills));
    for sk in &p.skills {
        h.write_u32(sk.cooldown_left);
        h.write_u32(sk.charges_left);
    }
    match p.hold {
        Some(HoldState { index, ticks }) => {
            h.write_bool(true);
            h.write_u32(index);
            h.write_u32(ticks);
        }
        None => h.write_bool(false),
    }
    match &s.boss {
        Some(b) => {
            h.write_bool(true);
            h.write_u32(b.id.0);
            h.write_i32(b.at.x.0);
            h.write_i32(b.at.y.0);
            h.write_u32(b.hp);
            h.write_u32(b.max_hp);
            hash_statuses(&mut h, &b.statuses);
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
    h.write_u32(count(&s.bullets));
    for b in &s.bullets {
        h.write_i32(b.at.x.0);
        h.write_i32(b.at.y.0);
        h.write_i32(b.vx.0);
        h.write_i32(b.vy.0);
        h.write_u32(b.owner.0);
        h.write_bool(b.friendly);
        h.write_u32(b.damage);
    }
    h.finish()
}
