//! The rules of `sim_version` 0 (ADR-035): a development version that changes in place until
//! the freeze at P3, with the hash check across CPUs never relaxed (ADR-019).
//!
//! What a tick does, in this order:
//! 1. the tick counter advances; waves spawn enemies;
//! 2. the main character moves (speed cap, focus, playfield) and fires on `fire` ticks only
//!    (ADR-038, no auto-fire);
//! 3. enemies move and act: the first rule that can fire, fires (ADR-036);
//! 4. bullets move and leave the field;
//! 5. collisions: main shots hurt enemies and the boss; enemy bullets hurt the main character;
//! 6. the outcome is decided: death or running out of ticks fails, an empty stage clears.
//!
//! Not yet in this version, each arriving with its own issue: companions, the main
//! character's skills (the input's `held` slot is ignored) and boss phases, parts and
//! timelines (core#16, core#17); attack contents (every `attack` fires one aimed bullet, until
//! core#18); and the `orbit` and `follow` movements (those enemies stand still). Rule
//! conditions, selectors and actions beyond the ones below never fire.
//!
//! Dynamic budgets (ADR-020): at most `MAX_ENEMIES_ALIVE` enemies, `MAX_BULLETS_ALIVE` bullets
//! and `MAX_BULLET_SPAWNS_PER_TICK` new bullets per tick. A spawn past a cap is dropped, in
//! spawn order, and a wave spawn that is dropped still counts as spawned.

use crate::behaviour::{Action, Condition, Movement, Selector};
use crate::engine::{BossState, BulletState, EnemyState, Outcome, PlayerState, RuleState, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::hash::StateHasher;
use crate::id::EntityId;
use crate::input::Input;
use crate::rng::SplitMix64;
use crate::stage::Stage;
use crate::validate::limits::{FIELD_H, FIELD_MARGIN, FIELD_W};

pub(crate) const MAX_ENEMIES_ALIVE: u32 = 300;
pub(crate) const MAX_BULLETS_ALIVE: u32 = 4000;
pub(crate) const MAX_BULLET_SPAWNS_PER_TICK: u32 = 200;

/// The main character is always entity 0; the boss, if any, is 1.
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
const UNLIMITED: u32 = u32::MAX;

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
        }
    });
    State {
        tick: 0,
        rng: SplitMix64::new(u64::from(stage.seed)),
        outcome: Outcome::Running,
        input: Input::default(),
        player: PlayerState {
            at: Point {
                x: Fx(FIELD_W / 2),
                y: Fx(FIELD_H - PLAYER_START_Y_FROM_BOTTOM),
            },
            hp: stage.player.hp,
            invulnerable: 0,
            shot_cooldown: 0,
        },
        boss,
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
    let mut spawns_left = MAX_BULLET_SPAWNS_PER_TICK;

    spawn_waves(stage, s);
    move_player(s, input);
    player_fire(stage, s, input, &mut spawns_left);
    act_enemies(stage, s, input, events, &mut spawns_left);
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
        s.enemies.push(EnemyState {
            id,
            wave: u32::try_from(i).unwrap_or(u32::MAX),
            at: wave.spawn,
            hp: wave.enemy.hp,
            age: 0,
            rules: wave
                .enemy
                .rules
                .iter()
                .map(|r| RuleState {
                    cooldown_left: 0,
                    charges_left: r.charges.map_or(UNLIMITED, u32::from),
                    fired: false,
                })
                .collect(),
        });
    }
}

/// Limits a move to `cap` units, keeping its direction: a diagonal is no faster (ADR-038).
fn capped_move(dx: i16, dy: i16, cap: i32) -> (i32, i32) {
    let (dx, dy) = (i64::from(dx), i64::from(dy));
    let len = (dx * dx + dy * dy).unsigned_abs().isqrt();
    let cap = i64::from(cap);
    let len = i64::try_from(len).unwrap_or(i64::MAX);
    if len <= cap {
        return (dx as i32, dy as i32);
    }
    // `len > cap >= 0`, and truncation toward zero is the same on every CPU.
    ((dx * cap / len) as i32, (dy * cap / len) as i32)
}

