//! Hit boxes: every active phase with a hit box hurts the targets inside it, in the
//! collision step, before orbs.
//!
//! - A hit box follows its attacker, or stays on the point an on-hit rune's attack fired at,
//!   and faces the way the attack faced when it started. Its
//!   offset is forward along that facing and to the attacker's right as seen from above: a
//!   quarter turn clockwise on the screen, so facing down the screen its right is the screen's
//!   left.
//! - A hit box is anchored at its attacker, or at the target's spot (where the target stood when
//!   the attack began); a line is a rectangle from its start along the facing turned by its
//!   angle. Zones (`zones`) are tested the same way, after the hit boxes, each target once.
//! - A circle hits a target whose circle overlaps it; an arc, a target whose circle overlaps
//!   the sector `half_angle` to each side of the facing. Integer only: squared distances in 64
//!   or 128 bits, and the arc's edges and angle from the sine table.
//! - Friendly hit boxes hit the boss, its parts and the enemies (found through the enemy grid),
//!   in entity-ID order; enemy hit boxes hit the present players in slot order, then the companions
//!   once. A hit box
//!   hits every target inside it (cleave), each once per active phase, or again `rehit_ticks`
//!   after its last hit. A homing missile's hits only the first, once, and the
//!   missile is gone once every hit box has landed. A target that cannot be hurt (invulnerable, dead) is not hit and not
//!   counted as hit.
//! - Attackers resolve in entity-ID order, and every hit box active at the start of the step
//!   lands, even if its attacker dies earlier in the step: hits on the same tick trade.

use super::World;
use crate::attack::{Anchor, DamageType, HitBox, Offset, PhaseKind, ZoneShape};
use crate::event::{DomainEvent, HitKind};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::rune::Rune;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Dir, Smash, State};
use crate::trig;

use super::entity::Who;
use super::grid::Grid;
use super::{Blame, Culprit, Shot, damage, enemy_grid, hurt_player, overlaps};

/// What a swing is made by, and so where the targets it has hit are kept.
#[derive(Clone, Copy)]
enum Source {
    /// The attack run at this place in `State::runs`.
    Run(usize),
    /// The zone at this place in `State::zones` (core Z1).
    Zone(usize),
}

/// One hit box or zone that is active this tick.
pub(super) struct Swing {
    source: Source,
    owner: EntityId,
    slot: u8,
    friendly: bool,
    area: Area,
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

/// The ground a hit box covers: a circle at `centre`, narrowed to a sector when `arc` is set.
struct Area {
    centre: Point,
    facing: Dir,
    /// The circle's radius; a line's reach from its start: its length and half its width, which
    /// holds every point of it.
    radius: i32,
    /// `None` for a circle, or an arc of 180 degrees or more to each side, which is one.
    arc: Option<Sector>,
    /// A rectangle instead of a circle or an arc.
    line: Option<Line>,
}

/// A rectangle from `Area::centre` along `dir`.
struct Line {
    dir: Dir,
    length: i32,
    /// Half the width: the reach to each side.
    half_width: i32,
}

impl Line {
    fn new(dir: Dir, length: Fx, width: Fx) -> Line {
        Line {
            dir,
            length: length.0,
            half_width: width.0 / 2,
        }
    }

    /// The area of a line starting at `centre`.
    fn area(self, centre: Point) -> Area {
        Area {
            centre,
            facing: self.dir,
            radius: self.length.saturating_add(self.half_width),
            arc: None,
            line: Some(self),
        }
    }

