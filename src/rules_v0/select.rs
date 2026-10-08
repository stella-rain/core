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
use crate::stage::Stage;

use super::entity::Who;

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
pub(super) fn select(_stage: &Stage, s: &State, me: Who, selector: &Selector) -> Option<Who> {
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
        // Boss parts arrive with boss timelines (core#17).
        Selector::Part(_) => None,
    }
}
