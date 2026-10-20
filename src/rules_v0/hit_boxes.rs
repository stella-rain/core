//! Hit boxes (ADR-055): every active phase with a hit box hurts the targets inside it, in the
//! collision step, before bullets.
//!
//! - A hit box follows its attacker, or stays on the point an on-hit rune's attack fired at
//!   (ADR-099), and faces the way the attack faced when it started. Its
//!   offset is forward along that facing and to the attacker's right as seen from above: a
//!   quarter turn clockwise on the screen, so facing down the screen its right is the screen's
//!   left.
//! - A circle hits a target whose circle overlaps it; an arc, a target whose circle overlaps
//!   the sector `half_angle` to each side of the facing. Integer only: squared distances in 64
//!   or 128 bits, and the arc's edges and angle from the sine table.
//! - Friendly hit boxes hit the boss, its parts and the enemies (found through the enemy grid),
//!   in entity-ID order; enemy hit boxes hit the main character, then the companions. A hit box
//!   hits every target inside it (cleave), each once per active phase, or again `rehit_ticks`
//!   after its last hit. A homing missile's (ADR-099) hits only the first, once, and the
//!   missile is gone once every hit box has landed. A target that cannot be hurt (invulnerable, dead) is not hit and not
//!   counted as hit.
//! - Attackers resolve in entity-ID order, and every hit box active at the start of the step
//!   lands, even if its attacker dies earlier in the step: hits on the same tick trade.