    /// Whether the circle at `d` (from the line's start) with `radius` touches the rectangle:
    /// the nearest point of it to the circle's centre is within `radius`. Squared and scaled by
    /// `trig::SCALE`, so no root is taken.
    fn touches(&self, d: (i128, i128), radius: i32) -> bool {
        let (ex, ey) = (i128::from(self.dir.x), i128::from(self.dir.y));
        let scale = i128::from(trig::SCALE);
        let along = d.0 * ex + d.1 * ey;
        let across = d.0 * ey - d.1 * ex;
        let reach = i128::from(self.half_width) * scale;
        let past_along = along - along.clamp(0, i128::from(self.length) * scale);
        let past_across = across - across.clamp(-reach, reach);
        let r = i128::from(radius) * scale;
        past_along * past_along + past_across * past_across <= r * r
    }
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
/// friendly hit box needs it and left for the orbs after.
// Kept out of `sim::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-SIM-5).
#[inline(never)]
pub(super) fn resolve(
    stage: &World,
    s: &mut State,
    grid: &mut Option<Grid>,
    events: &mut Vec<DomainEvent>,
) {
    let mut swings = std::mem::take(&mut s.scratch.swings);
    let mut found = std::mem::take(&mut s.scratch.found);
    let mut bits = std::mem::take(&mut s.scratch.found_bits);
    active_swings(stage, s, &mut swings);
    swings.sort_by_key(|w| (w.owner.0, w.slot));
    // The zones land after the hit boxes, in the order they were placed.
    zone_swings(s, &mut swings);
    if !s.orbs.is_empty() {
        for me in s.players.indices() {
            break_orbs(stage, s, me, &swings, events);
        }
    }
    if s.next_orb_group > 0 {
        // A beam's `last_hit` is shared, so a beam hurts at most one player per `rehit_ticks`,
        // the lowest slot first.
        for me in s.players.indices() {
            cage_beams(stage, s, me, events);
        }
    }
    for w in &swings {
        if w.friendly {
            let grid = grid.get_or_insert_with(|| enemy_grid(stage, s));
            hit_enemy_side(stage, s, grid, w, (&mut bits, &mut found), events);
        } else {
            hit_players_side(stage, s, w, events);
            hit_companions_side(stage, s, w, events);
        }
    }
    if swings.iter().any(|w| w.once) {
        super::attacks::retain_runs(s, |r| r.homing.is_none() || r.hits.is_empty());
        s.move_index.rebuild(stage, &s.runs);
    }
    s.scratch.swings = swings;
    s.scratch.found = found;
    s.scratch.found_bits = bits;
}

/// A breakable enemy orb that a hit box of the main character covers is removed, with no burst,
/// and reported once (core Z2, C24). It is gone before the orbs land, so it hurts nobody this
/// tick.
fn break_orbs(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    swings: &[Swing],
    events: &mut Vec<DomainEvent>,
) {
    let owner = s.players.id(me);
    let mine = |w: &&Swing| w.friendly && w.owner == owner;
    s.orbs.retain(|b| {
        if !b.breakable {
            return true;
        }
        let hit = swings
            .iter()
            .filter(mine)
            .any(|w| covers(w, b.at, stage.tuning.orb_radius));
        if hit {
            events.push(DomainEvent::OrbBroken {
                owner: b.owner,
                at: b.at,
            });
        }
        !hit
    });
}

/// The beams of the orbs' cages hurt the main character (Z3a-3), a beam at most once per its
/// `rehit_ticks`; an orb broken just above is gone, and its beams with it. Each beam is a line
/// from one orb to its neighbour, tested like a hit box line; its last hit is kept on the orb it
/// starts from.
fn cage_beams(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    for i in 0..s.orbs.len() {
        let p = &s.players[me];
        if p.hp == 0 || p.invulnerable > 0 {
            return;
        }
        let Some(j) = super::beam_to(&s.orbs, i) else {
            continue;
        };
        let (from, to) = (&s.orbs[i], &s.orbs[j]);
        let from_program = from.program;
        let Some(link) = from.link else {
            continue;
        };
        if link.last_hit.is_some_and(|t| s.tick - t < link.rehit_ticks) {
            continue;
        }
        let dx = i64::from(to.at.x.0) - i64::from(from.at.x.0);
        let dy = i64::from(to.at.y.0) - i64::from(from.at.y.0);
        let length = i32::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i32::MAX);
        let line = Line::new(super::attacks::normalize(dx, dy), Fx(length), link.width);
        let d = (
            i128::from(p.at.x.0) - i128::from(from.at.x.0),
            i128::from(p.at.y.0) - i128::from(from.at.y.0),
        );
        if !line.touches(d, stage.tuning.player_radius) {
            continue;
        }
        let shot = Shot {
            at: p.at,
            owner: from.owner,
            friendly: false,
            damage: link.damage,
            damage_type: from.damage_type,
            breaks_charge: from.breaks_charge,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        };
        if let Some(l) = s.orbs[i].link.as_mut() {
            l.last_hit = Some(s.tick);
        }
        let by = Blame::of_program(HitKind::Link, from_program);
        hurt_player(stage, s, me, super::aided(stage, s, shot), by, events);
    }
}

fn active_swings(stage: &World, s: &State, swings: &mut Vec<Swing>) {
    swings.clear();
    for (i, r) in s.runs.iter().enumerate() {
        let p = super::attacks::phase(stage, r, r.phase as usize);
        let (PhaseKind::Active, Some(h)) = (p.kind, p.hit_box()) else {
            continue;
        };
        let Some(owner) = s.find(r.owner) else {
            continue;
        };
        let from = match (h.anchor(), r.spot) {
            (Anchor::Target, Some(spot)) => spot,
            _ => r.at.unwrap_or_else(|| s.at(owner)),
        };
        let at_offset = |offset: &Offset| centre(from, r.facing, offset.forward.0, offset.right.0);
        let area = match h {
            HitBox::Circle { offset, radius, .. } => {
                circle(at_offset(offset), r.facing, radius.0, None)
            }
            HitBox::Arc {
                offset,
                radius,
                half_angle,
                ..
            } => circle(
                at_offset(offset),
                r.facing,
                radius.0,
                Sector::new(r.facing, *half_angle),
            ),
            HitBox::Line {
                offset,
                angle,
                length,
                width,
                ..
            } => Line::new(super::attacks::rotate(r.facing, *angle), *length, *width)
                .area(at_offset(offset)),
        };
        swings.push(Swing {
            source: Source::Run(i),
            owner: r.owner,
            slot: r.slot,
            friendly: r.friendly,
            area,
            damage: super::attacks::phase_damage(stage, r),
            damage_type: p.damage_type,
            rehit_ticks: p.rehit_ticks,
            breaks_charge: p.breaks_charge,
            smash: r.smash,
            combo: super::attacks::is_combo(s, r),
            rune: super::runes::of_run(r),
            once: r.homing.is_some(),
        });
    }
}

/// Adds a swing for each zone that is active this tick.
fn zone_swings(s: &State, swings: &mut Vec<Swing>) {
    for (i, z) in s.zones.iter().enumerate() {
        if z.wind_left > 0 || z.warn_only {
            continue;
        }
        let area = match &z.shape {
            ZoneShape::Circle { radius } => circle(z.at, z.dir, radius.0, None),
            ZoneShape::Arc { radius, half_angle } => {
                circle(z.at, z.dir, radius.0, Sector::new(z.dir, *half_angle))
            }
            ZoneShape::Line { length, width } => Line::new(z.dir, *length, *width).area(z.at),
        };
        swings.push(Swing {
            source: Source::Zone(i),
            owner: z.owner,
            slot: 0,
            friendly: z.friendly,
            area,
            damage: z.damage,
            damage_type: z.damage_type,
            rehit_ticks: None,
            breaks_charge: z.breaks_charge,
            smash: None,
            combo: false,
            rune: None,
            once: false,
        });
    }
}

fn circle(centre: Point, facing: Dir, radius: i32, arc: Option<Sector>) -> Area {
    Area {
        centre,
        facing,
        radius,
        arc,
        line: None,
    }
}

/// The hit box's centre: `forward` along `facing` and `right` a quarter turn clockwise from it.
pub(super) fn centre(at: Point, facing: Dir, forward: i32, right: i32) -> Point {
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
    w.area.covers(at, radius)
}

impl Area {
    fn covers(&self, at: Point, radius: i32) -> bool {
        if !overlaps(self.centre, self.radius, at, radius) {
            return false;
        }
        if self.arc.is_none() && self.line.is_none() {
            return true;
        }
        let d = (
            i128::from(at.x.0) - i128::from(self.centre.x.0),
            i128::from(at.y.0) - i128::from(self.centre.y.0),
        );
        let Some(arc) = &self.arc else {
            return self
                .line
                .as_ref()
                .is_some_and(|line| line.touches(d, radius));
        };
        // A centre within the angle and within reach overlaps; one outside the angle overlaps only
        // if its circle crosses one of the two edges, the nearest part of the sector to it.
        within_angle(d, self.facing, arc.cos)
            || arc
                .edges
                .iter()
                .any(|&edge| touches_edge(d, edge, self.radius, radius))
    }
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

/// The targets the swing has hit, by ID.
fn hits_of<'a>(s: &'a State, w: &Swing) -> &'a [(EntityId, u32)] {
    match w.source {
        Source::Run(i) => &s.runs[i].hits,
        Source::Zone(i) => &s.zones[i].hits,
    }
}

