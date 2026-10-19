//! Selectors (ADR-036): who a rule, a condition or a skill acts on. Ties break by squared
//! distance from the acting entity, then entity ID, so the choice never depends on storage
//! order.
//!
//! Allies are the main character and the companions, whichever side the acting entity is on:
//! an enemy's `nearest_ally` is the nearest of them. Enemies are the enemies and the boss.

use std::cmp::Reverse;

use crate::behaviour::Selector;
use crate::engine::State;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::stage::Stage;

use super::agent_def;
use super::entity::Who;
use super::overlaps;

fn dist2(a: Point, b: Point) -> u64 {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    (dx * dx + dy * dy).unsigned_abs()
}

fn allies(s: &State) -> impl Iterator<Item = Who> + '_ {
    std::iter::once(Who::Player)
        .chain((0..s.companions.len()).map(Who::Companion))
        .filter(|w| s.is_alive(*w))
}

fn enemies(s: &State) -> impl Iterator<Item = Who> + '_ {
    s.boss
        .iter()
        .map(|_| Who::Boss)
        .chain((0..s.enemies.len()).map(Who::Enemy))
        .filter(|w| s.is_alive(*w))
}

/// The entity `selector` picks for `me`, if there is one.
pub(super) fn select(stage: &Stage, s: &State, me: Who, selector: &Selector) -> Option<Who> {
    let from = s.at(me);
    let by_distance = |w: &Who| (dist2(from, s.at(*w)), s.id_of(*w));
    match selector {
        Selector::Myself => Some(me),
        Selector::Player => s.is_alive(Who::Player).then_some(Who::Player),
        Selector::NearestEnemy => enemies(s).min_by_key(by_distance),
        Selector::BossFirst => match s.boss {
            Some(_) if s.is_alive(Who::Boss) => Some(Who::Boss),
            _ => enemies(s).min_by_key(by_distance),
        },
        // Never itself: its own distance is 0, which would always win.
        Selector::NearestAlly => allies(s).filter(|w| *w != me).min_by_key(by_distance),
        Selector::LowestHpAlly => {
            allies(s).min_by_key(|w| (s.hp(*w), dist2(from, s.at(*w)), s.id_of(*w)))
        }
        Selector::HighestAggroAlly => {
            allies(s).min_by_key(|w| (Reverse(s.aggro(*w)), dist2(from, s.at(*w)), s.id_of(*w)))
        }
        // A part of the boss that has not broken yet.
        Selector::Part(id) => {
            let index = stage
                .boss
                .iter()
                .flat_map(|b| &b.parts)
                .position(|p| p.id == *id)?;
            let part = s.boss.as_ref()?.parts.get(index)?;
            (part.hp > 0).then_some(Who::BossPart(index))
        }
    }
}

/// Aim assist's target (ADR-072): among the boss, its unbroken parts and the live enemies whose
/// circle reaches within `range` of the main character, the boss or a part first, then an
/// elite, then any other enemy; within a class the nearest, then the lower entity ID. One pass
/// over the enemies, at most the enemy cap (ADR-020).
pub(super) fn assist_target(stage: &Stage, s: &State, range: i32) -> Option<EntityId> {
    let from = s.player.at;
    let mut best: Option<(u8, u64, EntityId)> = None;
    let mut consider = |class: u8, at: Point, radius: i32, id: EntityId| {
        if !overlaps(from, range, at, radius) {
            return;
        }
        let key = (class, dist2(from, at), id);
        if best.is_none_or(|b| key < b) {
            best = Some(key);
        }
    };
    if let Some((def, boss)) = stage.boss.as_ref().zip(s.boss.as_ref()) {
        if boss.hp > 0 {
            consider(0, boss.at, def.radius.0, boss.id);
        }
        for (i, part) in boss.parts.iter().enumerate() {
            if part.hp > 0 {
                consider(0, s.at(Who::BossPart(i)), part.radius, part.id);
            }
        }
    }
    for e in &s.enemies {
        if e.hp > 0 {
            let class = if e.elite.is_some() { 1 } else { 2 };
            consider(class, e.at, agent_def(stage, e.key).radius.0, e.id);
        }
    }
    best.map(|(_, _, id)| id)
}

/// Aim assist's first `n` targets, in its order (`assist_target`'s first): homing missiles'
/// (ADR-099). One pass over the enemies, each set against at most `n` kept. Its own walk, so
/// that aim assist's stays as the tick-cost gate measures it (ADR-058).
pub(super) fn assist_targets(stage: &Stage, s: &State, range: i32, n: usize) -> Vec<EntityId> {
    let from = s.player.at;
    let mut kept: Vec<(u8, u64, EntityId)> = Vec::with_capacity(n.saturating_add(1));
    let mut consider = |class: u8, at: Point, radius: i32, id: EntityId| {
        if !overlaps(from, range, at, radius) {
            return;
        }
        let key = (class, dist2(from, at), id);
        if kept.len() == n && kept.last().is_none_or(|last| key >= *last) {
            return;
        }
        let at = kept.partition_point(|k| *k < key);
        kept.insert(at, key);
        kept.truncate(n);
    };
    if let Some((def, boss)) = stage.boss.as_ref().zip(s.boss.as_ref()) {
        if boss.hp > 0 {
            consider(0, boss.at, def.radius.0, boss.id);
        }
        for (i, part) in boss.parts.iter().enumerate() {
            if part.hp > 0 {
                consider(0, s.at(Who::BossPart(i)), part.radius, part.id);
            }
        }
    }
    for e in &s.enemies {
        if e.hp > 0 {
            let class = if e.elite.is_some() { 1 } else { 2 };
            consider(class, e.at, agent_def(stage, e.key).radius.0, e.id);
        }
    }
    kept.into_iter().map(|(_, _, id)| id).collect()
}