use crate::attack::{DamageType, HitBox, PhaseKind};
use crate::engine::{Dir, Smash, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::rune::Rune;
use crate::stage::Stage;
use crate::trig;

use super::entity::Who;
use super::grid::Grid;
use super::{PLAYER_RADIUS, Shot, agent_def, damage, enemy_grid, hurt_player, overlaps};

/// One hit box that is active this tick.
struct Swing {
    run: usize,
    owner: EntityId,
    slot: u8,
    friendly: bool,
    centre: Point,
    facing: Dir,
    radius: i32,
    /// `None` for a circle, or an arc of 180 degrees or more to each side, which is one.
    arc: Option<Sector>,
    damage: u32,
    damage_type: Option<DamageType>,
    rehit_ticks: Option<u32>,
    breaks_charge: bool,
    smash: Option<Smash>,
    combo: bool,
    rune: Option<Rune>,
    /// A homing missile's: one hit, then nothing.
    once: bool,
}

/// What an arc's test needs, worked out once per swing rather than once per target: the cosine
/// of its half angle and its two edges.
struct Sector {
    cos: i32,
    edges: [Dir; 2],
}

impl Sector {
    fn new(facing: Dir, half_angle: i32) -> Option<Sector> {
        (half_angle < 180 * trig::PER_DEGREE).then(|| Sector {
            cos: trig::sin_cos(half_angle).1,
            edges: [half_angle, -half_angle].map(|turn| super::attacks::rotate(facing, turn)),
        })
    }
}

/// Every active hit box hurts what it covers. `grid` is the enemy grid, built here if a
/// friendly hit box needs it and left for the bullets after.
// Kept out of `rules_v0::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-058).
#[inline(never)]
pub(super) fn resolve(
    stage: &Stage,
    s: &mut State,
    grid: &mut Option<Grid>,
    events: &mut Vec<DomainEvent>,
) {
    let mut swings = active_swings(s);
    if swings.is_empty() {
        return;
    }
    swings.sort_by_key(|w| (w.owner.0, w.slot));
    let mut found = Vec::new();
    for w in &swings {
        if w.friendly {
            let grid = grid.get_or_insert_with(|| enemy_grid(stage, s));
            hit_enemy_side(stage, s, grid, w, &mut found, events);
        } else {
            hit_ally_side(stage, s, w, events);
        }
    }
    if swings.iter().any(|w| w.once) {
        s.runs.retain(|r| r.homing.is_none() || r.hits.is_empty());
    }
}

fn active_swings(s: &State) -> Vec<Swing> {
    let mut swings = Vec::new();
    for (i, r) in s.runs.iter().enumerate() {
        let p = &r.phases[r.phase as usize];
        let (PhaseKind::Active, Some(h)) = (p.kind, &p.hit_box) else {
            continue;
        };
        let Some(owner) = s.find(r.owner) else {
            continue;
        };
        let (HitBox::Circle { offset, radius } | HitBox::Arc { offset, radius, .. }) = h;
        let arc = match h {
            HitBox::Circle { .. } => None,
            HitBox::Arc { half_angle, .. } => Sector::new(r.facing, *half_angle),
        };
        swings.push(Swing {
            run: i,
            owner: r.owner,
            slot: r.slot,
            friendly: r.friendly,
            centre: centre(
                r.at.unwrap_or_else(|| s.at(owner)),
                r.facing,
                offset.forward.0,
                offset.right.0,
            ),
            facing: r.facing,
            radius: radius.0,
            arc,
            damage: super::attacks::phase_damage(r),
            damage_type: p.damage_type,
            rehit_ticks: p.rehit_ticks,
            breaks_charge: p.breaks_charge,
            smash: r.smash,
            combo: super::attacks::is_combo(r),
            rune: super::runes::of_run(r),
            once: r.homing.is_some(),
        });
    }
    swings
}

/// The hit box's centre: `forward` along `facing` and `right` a quarter turn clockwise from it.
fn centre(at: Point, facing: Dir, forward: i32, right: i32) -> Point {
    let (fx, fy) = (i64::from(facing.x), i64::from(facing.y));
    let (fw, rt) = (i64::from(forward), i64::from(right));
    let scale = i64::from(trig::SCALE);
    let dx = (fx * fw - fy * rt) / scale;
    let dy = (fy * fw + fx * rt) / scale;
    let move_by = |v: Fx, d: i64| Fx(i32::try_from(i64::from(v.0) + d).unwrap_or(v.0));
    Point {
        x: move_by(at.x, dx),
        y: move_by(at.y, dy),
    }
}

/// Whether the swing's hit box overlaps the circle at `at` with `radius`.
fn covers(w: &Swing, at: Point, radius: i32) -> bool {
    if !overlaps(w.centre, w.radius, at, radius) {
        return false;
    }
    let Some(arc) = &w.arc else {
        return true;
    };
    let d = (
        i128::from(at.x.0) - i128::from(w.centre.x.0),
        i128::from(at.y.0) - i128::from(w.centre.y.0),
    );
    // A centre within the angle and within reach overlaps; one outside the angle overlaps only
    // if its circle crosses one of the two edges, the nearest part of the sector to it.
    within_angle(d, w.facing, arc.cos)
        || arc
            .edges
            .iter()
            .any(|&edge| touches_edge(d, edge, w.radius, radius))
}

/// Whether `d` is at most the half angle whose cosine is `cos` from `facing`:
/// `d · f ≥ |d| |f| cos`, squared so no root is taken. At the centre itself, it is.
fn within_angle(d: (i128, i128), facing: Dir, cos: i32) -> bool {
    let (fx, fy) = (i128::from(facing.x), i128::from(facing.y));
    let dot = d.0 * fx + d.1 * fy;
    let d2 = d.0 * d.0 + d.1 * d.1;
    if d2 == 0 {
        return true;
    }
    let cos = i128::from(cos);
    let scale = i128::from(trig::SCALE);
    let lhs = dot * scale * (dot * scale);
    let rhs = cos * cos * d2 * (fx * fx + fy * fy);
    if cos >= 0 {
        dot >= 0 && lhs >= rhs
    } else {
        dot >= 0 || lhs <= rhs
    }
}

/// Whether the circle at `d` (from the arc's centre) with `radius` reaches the edge running
/// `length` along `edge`, a direction of length `trig::SCALE`.
fn touches_edge(d: (i128, i128), edge: Dir, length: i32, radius: i32) -> bool {
    let (ex, ey) = (i128::from(edge.x), i128::from(edge.y));
    let scale = i128::from(trig::SCALE);
    let r2 = i128::from(radius) * i128::from(radius);
    let along = d.0 * ex + d.1 * ey;
    if along <= 0 {
        return d.0 * d.0 + d.1 * d.1 <= r2;
    }
    let len = i128::from(length);
    if along >= len * scale {
        let (px, py) = (d.0 - len * ex / scale, d.1 - len * ey / scale);
        return px * px + py * py <= r2;
    }
    let across = d.0 * ey - d.1 * ex;
    across * across <= r2 * scale * scale
}

/// Whether the swing may hit `target` now, and if so, counts the hit.
fn take_hit(s: &mut State, w: &Swing, target: EntityId) -> bool {
    let tick = s.tick;
    let hits = &mut s.runs[w.run].hits;
    if w.once && !hits.is_empty() {
        return false;
    }
    match hits.binary_search_by_key(&target.0, |h| h.0.0) {
        Ok(i) => {
            let again = w
                .rehit_ticks
                .is_some_and(|k| tick >= hits[i].1.saturating_add(k));
            if again {
                hits[i].1 = tick;
            }
            again
        }
        Err(i) => {
            hits.insert(i, (target, tick));
            true
        }
    }
}

fn shot(w: &Swing) -> Shot {
    Shot {
        at: w.centre,
        owner: w.owner,
        friendly: w.friendly,
        damage: w.damage,
        damage_type: w.damage_type,
        breaks_charge: w.breaks_charge,
        smash: w.smash,
        combo: w.combo,
        rune: w.rune,
    }
}

/// The boss, its parts and the enemies, in entity-ID order. An invulnerable boss shields its
/// parts, as it does from bullets.
fn hit_enemy_side(
    stage: &Stage,
    s: &mut State,
    grid: &Grid,
    w: &Swing,
    found: &mut Vec<usize>,
    events: &mut Vec<DomainEvent>,
) {
    if let Some((def, boss)) = stage.boss.as_ref().zip(s.boss.as_ref()) {
        if boss.invulnerable == 0 {
            let (id, at, radius) = (boss.id, boss.at, def.radius.0);
            if boss.hp > 0 && covers(w, at, radius) && take_hit(s, w, id) {
                damage(stage, s, shot(w), Who::Boss, events);
            }
            let parts = s.boss.as_ref().map_or(0, |b| b.parts.len());
            for i in 0..parts {
                let Some(part) = s.boss.as_ref().map(|b| &b.parts[i]) else {
                    break;
                };
                let (id, hp, radius) = (part.id, part.hp, part.radius);
                if hp > 0 && covers(w, s.at(Who::BossPart(i)), radius) && take_hit(s, w, id) {
                    damage(stage, s, shot(w), Who::BossPart(i), events);
                }
            }
        }
    }
    grid.within(w.centre, w.radius, found);
    for &i in found.iter() {
        let e = &s.enemies[i];
        let (id, at, hp) = (e.id, e.at, e.hp);
        let radius = agent_def(stage, e.key).radius.0;
        // A shrine is not hurt, nor a mimic in disguise: the hit passes through them.
        if hp > 0
            && e.shrine.is_none()
            && !e.hidden()
            && covers(w, at, radius)
            && take_hit(s, w, id)
        {
            damage(stage, s, shot(w), Who::Enemy(i), events);
            match w.smash {
                Some(Smash::Pull(pull)) => super::pull_enemy(stage, s, i, pull),
                Some(Smash::Knockback(push)) => super::knock_enemy(stage, s, i, push),
                _ => {}
            }
        }
    }
}

/// The main character (unless it is invulnerable), then the companions in ID order.
fn hit_ally_side(stage: &Stage, s: &mut State, w: &Swing, events: &mut Vec<DomainEvent>) {
    let p = &s.player;
    if p.hp > 0
        && p.invulnerable == 0
        && covers(w, p.at, PLAYER_RADIUS)
        && take_hit(s, w, super::PLAYER_ID)
    {
        hurt_player(s, super::aided(stage, s, shot(w)), events);
    }
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let (id, at, hp) = (c.id, c.at, c.hp);
        let radius = agent_def(stage, c.key).radius.0;
        if hp > 0 && covers(w, at, radius) && take_hit(s, w, id) {
            damage(
                stage,
                s,
                super::aided(stage, s, shot(w)),
                Who::Companion(i),
                events,
            );
        }
    }
}
