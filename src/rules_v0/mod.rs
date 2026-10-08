//! The rules of `sim_version` 0 (ADR-035): a development version that changes in place until
//! the freeze at P3, with the hash check across CPUs never relaxed (ADR-019).
//!
//! What a tick does, in this order:
//! 1. the tick counter advances; waves spawn enemies; statuses run out and aggro fades;
//! 2. the main character moves (speed cap, focus, slowing, playfield), starts its shot attack
//!    on `fire` ticks only (ADR-038, no auto-fire; see `player::main_shot`), and its skills are
//!    held and cast (ADR-032);
//! 3. the boss (phases, timeline, rules: `boss`), then companions, then enemies, in entity-ID
//!    order: move, then the first rule that can fire, fires (ADR-036), which for an attack or
//!    a shot skill starts an attack task;
//! 4. bullets whose `on_bullet` wait is over start their attacks, then every running attack
//!    is carried out up to its next wait (`attacks`), making bullets;
//! 5. bullets move and leave the field;
//! 6. collisions: shots of allies hurt enemies and the boss, shots of enemies hurt the main
//!    character and the companions;
//! 7. the outcome is decided: death or running out of ticks fails, an empty stage clears.
//!
//! Not yet in this version: summons by companions; ways of shooting the main character acquires
//! during a run (`player::main_shot` is where they would plug in).
//! Rule conditions, selectors and actions beyond the ones in `agents` and `select` never fire.
//!
//! Dynamic budgets (ADR-020): at most `MAX_ENEMIES_ALIVE` enemies, `MAX_BULLETS_ALIVE` bullets,
//! `MAX_BULLET_SPAWNS_PER_TICK` new bullets per tick, `MAX_ATTACK_TASKS` attacks at once and
//! `MAX_EMITTER_STEPS_PER_TICK` steps of them per tick. A spawn past a cap is dropped, in
//! spawn order, and a wave spawn that is dropped still counts as spawned; attacks past the
//! step budget wait for the next tick.

mod act;
mod agents;
mod attacks;
mod boss;
mod entity;
mod player;
mod select;

use crate::engine::{
    AgentKey, AgentSource, BossState, BulletState, HoldState, Outcome, PartState, PlayerState,
    SkillState, State, UNLIMITED,
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
/// Attacks being carried out at once (ADR-036 tier 3), and the node steps they may take in a
/// tick between them. An attack that would take more waits for the next tick.
pub(crate) const MAX_ATTACK_TASKS: u32 = 256;
pub(crate) const MAX_EMITTER_STEPS_PER_TICK: u32 = 4096;

/// The main character is always entity 0; the boss, if any, is 1, then the companions.
const PLAYER_ID: EntityId = EntityId(0);
const PLAYER_SPEED: i32 = Fx::px(3).0;
const FOCUS_SPEED: i32 = Fx::px(1).0;
/// The hurt circle, a tiny one (ADR-038).
const PLAYER_RADIUS: i32 = Fx::px(2).0;
const PLAYER_START_Y_FROM_BOTTOM: i32 = Fx::px(64).0;
/// Ticks between main shots while `fire` is held.
const SHOT_INTERVAL: u32 = 6;
const PLAYER_BULLET_RADIUS: i32 = Fx::px(2).0;
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
    let boss_id = stage.boss.as_ref().map(|_| {
        let id = EntityId(next_id);
        next_id += 1;
        id
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
    // The boss's parts come after the companions in the order of entity IDs.
    let boss = stage.boss.as_ref().zip(boss_id).map(|(b, id)| BossState {
        id,
        at: b.spawn,
        hp: b.hp,
        max_hp: b.hp,
        statuses: Vec::new(),
        phase: 0,
        phase_age: 0,
        invulnerable: 0,
        step: 0,
        wait_left: 0,
        order: None,
        // The first phase begins on the first tick (see `boss::act`).
        rules: Vec::new(),
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
                    offset: p.offset,
                    radius: p.radius.0,
                    statuses: Vec::new(),
                }
            })
            .collect(),
    });
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
        tasks: Vec::new(),
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
    player::update(stage, s, input, events);
    boss::act(stage, s, events);
    agents::act(stage, s, events);
    attacks::fire_hooks(s);
    attacks::run(s, &mut spawns_left);
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

/// What a hit needs to know about the bullet that made it.
#[derive(Clone, Copy)]
struct Shot {
    at: Point,
    owner: EntityId,
    friendly: bool,
    damage: u32,
}

fn collide(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let mut consumed = Vec::with_capacity(s.bullets.len());
    for i in 0..s.bullets.len() {
        let b = &s.bullets[i];
        let shot = Shot {
            at: b.at,
            owner: b.owner,
            friendly: b.friendly,
            damage: b.damage,
        };
        consumed.push(if shot.friendly {
            hit_enemy_side(stage, s, shot, events)
        } else {
            hit_ally_side(stage, s, shot, events)
        });
    }
    let mut flags = consumed.into_iter();
    s.bullets.retain(|_| !flags.next().unwrap_or(false));
    s.companions.retain(|c| c.hp > 0);
    s.enemies.retain(|e| e.hp > 0);
}

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, reported, and
/// credited to the shooter's aggro. Death is reported too.
fn damage(s: &mut State, b: Shot, target: Who, events: &mut Vec<DomainEvent>) {
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
fn hit_enemy_side(stage: &Stage, s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) -> bool {
    for i in 0..s.enemies.len() {
        let e = &s.enemies[i];
        let radius = agent_def(stage, e.key).radius.0;
        if e.hp > 0 && overlaps(b.at, PLAYER_BULLET_RADIUS, e.at, radius) {
            damage(s, b, Who::Enemy(i), events);
            return true;
        }
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

/// An enemy's shot against the main character (unless it is invulnerable), then the
/// companions in ID order.
fn hit_ally_side(stage: &Stage, s: &mut State, b: Shot, events: &mut Vec<DomainEvent>) -> bool {
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
            h.write_bool(false);
            h.write_u32(id.0);
        }
        crate::engine::Aim::Fixed(d) => {
            h.write_bool(true);
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
                hash_statuses(&mut h, &part.statuses);
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
        match &b.hook {
            Some(hook) => {
                h.write_bool(true);
                hash_path(&mut h, &hook.path);
                h.write_u32(hook.left);
                h.write_u32(hook.owner.0);
                hash_aim(&mut h, hook.aim);
                h.write_bool(hook.friendly);
                h.write_u32(hook.damage);
            }
            None => h.write_bool(false),
        }
    }
    h.finish()
}
