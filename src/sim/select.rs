//! Selectors (ADR-SIM-4): who a rule, a condition or a skill acts on. Ties break by squared
//! distance from the acting entity, then entity ID, so the choice never depends on storage
//! order.
//!
//! Allies are the main character and the companions, whichever side the acting entity is on:
//! an enemy's `nearest_ally` is the nearest of them. Enemies are the enemies and the boss.

use super::World;
use crate::sim::state::{MAX_PLAYERS, PlayerIdx, Players};
use std::cmp::Reverse;

use crate::behaviour::Selector;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::sim::state::State;

use super::entity::Who;
use super::overlaps;

/// A copy of the places, slots and aggro of the players the world reads (`Players::targets`):
/// positions, hit points and aggro do not change while the agents act, so the agent step takes
/// it once. With one player every use reads that one place, no loop (ADR-SIM-6).
#[derive(Clone, Copy, Debug)]
pub(super) struct Up {
    pub(super) n: usize,
    pub(super) at: [Point; MAX_PLAYERS as usize],
    pub(super) aggro: [u32; MAX_PLAYERS as usize],
    pub(super) slot: [PlayerIdx; MAX_PLAYERS as usize],
}

impl Up {
    pub(super) fn of(players: &Players) -> Up {
        let mut up = Up {
            n: 0,
            at: [Point::default(); MAX_PLAYERS as usize],
            aggro: [0; MAX_PLAYERS as usize],
            slot: [PlayerIdx::SOLO; MAX_PLAYERS as usize],
        };
        for me in Players::slots(players.targets()) {
            let k = up.n;
            (up.at[k], up.aggro[k], up.slot[k]) = (players[me].at, players[me].aggro, me);
            up.n += 1;
        }
        up
    }

    /// Whether a circle of `radius` at `at` reaches a circle of `other` at any of them.
    pub(super) fn within(&self, at: Point, radius: i32, other: i32) -> bool {
        self.at[..self.n]
            .iter()
            .any(|&p| overlaps(at, radius, p, other))
    }

    /// The least squared distance from `from` to one of them.
    #[inline]
    pub(super) fn least_d2(&self, from: Point) -> u64 {
        if self.n == 1 {
            return dist2(from, self.at[0]);
        }
        self.at[..self.n]
            .iter()
            .map(|&p| dist2(from, p))
            .min()
            .unwrap_or(u64::MAX)
    }
}

fn dist2(a: Point, b: Point) -> u64 {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    (dx * dx + dy * dy).unsigned_abs()
}

/// The player an enemy, a companion or the boss at `from` goes for (ADR-SIM-7 point 6).
///
/// Over the players the world reads: the one that is alone; else the most aggro where any holds
/// some, then the nearest, ties to the lowest slot. `random` marks an enemy no aggro drives that
/// may switch target now and then (see `rolled`).
pub(super) fn chase(s: &State, up: &Up, from: Point, key: u32, random: bool) -> PlayerIdx {
    let _ = (s, key, random);
    if up.n == 1 {
        return up.slot[0];
    }
    // Slots run lowest first, so a strict `<` keeps the lowest of equals.
    let mut best = 0;
    let mut best_key = (Reverse(up.aggro[0]), dist2(from, up.at[0]));
    for k in 1..up.n {
        let key = (Reverse(up.aggro[k]), dist2(from, up.at[k]));
        if key < best_key {
            (best, best_key) = (k, key);
        }
    }
    up.slot[best]
}

/// The player `me`, an agent or the boss, goes for: a companion follows the lowest up slot; an
/// enemy and the boss take the chase target (ADR-SIM-7 point 6).
pub(super) fn target_of(s: &State, up: &Up, me: Who) -> PlayerIdx {
    if up.n == 1 || matches!(me, Who::Companion(_)) {
        return up.slot[0];
    }
    chase(s, up, s.at(me), s.roll_key(s.id_of(me)), true)
}

/// The place of `target_of`: one load for a single up player.
#[inline]
pub(super) fn target_at(s: &State, up: &Up, me: Who) -> Point {
    if up.n == 1 {
        return up.at[0];
    }
    s.players[target_of(s, up, me)].at
}

/// The order key of an ally after its distance: a player's slot, any other entity's number, so a
/// rejoined slot (a higher generation) keeps the place of its slot among equals (ADR-SIM-7).
fn ally_tie(s: &State, w: Who) -> u32 {
    match w {
        Who::Player(i) => u32::try_from(i.get()).unwrap_or(u32::MAX),
        other => s.id_of(other).0,
    }
}