/// Whether the swing may hit `target` now, and if so, counts the hit.
fn take_hit(s: &mut State, w: &Swing, target: EntityId) -> bool {
    let tick = s.tick;
    let hits = match w.source {
        Source::Run(i) => &mut s.runs[i].hits,
        Source::Zone(i) => &mut s.zones[i].hits,
    };
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
            // The first hit of a run or a zone takes a list that has a room already, if one is
            // waiting: only the runs and zones that hit hold one, so the pool settles at the
            // most that hit at once and a steady tick allocates nothing (T1b).
            if hits.capacity() == 0 {
                *hits = s.scratch.take_hit_list();
            }
            hits.insert(i, (target, tick));
            true
        }
    }
}

fn shot(w: &Swing) -> Shot {
    Shot {
        at: w.area.centre,
        owner: w.owner,
        friendly: w.friendly,
        damage: w.damage,
        damage_type: w.damage_type,
        breaks_charge: w.breaks_charge,
        smash: w.smash,
        combo: w.combo,
        rune: w.rune,
        chain: false,
    }
}

/// The boss, its parts and the enemies, in entity-ID order. An invulnerable boss shields its
/// parts, as it does from orbs.
fn hit_enemy_side(
    stage: &World,
    s: &mut State,
    grid: &Grid,
    w: &Swing,
    (bits, found): (&mut Vec<u64>, &mut Vec<usize>),
    events: &mut Vec<DomainEvent>,
) {
    // The player who made the swing; a companion's swing is the run's first player's (the stage's
    // companions belong to no player).
    let me = super::hitter(s, w.owner);
    if let Some((def, boss)) = s.boss_pair(stage) {
        if boss.invulnerable == 0 {
            let (id, at, radius) = (boss.id, boss.at, def.radius.0);
            if boss.hp > 0 && covers(w, at, radius) && take_hit(s, w, id) {
                damage(stage, s, me, shot(w), Who::Boss, events);
            }
            let parts = s.boss.as_ref().map_or(0, |b| b.parts.len());
            for i in 0..parts {
                let Some(part) = s.boss.as_ref().map(|b| &b.parts[i]) else {
                    break;
                };
                let (id, hp, radius) = (part.id, part.hp, part.radius);
                if hp > 0 && covers(w, s.at(Who::BossPart(i)), radius) && take_hit(s, w, id) {
                    damage(stage, s, me, shot(w), Who::BossPart(i), events);
                }
            }
        }
    }
    grid.within(w.area.centre, w.area.radius, bits, found);
    // A swing that never hits a target again (a zone, or a hit box with no `rehit_ticks`) passes
    // over the ones it has hit with nothing else to find out, so those are told from its hit
    // list, sorted by ID like `found`, with a cursor that only moves forward. A cursor that has
    // gone past a target only sends it down the full path, which decides alone.
    let hit_once = w.rehit_ticks.is_none() && !w.once;
    let mut seen = 0;
    for &i in found.iter() {
        let e = &s.enemies[i];
        let (id, at, hp) = (e.id, e.at, e.hp);
        if hit_once {
            let hits = hits_of(s, w);
            while seen < hits.len() && hits[seen].0.0 < id.0 {
                seen += 1;
            }
            if hits.get(seen).is_some_and(|h| h.0 == id) {
                continue;
            }
        }
        let radius = stage.defs.agent(e.def).radius.0;
        // A shrine is not hurt, nor a mimic in disguise: the hit passes through them.
        if hp > 0
            && e.shrine.is_none()
            && !e.hidden()
            && covers(w, at, radius)
            && take_hit(s, w, id)
        {
            damage(stage, s, me, shot(w), Who::Enemy(i), events);
            match w.smash {
                Some(Smash::Pull(pull)) => super::pull_enemy(stage, s, me, i, pull),
                Some(Smash::Knockback(push)) => super::knock_enemy(stage, s, me, i, push),
                _ => {}
            }
        }
    }
}