fn move_player(s: &mut State, input: Input) {
    let cap = if input.focus {
        FOCUS_SPEED
    } else {
        PLAYER_SPEED
    };
    let (dx, dy) = capped_move(input.dx, input.dy, cap);
    let p = &mut s.player;
    p.at.x = p.at.x.saturating_add(Fx(dx)).clamp(Fx(0), Fx(FIELD_W));
    p.at.y = p.at.y.saturating_add(Fx(dy)).clamp(Fx(0), Fx(FIELD_H));
    p.invulnerable = p.invulnerable.saturating_sub(1);
}

fn spawn_bullet(s: &mut State, bullet: BulletState, spawns_left: &mut u32) {
    if *spawns_left == 0 || count(&s.bullets) >= MAX_BULLETS_ALIVE {
        return;
    }
    *spawns_left -= 1;
    s.bullets.push(bullet);
}

/// The main shot fires only on ticks whose input has `fire` (ADR-038).
fn player_fire(stage: &Stage, s: &mut State, input: Input, spawns_left: &mut u32) {
    s.player.shot_cooldown = s.player.shot_cooldown.saturating_sub(1);
    if !input.fire || s.player.shot_cooldown > 0 {
        return;
    }
    s.player.shot_cooldown = SHOT_INTERVAL;
    let bullet = BulletState {
        at: s.player.at,
        vx: Fx(0),
        vy: Fx(-PLAYER_BULLET_SPEED),
        friendly: true,
        damage: stage.player.atk,
    };
    spawn_bullet(s, bullet, spawns_left);
}

/// A bullet from `from` toward `to` at `speed`; straight down if they coincide.
fn aimed(from: Point, to: Point, speed: i32) -> (Fx, Fx) {
    let dx = i64::from(to.x.0) - i64::from(from.x.0);
    let dy = i64::from(to.y.0) - i64::from(from.y.0);
    let dist = (dx * dx + dy * dy).unsigned_abs().isqrt();
    let dist = i64::try_from(dist).unwrap_or(i64::MAX);
    if dist == 0 {
        return (Fx(0), Fx(speed));
    }
    let speed = i64::from(speed);
    (
        Fx((dx * speed / dist) as i32),
        Fx((dy * speed / dist) as i32),
    )
}

fn condition_holds(cond: &Condition, age: u32, input: Input) -> bool {
    match cond {
        Condition::Always => true,
        Condition::TimeAbove { ticks } => age > *ticks,
        Condition::PlayerFiring => input.fire,
        _ => false,
    }
}