/// The live ally `by` ranks lowest (the first of equals), or `None`; `skip` is left out. Visits the
/// main character, the other players, then the companions: a plain loop, since every ally
/// selection of every agent of a tick runs through here (ADR-SIM-6).
fn best_ally<K: Ord>(s: &State, skip: Option<Who>, by: impl Fn(Who) -> K) -> Option<Who> {
    let mut best: Option<(K, Who)> = None;
    let mut visit = |w: Who| {
        if Some(w) == skip || !s.is_alive(w) {
            return;
        }
        let key = by(w);
        if best.as_ref().is_none_or(|(k, _)| key < *k) {
            best = Some((key, w));
        }
    };
    visit(Who::Player(PlayerIdx::SOLO));
    for i in s.players.after_first() {
        visit(Who::Player(i));
    }
    for i in 0..s.companions.len() {
        visit(Who::Companion(i));
    }
    best.map(|(_, w)| w)
}

fn enemies(s: &State) -> impl Iterator<Item = Who> + '_ {
    s.boss
        .iter()
        .map(|_| Who::Boss)
        .chain((0..s.enemies.len()).map(Who::Enemy))
        .filter(|w| s.is_alive(*w) && !matches!(*w, Who::Enemy(i) if s.enemies[i].hidden()))
}

/// The entity `selector` picks for `me`, if there is one.
pub(super) fn select(s: &State, me: Who, selector: &Selector) -> Option<Who> {
    let from = s.at(me);
    let by_distance = |w: &Who| (dist2(from, s.at(*w)), s.id_of(*w));
    match selector {
        Selector::Myself => Some(me),
        Selector::Player => std::iter::once(PlayerIdx::SOLO)
            .chain(s.players.after_first())
            .map(Who::Player)
            .find(|w| s.is_alive(*w)),
        Selector::NearestEnemy => enemies(s).min_by_key(by_distance),
        Selector::BossFirst => match s.boss {
            Some(_) if s.is_alive(Who::Boss) => Some(Who::Boss),
            _ => enemies(s).min_by_key(by_distance),
        },
        // Never itself: its own distance is 0, which would always win.
        Selector::NearestAlly => best_ally(s, Some(me), |w| (dist2(from, s.at(w)), ally_tie(s, w))),
        Selector::LowestHpAlly => {
            best_ally(s, None, |w| (s.hp(w), dist2(from, s.at(w)), ally_tie(s, w)))
        }
        Selector::HighestAggroAlly => best_ally(s, None, |w| {
            (Reverse(s.aggro(w)), dist2(from, s.at(w)), ally_tie(s, w))
        }),
        // A part of the boss that has not broken yet.
        Selector::Part(id) => {
            let index = s.boss.as_ref()?.parts.iter().position(|p| p.name == *id)?;
            let part = s.boss.as_ref()?.parts.get(index)?;
            (part.hp > 0).then_some(Who::BossPart(index))
        }
    }
}

/// Aim assist's target: among the boss, its unbroken parts and the live enemies whose
/// circle reaches within `range` of player `me`, the boss or a part first, then an
/// elite, then any other enemy; within a class the nearest, then the lower entity ID. One pass
/// over the enemies, at most the enemy cap (ADR-SVC-1).
pub(super) fn assist_target(
    stage: &World,
    s: &State,
    me: PlayerIdx,
    range: i32,
) -> Option<EntityId> {
    let from = s.players[me].at;
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
    if let Some((def, boss)) = s.boss_pair(stage) {
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
        if e.hp > 0 && !e.hidden() {
            let class = if e.elite.is_some() { 1 } else { 2 };
            consider(class, e.at, stage.defs.agent(e.def).radius.0, e.id);
        }
    }
    best.map(|(_, _, id)| id)
}

/// Aim assist's first `n` targets, in its order (`assist_target`'s first): homing missiles'
///. One pass over the enemies, each set against at most `n` kept. Its own walk, so
/// that aim assist's stays as the tick-cost gate measures it (ADR-SIM-5).
pub(super) fn assist_targets(
    stage: &World,
    s: &State,
    me: PlayerIdx,
    range: i32,
    n: usize,
) -> Vec<EntityId> {
    let from = s.players[me].at;
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
    if let Some((def, boss)) = s.boss_pair(stage) {
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
        if e.hp > 0 && !e.hidden() {
            let class = if e.elite.is_some() { 1 } else { 2 };
            consider(class, e.at, stage.defs.agent(e.def).radius.0, e.id);
        }
    }
    kept.into_iter().map(|(_, _, id)| id).collect()
}