/// Every player in the party that the swing covers, in slot order, unless it is invulnerable:
/// each is hurt once.
fn hit_players_side(stage: &World, s: &mut State, w: &Swing, events: &mut Vec<DomainEvent>) {
    for me in s.players.indices() {
        let id = s.players.id(me);
        let p = &s.players[me];
        if p.hp > 0
            && p.invulnerable == 0
            && covers(w, p.at, stage.tuning.player_radius)
            && take_hit(s, w, id)
        {
            let by = match w.source {
                Source::Run(i) => Blame {
                    kind: HitKind::Attack,
                    by: Culprit::Run(s.runs[i].attack),
                },
                Source::Zone(i) => Blame::of_program(HitKind::Zone, s.zones[i].program),
            };
            hurt_player(stage, s, me, super::aided(stage, s, shot(w)), by, events);
        }
    }
}

/// The companions in ID order, each hurt once however many players there are.
fn hit_companions_side(stage: &World, s: &mut State, w: &Swing, events: &mut Vec<DomainEvent>) {
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let (id, at, hp) = (c.id, c.at, c.hp);
        let radius = stage.defs.agent(c.def).radius.0;
        if hp > 0 && covers(w, at, radius) && take_hit(s, w, id) {
            // An enemy's hit on a companion reads no player (it neither crits nor echoes).
            damage(
                stage,
                s,
                PlayerIdx::SOLO,
                super::aided(stage, s, shot(w)),
                Who::Companion(i),
                events,
            );
        }
    }
}