fn act_enemies(
    stage: &Stage,
    s: &mut State,
    input: Input,
    events: &mut Vec<DomainEvent>,
    spawns_left: &mut u32,
) {
    let target = s.player.at;
    let mut shots = Vec::new();
    for e in &mut s.enemies {
        let agent = &stage.waves[e.wave as usize].enemy;
        if let Movement::Straight { vx, vy } = agent.movement {
            e.at.x = e.at.x.saturating_add(vx);
            e.at.y = e.at.y.saturating_add(vy);
        }
        e.age = e.age.saturating_add(1);
        for r in &mut e.rules {
            r.cooldown_left = r.cooldown_left.saturating_sub(1);
        }
        for (i, rule) in agent.rules.iter().enumerate() {
            let st = &mut e.rules[i];
            if st.cooldown_left > 0 || st.charges_left == 0 || (rule.once && st.fired) {
                continue;
            }
            if !condition_holds(&rule.when, e.age, input) || rule.target != Selector::Player {
                continue;
            }
            match &rule.action {
                Action::Attack { .. } => {
                    let (vx, vy) = aimed(e.at, target, ENEMY_BULLET_SPEED);
                    shots.push(BulletState {
                        at: e.at,
                        vx,
                        vy,
                        friendly: false,
                        damage: 1,
                    });
                }
                Action::Telegraph { ticks } => events.push(DomainEvent::Telegraph {
                    source: e.id,
                    ticks: *ticks,
                }),
                _ => continue,
            }
            st.cooldown_left = rule.cooldown_ticks;
            if st.charges_left != UNLIMITED {
                st.charges_left -= 1;
            }
            st.fired = true;
            events.push(DomainEvent::RuleFired {
                agent: e.id,
                rule: u8::try_from(i).unwrap_or(u8::MAX),
                target: Some(PLAYER_ID),
            });
            break;
        }
    }
    for shot in shots {
        spawn_bullet(s, shot, spawns_left);
    }
    s.enemies.retain(|e| inside_with_margin(e.at));
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
    let State {
        bullets,
        enemies,
        boss,
        player,
        ..
    } = s;
    bullets.retain(|b| {
        if b.friendly {
            for e in enemies.iter_mut().filter(|e| e.hp > 0) {
                let radius = stage.waves[e.wave as usize].enemy.radius.0;
                if overlaps(b.at, PLAYER_BULLET_RADIUS, e.at, radius) {
                    hurt(&mut e.hp, e.id, b.damage, events);
                    return false;
                }
            }
            if let (Some(state), Some(def)) = (boss.as_mut(), stage.boss.as_ref()) {
                if state.hp > 0 && overlaps(b.at, PLAYER_BULLET_RADIUS, state.at, def.radius.0) {
                    hurt(&mut state.hp, state.id, b.damage, events);
                    return false;
                }
            }
            true
        } else {
            if player.invulnerable == 0
                && player.hp > 0
                && overlaps(b.at, ENEMY_BULLET_RADIUS, player.at, PLAYER_RADIUS)
            {
                player.hp = player.hp.saturating_sub(b.damage);
                player.invulnerable = HIT_INVULNERABLE_TICKS;
                events.push(DomainEvent::PlayerHit { hp_left: player.hp });
                return false;
            }
            true
        }
    });
    enemies.retain(|e| e.hp > 0);
}

fn hurt(hp: &mut u32, target: EntityId, damage: u32, events: &mut Vec<DomainEvent>) {
    *hp = hp.saturating_sub(damage);
    events.push(DomainEvent::Hit {
        source: PLAYER_ID,
        target,
        damage,
    });
    if *hp == 0 {
        events.push(DomainEvent::Died { entity: target });
    }
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
    h.write_i32(s.player.at.x.0);
    h.write_i32(s.player.at.y.0);
    h.write_u32(s.player.hp);
    h.write_u32(s.player.invulnerable);
    h.write_u32(s.player.shot_cooldown);
    match &s.boss {
        Some(b) => {
            h.write_bool(true);
            h.write_u32(b.id.0);
            h.write_i32(b.at.x.0);
            h.write_i32(b.at.y.0);
            h.write_u32(b.hp);
        }
        None => h.write_bool(false),
    }
    h.write_u32(s.next_id);
    h.write_u32(count(&s.spawned));
    for n in &s.spawned {
        h.write_u32(u32::from(*n));
    }
    h.write_u32(count(&s.enemies));
    for e in &s.enemies {
        h.write_u32(e.id.0);
        h.write_u32(e.wave);
        h.write_i32(e.at.x.0);
        h.write_i32(e.at.y.0);
        h.write_u32(e.hp);
        h.write_u32(e.age);
        h.write_u32(count(&e.rules));
        for r in &e.rules {
            h.write_u32(r.cooldown_left);
            h.write_u32(r.charges_left);
            h.write_bool(r.fired);
        }
    }
    h.write_u32(count(&s.bullets));
    for b in &s.bullets {
        h.write_i32(b.at.x.0);
        h.write_i32(b.at.y.0);
        h.write_i32(b.vx.0);
        h.write_i32(b.vy.0);
        h.write_bool(b.friendly);
        h.write_u32(b.damage);
    }
    h.finish()
}