#[cfg(test)]
mod party_tests {
    //! Hits on each player of a party (ADR-SIM-7): an enemy's swing or orb reaches every player
    //! it covers, once each and in slot order, and the companions once.

    use super::*;
    use crate::bench_fixtures::worst_case;
    use crate::engine::Run;
    use crate::event::DomainEvent;

    fn slot(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).expect("a slot")
    }

    fn party() -> (World, State) {
        let e = worst_case::warmed_up().into_party(3);
        let Run::V2 { world, state } = e.run() else {
            panic!("the live rules");
        };
        let mut s = state.clone();
        assert!(!s.runs.is_empty(), "a run for the swing to belong to");
        s.runs[0].hits.clear();
        for i in 0..3 {
            let p = &mut s.players[slot(i)];
            p.hp = p.max_hp;
            p.invulnerable = 0;
            p.at.x.0 = i32::from(i) * 4_000_000;
            p.at.y.0 = 0;
        }
        s.companions.clear();
        (world.clone(), s)
    }

    fn swing(at: Point, radius: i32, friendly: bool, owner: EntityId) -> Swing {
        Swing {
            source: Source::Run(0),
            owner,
            slot: 0,
            friendly,
            area: Area {
                centre: at,
                facing: Dir::UP,
                radius,
                arc: None,
                line: None,
            },
            damage: 10,
            damage_type: None,
            rehit_ticks: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            once: false,
        }
    }

    fn hp(s: &State) -> [u32; 3] {
        [0, 1, 2].map(|i| s.players[slot(i)].hp)
    }

    fn hurt_in(ev: &[DomainEvent]) -> usize {
        ev.iter()
            .filter(|e| matches!(e, DomainEvent::PlayerHit { .. }))
            .count()
    }

    #[test]
    fn a_swing_hurts_the_player_it_covers_and_no_other() {
        let (w, mut s) = party();
        let before = hp(&s);
        let mut ev = Vec::new();
        let sw = swing(
            s.players[slot(2)].at,
            w.tuning.player_radius,
            false,
            EntityId(900),
        );
        hit_players_side(&w, &mut s, &sw, &mut ev);
        let after = hp(&s);
        assert_eq!((after[0], after[1]), (before[0], before[1]));
        assert!(after[2] < before[2]);
        assert_eq!(hurt_in(&ev), 1);
    }

    #[test]
    fn a_swing_over_all_three_hurts_each_once_in_slot_order() {
        let (w, mut s) = party();
        for i in 1..3 {
            s.players[slot(i)].at = s.players[PlayerIdx::SOLO].at;
        }
        let before = hp(&s);
        let mut ev = Vec::new();
        let at = s.players[PlayerIdx::SOLO].at;
        let sw = swing(at, w.tuning.player_radius, false, EntityId(900));
        hit_players_side(&w, &mut s, &sw, &mut ev);
        assert!(hp(&s).iter().zip(before).all(|(a, b)| *a < b));
        assert_eq!(hurt_in(&ev), 3);
        let order: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                DomainEvent::PlayerHit { hp_left, .. } => Some(*hp_left),
                _ => None,
            })
            .collect();
        assert_eq!(order, hp(&s).to_vec(), "in slot order");
    }

    #[test]
    fn an_invulnerable_player_is_skipped() {
        let (w, mut s) = party();
        for i in 1..3 {
            s.players[slot(i)].at = s.players[PlayerIdx::SOLO].at;
        }
        s.players[slot(1)].invulnerable = 30;
        let before = hp(&s);
        let mut ev = Vec::new();
        let at = s.players[PlayerIdx::SOLO].at;
        let sw = swing(at, w.tuning.player_radius, false, EntityId(900));
        hit_players_side(&w, &mut s, &sw, &mut ev);
        let after = hp(&s);
        assert!(after[0] < before[0] && after[2] < before[2]);
        assert_eq!(after[1], before[1]);
    }

    #[test]
    fn an_orb_is_used_up_by_the_first_player_it_overlaps() {
        let (w, mut s) = party();
        s.players[slot(1)].at = s.players[PlayerIdx::SOLO].at;
        let before = hp(&s);
        let shot = Shot {
            at: s.players[PlayerIdx::SOLO].at,
            owner: EntityId(900),
            friendly: false,
            damage: 10,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        };
        let by = Blame::of_program(HitKind::Attack, crate::sim::world::ProgramIndex(0));
        let mut ev = Vec::new();
        assert!(super::super::hit_players_side(
            &w, &mut s, shot, by, &mut ev
        ));
        let after = hp(&s);
        assert!(after[0] < before[0]);
        assert_eq!((after[1], after[2]), (before[1], before[2]));
        assert_eq!(hurt_in(&ev), 1);
    }

    fn orb(
        at: Point,
        owner: EntityId,
        link: Option<crate::sim::state::OrbLink>,
    ) -> crate::sim::state::OrbState {
        crate::sim::state::OrbState {
            at,
            vx: Fx(0),
            vy: Fx(0),
            dir: Dir::UP,
            style: 0,
            owner,
            damage: 1,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            hook: None,
            breakable: true,
            life: 100,
            stop: None,
            link,
            program: crate::sim::world::ProgramIndex(0),
        }
    }

    #[test]
    fn a_players_swing_breaks_an_orb_and_a_companions_does_not() {
        let (w, mut s) = party();
        for (owner, broken) in [(slot(1).id(), true), (EntityId(900), false)] {
            s.orbs = vec![orb(Point::default(), EntityId(900), None)];
            let sw = swing(Point::default(), 1_000_000, true, owner);
            let mut ev = Vec::new();
            for me in s.players.indices() {
                break_orbs(&w, &mut s, me, std::slice::from_ref(&sw), &mut ev);
            }
            assert_eq!(s.orbs.is_empty(), broken, "owner {owner:?}");
            assert_eq!(
                ev.iter()
                    .filter(|e| matches!(e, DomainEvent::OrbBroken { .. }))
                    .count(),
                usize::from(broken)
            );
        }
    }

    #[test]
    fn a_beam_hurts_one_player_per_rehit_by_the_shared_last_hit() {
        let (w, mut s) = party();
        let link = |slot| crate::sim::state::OrbLink {
            group: 1,
            slot,
            of: 2,
            width: Fx::px(40),
            damage: 10,
            rehit_ticks: 30,
            last_hit: None,
        };
        let (a, b) = (
            Point::default(),
            Point {
                x: Fx::px(200),
                y: Fx(0),
            },
        );
        s.orbs = vec![
            orb(a, EntityId(900), Some(link(0))),
            orb(b, EntityId(900), Some(link(1))),
        ];
        s.tick = 100;
        let mid = Point {
            x: Fx::px(100),
            y: Fx(0),
        };
        for i in 0..2 {
            s.players[slot(i)].at = mid;
        }
        let before = hp(&s);
        let mut ev = Vec::new();
        for me in s.players.indices() {
            cage_beams(&w, &mut s, me, &mut ev);
        }
        let after = hp(&s);
        assert!(after[0] < before[0], "the lowest slot is hit");
        assert_eq!(
            after[1], before[1],
            "the shared last hit spares slot 1 this tick"
        );
        assert_eq!(hurt_in(&ev), 1);
    }
}
